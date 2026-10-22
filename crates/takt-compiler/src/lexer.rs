//! Analyse lexicale du Structured Text.
//!
//! Insensible à la casse pour les mots-clés. Accepte la syntaxe SCL de
//! Siemens : `#variableLocale` et `"Variable API"`.

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Identifiant ; `local` vaut vrai s'il était préfixé par `#`.
    Ident {
        name: String,
        local: bool,
    },
    /// Variable API entre guillemets.
    Quoted(String),
    Int(i64),
    Real(f64),
    /// Durée en ms (`T#1s500ms`, `TIME#2m`).
    Time(i64),
    Assign,    // :=
    OutAssign, // =>
    Semi,
    Colon,
    Comma,
    Dot,
    DotDot,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Amp,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

#[derive(Debug)]
pub struct LexError {
    pub pos: Pos,
    pub msg: String,
}

impl Tok {
    /// Vrai si le jeton est le mot-clé `kw` (en majuscules).
    pub fn is_kw(&self, kw: &str) -> bool {
        matches!(self, Tok::Ident { name, local: false } if name.eq_ignore_ascii_case(kw))
    }
}

pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;

    macro_rules! adv {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, col };
        let peek = chars.get(i + 1).copied();

        if c.is_whitespace() {
            adv!();
            continue;
        }
        // Commentaires : // …, (* … *), /* … */
        if c == '/' && peek == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                adv!();
            }
            continue;
        }
        if (c == '(' && peek == Some('*')) || (c == '/' && peek == Some('*')) {
            let close = if c == '(' { ')' } else { '/' };
            adv!();
            adv!();
            loop {
                if i + 1 >= chars.len() {
                    return Err(LexError { pos, msg: "commentaire non fermé".into() });
                }
                if chars[i] == '*' && chars[i + 1] == close {
                    adv!();
                    adv!();
                    break;
                }
                adv!();
            }
            continue;
        }
        // Attributs Siemens `{ S7_Optimized_Access := 'TRUE' }` : ignorés.
        if c == '{' {
            while i < chars.len() && chars[i] != '}' {
                adv!();
            }
            if i >= chars.len() {
                return Err(LexError { pos, msg: "accolade non fermée".into() });
            }
            adv!();
            continue;
        }
        if c == '"' {
            adv!();
            let start = i;
            while i < chars.len() && chars[i] != '"' && chars[i] != '\n' {
                adv!();
            }
            if i >= chars.len() || chars[i] != '"' {
                return Err(LexError { pos, msg: "guillemet non fermé".into() });
            }
            let name: String = chars[start..i].iter().collect();
            adv!();
            out.push(Token { tok: Tok::Quoted(name), pos });
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' || c == '#' {
            let local = c == '#';
            if local {
                adv!();
            }
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                adv!();
            }
            let name: String = chars[start..i].iter().collect();
            if name.is_empty() {
                return Err(LexError { pos, msg: "nom attendu après #".into() });
            }
            // Littéral durée : T#… ou TIME#…
            if !local
                && i < chars.len()
                && chars[i] == '#'
                && (name.eq_ignore_ascii_case("T") || name.eq_ignore_ascii_case("TIME"))
            {
                adv!();
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                    adv!();
                }
                let text: String = chars[start..i].iter().collect();
                let ms = parse_duration(&text)
                    .ok_or_else(|| LexError { pos, msg: format!("durée invalide : T#{text}") })?;
                out.push(Token { tok: Tok::Time(ms), pos });
                continue;
            }
            out.push(Token { tok: Tok::Ident { name, local }, pos });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                adv!();
            }
            // Base : 16#FF, 2#1010, 8#17
            if i < chars.len() && chars[i] == '#' {
                let base: String = chars[start..i].iter().filter(|c| **c != '_').collect();
                let radix: u32 = base.parse().unwrap_or(0);
                if ![2, 8, 16].contains(&radix) {
                    return Err(LexError { pos, msg: format!("base {base} non supportée") });
                }
                adv!();
                let s = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    adv!();
                }
                let digits: String = chars[s..i].iter().filter(|c| **c != '_').collect();
                let v = i64::from_str_radix(&digits, radix)
                    .map_err(|_| LexError { pos, msg: format!("nombre invalide : {base}#{digits}") })?;
                out.push(Token { tok: Tok::Int(v), pos });
                continue;
            }
            let mut is_real = false;
            if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                is_real = true;
                adv!();
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    adv!();
                }
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let save = (i, line, col);
                adv!();
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    adv!();
                }
                if i < chars.len() && chars[i].is_ascii_digit() {
                    is_real = true;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        adv!();
                    }
                } else {
                    (i, line, col) = save;
                }
            }
            let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            let tok = if is_real {
                Tok::Real(text.parse().map_err(|_| LexError { pos, msg: format!("réel invalide : {text}") })?)
            } else {
                Tok::Int(text.parse().map_err(|_| LexError { pos, msg: format!("entier trop grand : {text}") })?)
            };
            out.push(Token { tok, pos });
            continue;
        }
        let two: Option<Tok> = match (c, peek) {
            (':', Some('=')) => Some(Tok::Assign),
            ('=', Some('>')) => Some(Tok::OutAssign),
            ('<', Some('>')) => Some(Tok::Ne),
            ('<', Some('=')) => Some(Tok::Le),
            ('>', Some('=')) => Some(Tok::Ge),
            ('.', Some('.')) => Some(Tok::DotDot),
            _ => None,
        };
        if let Some(tok) = two {
            adv!();
            adv!();
            out.push(Token { tok, pos });
            continue;
        }
        let one = match c {
            ';' => Tok::Semi,
            ':' => Tok::Colon,
            ',' => Tok::Comma,
            '.' => Tok::Dot,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            '[' => Tok::LBracket,
            ']' => Tok::RBracket,
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '=' => Tok::Eq,
            '<' => Tok::Lt,
            '>' => Tok::Gt,
            '&' => Tok::Amp,
            _ => {
                return Err(LexError { pos, msg: format!("caractère inattendu « {c} »") });
            }
        };
        adv!();
        out.push(Token { tok: one, pos });
    }
    out.push(Token { tok: Tok::Eof, pos: Pos { line, col } });
    Ok(out)
}

