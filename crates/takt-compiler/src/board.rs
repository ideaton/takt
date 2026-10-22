//! Cartes supportées et affectation des voies d'E/S aux broches.
//!
//! Les adresses suivent la convention Siemens : `%I0.0`…`%I0.7` puis
//! `%I1.0`… pour les entrées TOR, `%IW0`, `%IW2`… pour les entrées
//! analogiques (adresses d'octet, pas de 2).

use takt_vm::image::IoKind;

#[derive(Debug)]
pub struct Board {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// GPIO des entrées TOR, dans l'ordre des voies.
    pub di: &'static [u8],
    pub dq: &'static [u8],
    /// GPIO des entrées analogiques (26..29 = ADC0..3).
    pub ai: &'static [u8],
    /// GPIO des sorties analogiques (PWM).
    pub aq: &'static [u8],
}

pub const BOARDS: &[Board] = &[
    Board {
        id: "pico2",
        name: "Raspberry Pi Pico 2",
        description: "RP2350A, 2 × Cortex-M33 150 MHz, 520 Ko SRAM",
        di: &[2, 3, 4, 5, 6, 7, 8, 9],
        dq: &[10, 11, 12, 13, 14, 15, 16, 17],
        ai: &[26, 27, 28],
        aq: &[18, 19],
    },
    Board {
        id: "rp2350-zero",
        name: "Waveshare RP2350-Zero",
        description: "RP2350A format mini, 2 × Cortex-M33 150 MHz",
        di: &[0, 1, 2, 3],
        dq: &[4, 5, 6, 7],
        ai: &[26, 27, 28],
        aq: &[8],
    },
];

pub fn find(id: &str) -> Option<&'static Board> {
    BOARDS.iter().find(|b| b.id == id)
}

/// Adresse décodée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Io {
        kind: IoKind,
        channel: u16,
    },
    /// Mémento (%M) : simple variable mémoire, pas de recouvrement d'octets.
    Marker,
}

/// Décode une adresse et indique le type attendu (`BOOL`, `INT`, ou mot double).
pub fn parse_address(text: &str) -> Result<(Address, AddrSize), String> {
    let t = text.trim().to_ascii_uppercase();
    let body = t.strip_prefix('%').ok_or_else(|| format!("l'adresse « {text} » doit commencer par %"))?;
    let (area, rest) = body.split_at(1.min(body.len()));
    let (size, rest) = match rest.chars().next() {
        Some('W') => (AddrSize::Word, &rest[1..]),
        Some('D') => (AddrSize::DWord, &rest[1..]),
        Some('X') => (AddrSize::Bit, &rest[1..]),
        _ => (AddrSize::Bit, rest),
    };
    let channel = match size {
        AddrSize::Bit => {
            let (byte, bit) = rest.split_once('.').ok_or_else(|| format!("« {text} » : forme %I0.0 attendue"))?;
            let byte: u16 = byte.parse().map_err(|_| format!("« {text} » : octet invalide"))?;
            let bit: u16 = bit.parse().map_err(|_| format!("« {text} » : bit invalide"))?;
            if bit > 7 {
                return Err(format!("« {text} » : le bit va de 0 à 7"));
            }
            byte * 8 + bit
        }
        _ => {
            let byte: u16 = rest.parse().map_err(|_| format!("« {text} » : adresse invalide"))?;
            if !byte.is_multiple_of(2) {
                return Err(format!("« {text} » : les mots sont à des adresses paires"));
            }
            byte / 2
        }
    };
    let kind = match (area, size) {
        ("M", _) => return Ok((Address::Marker, size)),
        ("I", AddrSize::Bit) => IoKind::Di,
        ("Q", AddrSize::Bit) => IoKind::Dq,
        ("I", AddrSize::Word) => IoKind::Ai,
        ("Q", AddrSize::Word) => IoKind::Aq,
        ("I" | "Q", AddrSize::DWord) => {
            return Err(format!("« {text} » : pas d'E/S sur double mot"));
        }
        _ => return Err(format!("« {text} » : zone inconnue (I, Q ou M)")),
    };
    Ok((Address::Io { kind, channel }, size))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrSize {
    Bit,
    Word,
    DWord,
}

impl Board {
    pub fn pins(&self, kind: IoKind) -> &'static [u8] {
        match kind {
            IoKind::Di => self.di,
            IoKind::Dq => self.dq,
            IoKind::Ai => self.ai,
            IoKind::Aq => self.aq,
        }
    }

    pub fn pin(&self, kind: IoKind, channel: u16) -> Option<u8> {
        self.pins(kind).get(channel as usize).copied()
    }

    /// Adresse Siemens de la voie `channel`.
    pub fn address_of(kind: IoKind, channel: u16) -> String {
        match kind {
            IoKind::Di => format!("%I{}.{}", channel / 8, channel % 8),
            IoKind::Dq => format!("%Q{}.{}", channel / 8, channel % 8),
            IoKind::Ai => format!("%IW{}", channel * 2),
            IoKind::Aq => format!("%QW{}", channel * 2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_address("%I1.2").unwrap().0, Address::Io { kind: IoKind::Di, channel: 10 });
        assert_eq!(parse_address("%qw4").unwrap().0, Address::Io { kind: IoKind::Aq, channel: 2 });
        assert_eq!(parse_address("%MD8").unwrap(), (Address::Marker, AddrSize::DWord));
        assert!(parse_address("%I0.8").is_err());
        assert!(parse_address("I0.0").is_err());
    }
}
