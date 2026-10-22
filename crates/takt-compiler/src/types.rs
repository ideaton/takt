//! Types de données et conversion des valeurs pour l'affichage.

use takt_vm::FbKind;

#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    Bool,
    /// Entier 16 bits.
    Int,
    /// Entier 32 bits.
    DInt,
    /// Flottant 32 bits.
    Real,
    /// Durée en ms sur 32 bits.
    Time,
    Array {
        elem: Box<Ty>,
        lo: i32,
        len: u16,
    },
    Fb(FbType),
}

#[derive(Clone, Debug, PartialEq)]
pub enum FbType {
    Std(FbKind),
    /// Clé en majuscules du FUNCTION_BLOCK utilisateur.
    User(String),
}

impl Ty {
    pub fn elementary(name: &str) -> Option<Ty> {
        Some(match name.to_ascii_uppercase().as_str() {
            "BOOL" => Ty::Bool,
            "INT" => Ty::Int,
            "DINT" => Ty::DInt,
            "REAL" => Ty::Real,
            "TIME" => Ty::Time,
            _ => return None,
        })
    }

    pub fn name(&self) -> String {
        match self {
            Ty::Bool => "BOOL".into(),
            Ty::Int => "INT".into(),
            Ty::DInt => "DINT".into(),
            Ty::Real => "REAL".into(),
            Ty::Time => "TIME".into(),
            Ty::Array { elem, lo, len } => {
                format!("ARRAY[{lo}..{}] OF {}", lo + *len as i32 - 1, elem.name())
            }
            Ty::Fb(FbType::Std(k)) => k.name().into(),
            Ty::Fb(FbType::User(n)) => n.clone(),
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Ty::Int | Ty::DInt)
    }

    pub fn is_num(&self) -> bool {
        matches!(self, Ty::Int | Ty::DInt | Ty::Real)
    }

    pub fn is_scalar(&self) -> bool {
        matches!(self, Ty::Bool | Ty::Int | Ty::DInt | Ty::Real | Ty::Time)
    }
}

/// Affiche une valeur brute de la VM selon son type, façon TIA.
pub fn format_value(bits: u32, ty: &Ty) -> String {
    match ty {
        Ty::Bool => if bits != 0 { "TRUE" } else { "FALSE" }.into(),
        Ty::Int | Ty::DInt => (bits as i32).to_string(),
        Ty::Real => {
            let f = f32::from_bits(bits);
            if f.fract() == 0.0 && f.abs() < 1e7 { format!("{f:.1}") } else { format!("{f}") }
        }
        Ty::Time => format_time(bits as i32 as i64),
        _ => format!("16#{bits:08X}"),
    }
}

pub fn format_time(ms: i64) -> String {
    if ms == 0 {
        return "T#0ms".into();
    }
    let mut s = String::from("T#");
    let mut rest = ms;
    if rest < 0 {
        s.push('-');
        rest = -rest;
    }
    for (unit, size) in [("d", 86_400_000), ("h", 3_600_000), ("m", 60_000), ("s", 1000), ("ms", 1)] {
        if rest >= size {
            s.push_str(&format!("{}{unit}", rest / size));
            rest %= size;
        }
    }
    s
}

/// Lit une valeur saisie dans une table de visualisation.
pub fn parse_value(text: &str, ty: &Ty) -> Option<u32> {
    let t = text.trim();
    match ty {
        Ty::Bool => match t.to_ascii_uppercase().as_str() {
            "TRUE" | "1" => Some(1),
            "FALSE" | "0" => Some(0),
            _ => None,
        },
        Ty::Int => t.parse::<i16>().ok().map(|v| v as i32 as u32),
        Ty::DInt => t.parse::<i32>().ok().map(|v| v as u32),
        Ty::Real => t.replace(',', ".").parse::<f32>().ok().map(f32::to_bits),
        Ty::Time => {
            let toks = crate::lexer::lex(t).ok()?;
            match toks.first().map(|t| &t.tok) {
                Some(crate::lexer::Tok::Time(ms)) => Some(*ms as i32 as u32),
                Some(crate::lexer::Tok::Int(ms)) => Some(*ms as i32 as u32),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_roundtrip() {
        assert_eq!(format_time(1500), "T#1s500ms");
        assert_eq!(parse_value("T#1s500ms", &Ty::Time), Some(1500));
        assert_eq!(parse_value("-12", &Ty::Int), Some((-12i32) as u32));
        assert_eq!(format_value(parse_value("2,5", &Ty::Real).unwrap(), &Ty::Real), "2.5");
        assert_eq!(parse_value("40000", &Ty::Int), None);
    }
}
