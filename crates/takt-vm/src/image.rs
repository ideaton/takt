//! Format binaire du programme téléchargé dans l'automate.
//!
//! ```text
//! en-tête (32 octets)
//!   0  "TAKT"          4  version u16       6  réservé u16
//!   8  cycle_us u32    12 mem_slots u16     14 max_stack u16
//!   16 wcet_cycles u32 20 n_io u16          22 n_init u16
//!   24 n_code u32      28 réservé u32
//! n_io   × 4 octets : slot u16, type u8, broche u8
//! n_init × 8 octets : slot u16, réservé u16, valeur u32
//! n_code × 8 octets : instructions
//! crc32 u32 de tout ce qui précède
//! ```
//! Tous les entiers sont en little endian.

use crate::exec::STACK_MAX;
use crate::instr::Instr;

pub const MAGIC: [u8; 4] = *b"TAKT";
pub const VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 32;
pub const IO_SIZE: usize = 4;
pub const INIT_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub cycle_us: u32,
    pub mem_slots: u16,
    pub max_stack: u16,
    /// WCET du code utilisateur seul (sans la partie fixe du cycle).
    pub wcet_cycles: u32,
    pub n_io: u16,
    pub n_init: u16,
    pub n_code: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IoKind {
    /// Entrée TOR (BOOL).
    Di = 0,
    /// Sortie TOR (BOOL).
    Dq = 1,
    /// Entrée analogique (INT, 0..27648 comme chez Siemens).
    Ai = 2,
    /// Sortie analogique en PWM (INT, 0..27648).
    Aq = 3,
}

impl IoKind {
    pub fn from_u8(v: u8) -> Option<IoKind> {
        Some(match v {
            0 => IoKind::Di,
            1 => IoKind::Dq,
            2 => IoKind::Ai,
            3 => IoKind::Aq,
            _ => return None,
        })
    }

