//! Blocs standard IEC 61131-3.
//!
//! Chaque instance occupe des slots consécutifs dans la mémoire de la VM. Les
//! champs `Hidden` sont l'état interne du bloc. Le temps est celui figé au
//! début du cycle, donc deux lectures dans le même cycle voient la même valeur.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FbKind {
    Ton = 0,
    Tof = 1,
    Tp = 2,
    RTrig = 3,
    FTrig = 4,
    Ctu = 5,
    Ctd = 6,
    Sr = 7,
    Rs = 8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldTy {
    Bool,
    Int,
    Time,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    In,
    Out,
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub name: &'static str,
    pub ty: FieldTy,
    pub dir: Dir,
}

const fn f(name: &'static str, ty: FieldTy, dir: Dir) -> Field {
    Field { name, ty, dir }
}

use Dir::*;
use FieldTy::*;

const TIMER: [Field; 6] = [
    f("IN", Bool, In),
    f("PT", Time, In),
    f("Q", Bool, Out),
    f("ET", Time, Out),
    f("_START", Time, Hidden),
    f("_STATE", Int, Hidden),
];
const R_TRIG: [Field; 3] = [f("CLK", Bool, In), f("Q", Bool, Out), f("_M", Bool, Hidden)];
const CTU: [Field; 6] = [
    f("CU", Bool, In),
    f("R", Bool, In),
    f("PV", Int, In),
    f("Q", Bool, Out),
    f("CV", Int, Out),
    f("_PREV", Bool, Hidden),
];
const CTD: [Field; 6] = [
    f("CD", Bool, In),
    f("LD", Bool, In),
    f("PV", Int, In),
    f("Q", Bool, Out),
    f("CV", Int, Out),
    f("_PREV", Bool, Hidden),
];
const SR: [Field; 3] = [f("S1", Bool, In), f("R", Bool, In), f("Q1", Bool, Out)];
const RS: [Field; 3] = [f("S", Bool, In), f("R1", Bool, In), f("Q1", Bool, Out)];

pub const ALL: [FbKind; 9] = [
    FbKind::Ton,
    FbKind::Tof,
    FbKind::Tp,
    FbKind::RTrig,
    FbKind::FTrig,
    FbKind::Ctu,
    FbKind::Ctd,
    FbKind::Sr,
    FbKind::Rs,
];

impl FbKind {
    pub fn from_u8(v: u8) -> Option<FbKind> {
        ALL.get(v as usize).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            FbKind::Ton => "TON",
            FbKind::Tof => "TOF",
            FbKind::Tp => "TP",
            FbKind::RTrig => "R_TRIG",
            FbKind::FTrig => "F_TRIG",
            FbKind::Ctu => "CTU",
            FbKind::Ctd => "CTD",
            FbKind::Sr => "SR",
            FbKind::Rs => "RS",
        }
    }

    pub fn from_name(name: &str) -> Option<FbKind> {
        ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(name))
    }

    pub fn fields(self) -> &'static [Field] {
        match self {
            FbKind::Ton | FbKind::Tof | FbKind::Tp => &TIMER,
            FbKind::RTrig | FbKind::FTrig => &R_TRIG,
            FbKind::Ctu => &CTU,
            FbKind::Ctd => &CTD,
            FbKind::Sr => &SR,
            FbKind::Rs => &RS,
        }
    }

    pub fn size(self) -> u16 {
        self.fields().len() as u16
    }

    /// Exécute le bloc sur sa mémoire d'instance `m` (longueur `size()`).
    pub fn exec(self, m: &mut [u32], now_ms: u32) {
        let b = |v: u32| v != 0;
        match self {
            FbKind::Ton => {
                // IN PT Q ET _START _STATE
                if !b(m[0]) {
                    m[2] = 0;
                    m[3] = 0;
                    m[5] = 0;
                } else {
                    if m[5] == 0 {
                        m[5] = 1;
                        m[4] = now_ms;
                    }
                    let pt = pt(m[1]);
                    let et = now_ms.wrapping_sub(m[4]).min(pt);
                    m[3] = et;
                    m[2] = (et >= pt) as u32;
                }
            }
            FbKind::Tof => {
                if b(m[0]) {
                    m[2] = 1;
                    m[3] = 0;
                    m[5] = 0;
                } else if b(m[2]) {
                    if m[5] == 0 {
                        m[5] = 1;
                        m[4] = now_ms;
                    }
                    let pt = pt(m[1]);
                    let et = now_ms.wrapping_sub(m[4]).min(pt);
                    m[3] = et;
                    if et >= pt {
                        m[2] = 0;
                    }
                }
            }
            FbKind::Tp => {
                // _STATE : 0 au repos, 1 impulsion en cours, 2 attente de IN = FALSE
                match m[5] {
                    0 => {
                        if b(m[0]) {
                            m[5] = 1;
                            m[4] = now_ms;
                            m[2] = 1;
                            m[3] = 0;
                        }
                    }
                    1 => {
                        let pt = pt(m[1]);
                        let et = now_ms.wrapping_sub(m[4]).min(pt);
                        m[3] = et;
                        if et >= pt {
                            m[2] = 0;
                            m[5] = 2;
                        }
                    }
                    _ => {
                        if !b(m[0]) {
                            m[5] = 0;
                            m[3] = 0;
                        }
                    }
                }
            }
            FbKind::RTrig => {
                m[1] = (b(m[0]) && !b(m[2])) as u32;
                m[2] = b(m[0]) as u32;
            }
            FbKind::FTrig => {
                // M mémorise NOT CLK, comme dans la norme
                m[1] = (!b(m[0]) && !b(m[2])) as u32;
                m[2] = !b(m[0]) as u32;
            }
            FbKind::Ctu => {
                let cv = m[4] as i32;
                if b(m[1]) {
                    m[4] = 0;
                } else if b(m[0]) && !b(m[5]) && cv < i16::MAX as i32 {
                    m[4] = (cv + 1) as u32;
                }
                m[5] = b(m[0]) as u32;
                m[3] = (m[4] as i32 >= m[2] as i32) as u32;
            }
            FbKind::Ctd => {
                let cv = m[4] as i32;
                if b(m[1]) {
                    m[4] = m[2];
                } else if b(m[0]) && !b(m[5]) && cv > i16::MIN as i32 {
                    m[4] = (cv - 1) as u32;
                }
                m[5] = b(m[0]) as u32;
                m[3] = (m[4] as i32 <= 0) as u32;
            }
            FbKind::Sr => m[2] = (b(m[0]) || (!b(m[1]) && b(m[2]))) as u32,
            FbKind::Rs => m[2] = (!b(m[1]) && (b(m[0]) || b(m[2]))) as u32,
        }
    }
}

/// Une durée négative vaut 0.
fn pt(v: u32) -> u32 {
    (v as i32).max(0) as u32
}
