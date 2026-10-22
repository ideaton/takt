//! Interpréteur d'un cycle.
//!
//! Aucune opération ne peut paniquer ni boucler sans fin : division par zéro
//! et index hors bornes donnent un résultat défini et lèvent un drapeau de
//! diagnostic, et le nombre d'instructions exécutées est plafonné par la borne
//! calculée à la compilation.

use crate::instr::Instr;

pub const STACK_MAX: usize = 64;

/// Drapeaux de diagnostic (cumulés par le runtime).
pub const DIAG_DIV0: u16 = 1;
pub const DIAG_INDEX: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Fault {
    StackOverflow = 1,
    StackUnderflow = 2,
    BadSlot = 3,
    BadJump = 4,
    /// Plus d'instructions exécutées que la borne prouvée : ne doit jamais arriver.
    StepLimit = 5,
}

impl Fault {
    pub fn from_u8(v: u8) -> Option<Fault> {
        Some(match v {
            1 => Fault::StackOverflow,
            2 => Fault::StackUnderflow,
            3 => Fault::BadSlot,
            4 => Fault::BadJump,
            5 => Fault::StepLimit,
            _ => return None,
        })
    }

    pub fn describe(self) -> &'static str {
        match self {
            Fault::StackOverflow => "débordement de pile",
            Fault::StackUnderflow => "pile vide",
            Fault::BadSlot => "accès mémoire hors programme",
            Fault::BadJump => "saut invalide",
            Fault::StepLimit => "borne d'exécution dépassée",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanOk {
    pub steps: u32,
    pub diag: u16,
}

struct Stack {
    d: [u32; STACK_MAX],
    sp: usize,
}

impl Stack {
    #[inline(always)]
    fn push(&mut self, v: u32) -> Result<(), Fault> {
        let slot = self.d.get_mut(self.sp).ok_or(Fault::StackOverflow)?;
        *slot = v;
        self.sp += 1;
        Ok(())
    }

    #[inline(always)]
    fn pop(&mut self) -> Result<u32, Fault> {
        if self.sp == 0 {
            return Err(Fault::StackUnderflow);
        }
        self.sp -= 1;
        Ok(self.d[self.sp])
    }

    #[inline(always)]
    fn pop2i(&mut self) -> Result<(i32, i32), Fault> {
        let b = self.pop()? as i32;
        let a = self.pop()? as i32;
        Ok((a, b))
    }

    #[inline(always)]
    fn pop2f(&mut self) -> Result<(f32, f32), Fault> {
        let b = f32::from_bits(self.pop()?);
        let a = f32::from_bits(self.pop()?);
        Ok((a, b))
    }
}

#[inline(always)]
fn fv(v: f32) -> u32 {
    v.to_bits()
}

#[inline(always)]
fn bv(v: bool) -> u32 {
    v as u32
}

fn clamp_index(idx: u32, len: u16, diag: &mut u16) -> usize {
    let i = idx as i32;
    if i < 0 || i >= len as i32 {
        *diag |= DIAG_INDEX;
        if i < 0 { 0 } else { len.saturating_sub(1) as usize }
    } else {
        i as usize
    }
}

fn f2i_round(f: f32) -> i32 {
    if f.is_nan() {
        return 0;
    }
    let r = if f >= 0.0 { f + 0.5 } else { f - 0.5 };
    r as i32
}

/// Exécute le programme une fois, du début jusqu'à `Halt` ou la fin du code.
pub fn scan(code: &[Instr], mem: &mut [u32], now_ms: u32, max_steps: u32) -> Result<ScanOk, Fault> {
    use Instr::*;
    let mut st = Stack { d: [0; STACK_MAX], sp: 0 };
    let mut pc = 0usize;
    let mut steps = 0u32;
    let mut diag = 0u16;

    while let Some(&ins) = code.get(pc) {
        steps += 1;
        if steps > max_steps {
            return Err(Fault::StepLimit);
        }
        pc += 1;
        match ins {
            Nop => {}
            PushI(v) => st.push(v as u32)?,
            PushF(v) => st.push(fv(v))?,
            Load(s) => st.push(*mem.get(s as usize).ok_or(Fault::BadSlot)?)?,
            Store(s) => {
                let v = st.pop()?;
                *mem.get_mut(s as usize).ok_or(Fault::BadSlot)? = v;
            }
            LoadIdx { base, len } => {
                let i = clamp_index(st.pop()?, len, &mut diag);
                st.push(*mem.get(base as usize + i).ok_or(Fault::BadSlot)?)?;
            }
            StoreIdx { base, len } => {
                let v = st.pop()?;
                let i = clamp_index(st.pop()?, len, &mut diag);
                *mem.get_mut(base as usize + i).ok_or(Fault::BadSlot)? = v;
            }
            AddI => {
                let (a, b) = st.pop2i()?;
                st.push(a.wrapping_add(b) as u32)?
            }
            SubI => {
                let (a, b) = st.pop2i()?;
                st.push(a.wrapping_sub(b) as u32)?
            }
            MulI => {
                let (a, b) = st.pop2i()?;
                st.push(a.wrapping_mul(b) as u32)?
            }
            DivI | ModI => {
                let (a, b) = st.pop2i()?;
                let r = if b == 0 {
                    diag |= DIAG_DIV0;
                    0
                } else if matches!(ins, DivI) {
                    a.wrapping_div(b)
                } else {
                    a.wrapping_rem(b)
                };
                st.push(r as u32)?
            }
            NegI => {
                let a = st.pop()? as i32;
                st.push(a.wrapping_neg() as u32)?
            }
            AbsI => {
                let a = st.pop()? as i32;
                st.push(a.wrapping_abs() as u32)?
            }
            MinI => {
                let (a, b) = st.pop2i()?;
                st.push(a.min(b) as u32)?
            }
            MaxI => {
                let (a, b) = st.pop2i()?;
                st.push(a.max(b) as u32)?
            }
            AddF => {
                let (a, b) = st.pop2f()?;
                st.push(fv(a + b))?
            }
            SubF => {
                let (a, b) = st.pop2f()?;
                st.push(fv(a - b))?
            }
            MulF => {
                let (a, b) = st.pop2f()?;
                st.push(fv(a * b))?
            }
            DivF => {
                let (a, b) = st.pop2f()?;
                let r = if b == 0.0 {
                    diag |= DIAG_DIV0;
                    0.0
                } else {
                    a / b
                };
                st.push(fv(r))?
            }
            NegF => {
                let a = st.pop()?;
                st.push(a ^ 0x8000_0000)?
            }
            AbsF => {
                let a = st.pop()?;
                st.push(a & 0x7fff_ffff)?
            }
            MinF => {
                let (a, b) = st.pop2f()?;
                st.push(fv(if b < a { b } else { a }))?
            }
            MaxF => {
                let (a, b) = st.pop2f()?;
                st.push(fv(if b > a { b } else { a }))?
            }
            EqI | NeI | LtI | LeI | GtI | GeI => {
                let (a, b) = st.pop2i()?;
                let r = match ins {
                    EqI => a == b,
                    NeI => a != b,
                    LtI => a < b,
                    LeI => a <= b,
                    GtI => a > b,
                    _ => a >= b,
                };
                st.push(bv(r))?
            }
            EqF | NeF | LtF | LeF | GtF | GeF => {
                let (a, b) = st.pop2f()?;
                let r = match ins {
                    EqF => a == b,
                    NeF => a != b,
                    LtF => a < b,
                    LeF => a <= b,
                    GtF => a > b,
                    _ => a >= b,
                };
                st.push(bv(r))?
            }
            And | Or | Xor => {
                let b = st.pop()? != 0;
                let a = st.pop()? != 0;
                let r = match ins {
                    And => a && b,
                    Or => a || b,
                    _ => a != b,
                };
                st.push(bv(r))?
            }
            Not => {
                let a = st.pop()?;
                st.push(bv(a == 0))?
            }
            I2F => {
                let a = st.pop()? as i32;
                st.push(fv(a as f32))?
            }
            F2IRound => {
                let a = f32::from_bits(st.pop()?);
                st.push(f2i_round(a) as u32)?
            }
            F2ITrunc => {
                let a = f32::from_bits(st.pop()?);
                st.push(a as i32 as u32)?
            }
            WrapI16 => {
                let a = st.pop()?;
                st.push(a as i16 as i32 as u32)?
            }
            ToBool => {
                let a = st.pop()?;
                st.push(bv(a != 0))?
            }
            Jmp(t) => {
                if t as usize > code.len() {
                    return Err(Fault::BadJump);
                }
                pc = t as usize;
            }
            Jz(t) => {
                if st.pop()? == 0 {
                    if t as usize > code.len() {
                        return Err(Fault::BadJump);
                    }
                    pc = t as usize;
                }
            }
            Fb { kind, base } => {
                let b = base as usize;
                let m = mem.get_mut(b..b + kind.size() as usize).ok_or(Fault::BadSlot)?;
                kind.exec(m, now_ms);
            }
            Halt => break,
        }
    }
    Ok(ScanOk { steps, diag })
}