    pub fn is_input(self) -> bool {
        matches!(self, IoKind::Di | IoKind::Ai)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoEntry {
    pub slot: u16,
    pub kind: IoKind,
    /// Numéro de GPIO (ou de voie ADC pour `Ai`).
    pub pin: u8,
}

impl IoEntry {
    pub const EMPTY: IoEntry = IoEntry { slot: 0, kind: IoKind::Di, pin: 0 };

    pub fn encode(&self) -> [u8; IO_SIZE] {
        let s = self.slot.to_le_bytes();
        [s[0], s[1], self.kind as u8, self.pin]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    TooShort,
    BadMagic,
    BadVersion,
    BadLength,
    BadCrc,
    /// Le programme ne tient pas dans cet automate.
    TooLarge,
    BadIo,
    BadInstr,
    /// Une instruction référence un slot ou un saut hors du programme.
    BadRef,
}

impl LoadError {
    pub fn code(self) -> u8 {
        self as u8 + 1
    }

    pub fn from_code(c: u8) -> Option<LoadError> {
        use LoadError::*;
        [TooShort, BadMagic, BadVersion, BadLength, BadCrc, TooLarge, BadIo, BadInstr, BadRef]
            .get(c.checked_sub(1)? as usize)
            .copied()
    }

    pub fn describe(self) -> &'static str {
        match self {
            LoadError::TooShort => "image trop courte",
            LoadError::BadMagic => "ce n'est pas un programme Takt",
            LoadError::BadVersion => "version d'image non supportée par ce firmware",
            LoadError::BadLength => "longueur incohérente",
            LoadError::BadCrc => "CRC faux (transfert corrompu)",
            LoadError::TooLarge => "programme trop gros pour cet automate",
            LoadError::BadIo => "entrée/sortie invalide",
            LoadError::BadInstr => "instruction inconnue",
            LoadError::BadRef => "référence mémoire ou saut invalide",
        }
    }
}

pub fn write_header(h: &Header, out: &mut [u8; HEADER_SIZE]) {
    *out = [0; HEADER_SIZE];
    out[0..4].copy_from_slice(&MAGIC);
    out[4..6].copy_from_slice(&VERSION.to_le_bytes());
    out[8..12].copy_from_slice(&h.cycle_us.to_le_bytes());
    out[12..14].copy_from_slice(&h.mem_slots.to_le_bytes());
    out[14..16].copy_from_slice(&h.max_stack.to_le_bytes());
    out[16..20].copy_from_slice(&h.wcet_cycles.to_le_bytes());
    out[20..22].copy_from_slice(&h.n_io.to_le_bytes());
    out[22..24].copy_from_slice(&h.n_init.to_le_bytes());
    out[24..28].copy_from_slice(&h.n_code.to_le_bytes());
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn image_len(h: &Header) -> usize {
    HEADER_SIZE + IO_SIZE * h.n_io as usize + INIT_SIZE * h.n_init as usize + Instr::SIZE * h.n_code as usize + 4
}

/// CRC-32 (IEEE 802.3), version sans table : lente mais seulement au chargement.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Vue validée sur une image binaire.
#[derive(Clone, Copy, Debug)]
pub struct ImageView<'a> {
    pub header: Header,
    pub crc: u32,
    bytes: &'a [u8],
}

impl<'a> ImageView<'a> {
    /// Vérifie l'image en entier : CRC, tailles, et que chaque instruction ne
    /// touche que des slots et des adresses de saut valides. Après ça,
    /// l'exécution ne peut plus faire de faute d'accès.
    pub fn parse(bytes: &'a [u8]) -> Result<ImageView<'a>, LoadError> {
        if bytes.len() < HEADER_SIZE + 4 {
            return Err(LoadError::TooShort);
        }
        if bytes[0..4] != MAGIC {
            return Err(LoadError::BadMagic);
        }
        if u16_at(bytes, 4) != VERSION {
            return Err(LoadError::BadVersion);
        }
        let header = Header {
            cycle_us: u32_at(bytes, 8),
            mem_slots: u16_at(bytes, 12),
            max_stack: u16_at(bytes, 14),
            wcet_cycles: u32_at(bytes, 16),
            n_io: u16_at(bytes, 20),
            n_init: u16_at(bytes, 22),
            n_code: u32_at(bytes, 24),
        };
        if header.n_code > u16::MAX as u32 || image_len(&header) != bytes.len() {
            return Err(LoadError::BadLength);
        }
        let body = bytes.len() - 4;
        let crc = u32_at(bytes, body);
        if crc32(&bytes[..body]) != crc {
            return Err(LoadError::BadCrc);
        }
        if header.max_stack as usize > STACK_MAX || header.cycle_us == 0 {
            return Err(LoadError::TooLarge);
        }
        let view = ImageView { header, crc, bytes };
        let slots = header.mem_slots as u32;
        for i in 0..header.n_io as usize {
            let io = view.io(i).ok_or(LoadError::BadIo)?;
            if io.slot as u32 >= slots {
                return Err(LoadError::BadIo);
            }
        }
        for i in 0..header.n_init as usize {
            let (slot, _) = view.init(i);
            if slot as u32 >= slots {
                return Err(LoadError::BadRef);
            }
        }
        for i in 0..header.n_code as usize {
            let ins = view.instr(i).ok_or(LoadError::BadInstr)?;
            if let Some((first, n)) = ins.slot_range()
                && (first as u32 + n as u32 > slots || n == 0)
            {
                return Err(LoadError::BadRef);
            }
            if let Some(t) = ins.jump_target()
                && t as u32 > header.n_code
            {
                return Err(LoadError::BadRef);
            }
        }
        Ok(view)
    }

    pub fn io(&self, i: usize) -> Option<IoEntry> {
        let o = HEADER_SIZE + i * IO_SIZE;
        Some(IoEntry { slot: u16_at(self.bytes, o), kind: IoKind::from_u8(self.bytes[o + 2])?, pin: self.bytes[o + 3] })
    }

    pub fn init(&self, i: usize) -> (u16, u32) {
        let o = HEADER_SIZE + self.header.n_io as usize * IO_SIZE + i * INIT_SIZE;
        (u16_at(self.bytes, o), u32_at(self.bytes, o + 4))
    }

    pub fn instr(&self, i: usize) -> Option<Instr> {
        let o = HEADER_SIZE
            + self.header.n_io as usize * IO_SIZE
            + self.header.n_init as usize * INIT_SIZE
            + i * Instr::SIZE;
        Instr::decode(&self.bytes[o..o + Instr::SIZE])
    }
}
