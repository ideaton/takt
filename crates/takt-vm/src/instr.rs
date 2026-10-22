//! Jeu d'instructions de la VM.
//!
//! Machine à pile. Toutes les valeurs font 32 bits : BOOL (0/1), INT et DINT
//! (complément à deux), REAL (f32), TIME (ms sur i32). Chaque instruction est
//! encodée sur 8 octets : `op u8 | a u8 | b u16 | x u32` (little endian).

use crate::fb::FbKind;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Instr {
    Nop,
    PushI(i32),
    PushF(f32),
    Load(u16),
    Store(u16),
    /// Dépile un index (base 0) et empile `mem[base + index]`.
    LoadIdx {
        base: u16,
        len: u16,
    },
    /// Dépile une valeur puis un index (base 0) et écrit `mem[base + index]`.
    StoreIdx {
        base: u16,
        len: u16,
    },
    AddI,
    SubI,
    MulI,
    DivI,
    ModI,
    NegI,
    AbsI,
    MinI,
    MaxI,
    AddF,
    SubF,
    MulF,
    DivF,
    NegF,
    AbsF,
    MinF,
    MaxF,
    EqI,
    NeI,
    LtI,
    LeI,
    GtI,
    GeI,
    EqF,
    NeF,
    LtF,
    LeF,
    GtF,
    GeF,
    And,
    Or,
    Xor,
    Not,
    I2F,
    /// REAL → DINT, arrondi au plus proche (comme `REAL_TO_DINT` de Siemens).
    F2IRound,
    /// REAL → DINT, troncature (`TRUNC`).
    F2ITrunc,
    /// Ramène un entier dans la plage d'un INT 16 bits (débordement en boucle).
    WrapI16,
    /// Normalise une valeur en BOOL (0 ou 1).
    ToBool,
    Jmp(u16),
    /// Dépile une valeur et saute si elle vaut 0.
    Jz(u16),
    /// Exécute un bloc standard dont l'instance commence au slot `base`.
    Fb {
        kind: FbKind,
        base: u16,
    },
    Halt,
}

impl Instr {
    pub const SIZE: usize = 8;

    /// Effet sur la profondeur de pile (utilisé par le compilateur).
    pub fn stack_effect(&self) -> i32 {
        use Instr::*;
        match self {
            PushI(_) | PushF(_) | Load(_) => 1,
            Store(_) | Jz(_) => -1,
            StoreIdx { .. } => -2,
            AddI | SubI | MulI | DivI | ModI | MinI | MaxI | AddF | SubF | MulF | DivF | MinF | MaxF | EqI | NeI
            | LtI | LeI | GtI | GeI | EqF | NeF | LtF | LeF | GtF | GeF | And | Or | Xor => -1,
            _ => 0,
        }
    }

    /// Slots mémoire touchés par l'instruction : `(premier, nombre)`.
    pub fn slot_range(&self) -> Option<(u16, u16)> {
        match *self {
            Instr::Load(s) | Instr::Store(s) => Some((s, 1)),
            Instr::LoadIdx { base, len } | Instr::StoreIdx { base, len } => Some((base, len)),
            Instr::Fb { kind, base } => Some((base, kind.size())),
            _ => None,
        }
    }

    pub fn jump_target(&self) -> Option<u16> {
        match *self {
            Instr::Jmp(t) | Instr::Jz(t) => Some(t),
            _ => None,
        }
    }

    pub fn encode(&self) -> [u8; 8] {
        use Instr::*;
        let (op, a, b, x): (u8, u8, u16, u32) = match *self {
            PushI(v) => (1, 0, 0, v as u32),
            PushF(v) => (2, 0, 0, v.to_bits()),
            Load(s) => (3, 0, s, 0),
            Store(s) => (4, 0, s, 0),
            LoadIdx { base, len } => (5, 0, base, len as u32),
            StoreIdx { base, len } => (6, 0, base, len as u32),
            Jmp(t) => (70, 0, t, 0),
            Jz(t) => (71, 0, t, 0),
            Fb { kind, base } => (80, kind as u8, base, 0),
            simple => (simple_opcode(simple), 0, 0, 0),
        };
        let mut o = [0u8; 8];
        o[0] = op;
        o[1] = a;
        o[2..4].copy_from_slice(&b.to_le_bytes());
        o[4..8].copy_from_slice(&x.to_le_bytes());
        o
    }

    pub fn decode(o: &[u8]) -> Option<Instr> {
        use Instr::*;
        if o.len() < 8 {
            return None;
        }
        let a = o[1];
        let b = u16::from_le_bytes([o[2], o[3]]);
        let x = u32::from_le_bytes([o[4], o[5], o[6], o[7]]);
        Some(match o[0] {
            1 => PushI(x as i32),
            2 => PushF(f32::from_bits(x)),
            3 => Load(b),
            4 => Store(b),
            5 => LoadIdx { base: b, len: u16::try_from(x).ok()? },
            6 => StoreIdx { base: b, len: u16::try_from(x).ok()? },
            70 => Jmp(b),
            71 => Jz(b),
            80 => Fb { kind: FbKind::from_u8(a)?, base: b },
            op => SIMPLE.iter().find(|(c, _)| *c == op)?.1,
        })
    }
}

/// Instructions sans opérande et leur code.
const SIMPLE: [(u8, Instr); 40] = {
    use Instr::*;
    [
        (0, Nop),
        (10, AddI),
        (11, SubI),
        (12, MulI),
        (13, DivI),
        (14, ModI),
        (15, NegI),
        (16, AbsI),
        (17, MinI),
        (18, MaxI),
        (20, AddF),
        (21, SubF),
        (22, MulF),
        (23, DivF),
        (24, NegF),
        (25, AbsF),
        (26, MinF),
        (27, MaxF),
        (30, EqI),
        (31, NeI),
        (32, LtI),
        (33, LeI),
        (34, GtI),
        (35, GeI),
        (40, EqF),
        (41, NeF),
        (42, LtF),
        (43, LeF),
        (44, GtF),
        (45, GeF),
        (50, And),
        (51, Or),
        (52, Xor),
        (53, Not),
        (60, I2F),
        (61, F2IRound),
        (62, F2ITrunc),
        (63, WrapI16),
        (64, ToBool),
        (255, Halt),
    ]
};

fn simple_opcode(i: Instr) -> u8 {
    SIMPLE.iter().find(|(_, s)| *s == i).map(|(c, _)| *c).unwrap_or(0)
}