/// `1s500ms`, `2m`, `1h30m`, `250ms`, `1.5s` → millisecondes.
fn parse_duration(text: &str) -> Option<i64> {
    let t = text.replace('_', "").to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    let mut total = 0f64;
    let mut rest = t.as_str();
    while !rest.is_empty() {
        let n = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        if n == 0 {
            return None;
        }
        let value: f64 = rest[..n].parse().ok()?;
        rest = &rest[n..];
        let unit_len = rest.find(|c: char| c.is_ascii_digit()).unwrap_or(rest.len());
        let factor = match &rest[..unit_len] {
            "d" => 86_400_000.0,
            "h" => 3_600_000.0,
            "m" => 60_000.0,
            "s" => 1000.0,
            "ms" => 1.0,
            _ => return None,
        };
        total += value * factor;
        rest = &rest[unit_len..];
    }
    Some(total.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("1s500ms"), Some(1500));
        assert_eq!(parse_duration("2m"), Some(120_000));
        assert_eq!(parse_duration("1.5s"), Some(1500));
        assert_eq!(parse_duration("1h30m"), Some(5_400_000));
        assert_eq!(parse_duration("abc"), None);
    }

    #[test]
    fn siemens_syntax() {
        let t = lex("#x := \"Moteur 1\" + 16#FF; // fin").unwrap();
        assert_eq!(t[0].tok, Tok::Ident { name: "x".into(), local: true });
        assert_eq!(t[2].tok, Tok::Quoted("Moteur 1".into()));
        assert_eq!(t[4].tok, Tok::Int(255));
        assert_eq!(t.len(), 7);
    }
}
