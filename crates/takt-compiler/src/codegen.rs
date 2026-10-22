//! Analyse sémantique, génération du bytecode et calcul du WCET en une passe.
//!
//! Chaque fonction de compilation renvoie le coût au pire cas, en cycles CPU,
//! du code qu'elle a émis. Le calcul est structurel :
//! - séquence : somme ;
//! - IF / CASE : toutes les conditions + la branche la plus chère ;
//! - FOR : bornes constantes obligatoires, donc nombre d'itérations connu ;
//! - FB et FUNCTION utilisateur : développés en ligne à chaque appel, la
//!   récursion est refusée.
//!
//! `WHILE` et `REPEAT` sont refusés : leur nombre d'itérations dépend des
//! données, le temps de cycle ne serait plus prouvable.

use std::collections::HashMap;
use std::rc::Rc;

use takt_vm::cost;
use takt_vm::fb::{Dir, FieldTy};
use takt_vm::image::IoKind;
use takt_vm::{FbKind, Instr, sys};

use crate::ast::*;
use crate::lexer::Pos;
use crate::types::{FbType, Ty};
use crate::{Diag, Severity, Symbol};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConstVal {
    Int(i64),
    Real(f64),
    Bool(bool),
    Time(i64),
}

impl ConstVal {
    fn ty(&self) -> Ty {
        match self {
            ConstVal::Int(v) if i16::try_from(*v).is_ok() => Ty::Int,
            ConstVal::Int(_) => Ty::DInt,
            ConstVal::Real(_) => Ty::Real,
            ConstVal::Bool(_) => Ty::Bool,
            ConstVal::Time(_) => Ty::Time,
        }
    }

    /// Valeur brute pour la VM, convertie vers le type `ty`.
    pub fn bits(&self, ty: &Ty) -> Result<u32, String> {
        let range = |v: i64, lo: i64, hi: i64, t: &str| {
            if (lo..=hi).contains(&v) { Ok(v as i32 as u32) } else { Err(format!("{v} hors de la plage {t}")) }
        };
        match (self, ty) {
            (ConstVal::Bool(b), Ty::Bool) => Ok(*b as u32),
            (ConstVal::Int(v), Ty::Int) => range(*v, i16::MIN as i64, i16::MAX as i64, "INT"),
            (ConstVal::Int(v), Ty::DInt) => range(*v, i32::MIN as i64, i32::MAX as i64, "DINT"),
            (ConstVal::Int(v), Ty::Real) => Ok((*v as f32).to_bits()),
            (ConstVal::Real(v), Ty::Real) => Ok((*v as f32).to_bits()),
            (ConstVal::Time(v), Ty::Time) => range(*v, i32::MIN as i64, i32::MAX as i64, "TIME"),
            (v, t) => Err(format!("valeur de type {} incompatible avec {}", v.ty().name(), t.name())),
        }
    }
}

/// Évalue une expression constante (littéraux, constantes, arithmétique entière).
pub fn const_eval(e: &Expr, consts: &HashMap<String, ConstVal>) -> Option<ConstVal> {
    use ConstVal::*;
    Some(match e {
        Expr::Int(v, _) => Int(*v),
        Expr::Real(v, _) => Real(*v),
        Expr::Bool(v, _) => Bool(*v),
        Expr::Time(v, _) => Time(*v),
        Expr::Name { name, scope, .. } if *scope != NameScope::Global => *consts.get(&name.to_ascii_uppercase())?,
        Expr::Unary { op: UnOp::Neg, e, .. } => match const_eval(e, consts)? {
            Int(v) => Int(-v),
            Real(v) => Real(-v),
            Time(v) => Time(-v),
            Bool(_) => return None,
        },
        Expr::Unary { op: UnOp::Not, e, .. } => match const_eval(e, consts)? {
            Bool(v) => Bool(!v),
            _ => return None,
        },
        Expr::Binary { op, l, r, .. } => {
            let (l, r) = (const_eval(l, consts)?, const_eval(r, consts)?);
            match (l, r) {
                (Int(a), Int(b)) => Int(match op {
                    BinOp::Add => a.checked_add(b)?,
                    BinOp::Sub => a.checked_sub(b)?,
                    BinOp::Mul => a.checked_mul(b)?,
                    BinOp::Div => a.checked_div(b)?,
                    BinOp::Mod => a.checked_rem(b)?,
                    _ => return None,
                }),
                (Real(a), Real(b)) => Real(match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div if b != 0.0 => a / b,
                    _ => return None,
                }),
                (Time(a), Time(b)) => Time(match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    _ => return None,
                }),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Champ d'un FUNCTION_BLOCK ou d'une FUNCTION utilisateur.
#[derive(Clone, Debug)]
struct FieldInfo {
    name: String,
    offset: u16,
    ty: Ty,
    section: VarSection,
    init: Option<u32>,
}

#[derive(Debug)]
struct Layout {
    size: u16,
    fields: Vec<FieldInfo>,
    by_key: HashMap<String, usize>,
    consts: HashMap<String, ConstVal>,
}

impl Layout {
    fn field(&self, name: &str) -> Option<&FieldInfo> {
        self.by_key.get(&name.to_ascii_uppercase()).map(|&i| &self.fields[i])
    }
}

#[derive(Clone, Debug)]
enum Local {
    Var { slot: u16, ty: Ty, read_only: bool },
    Const(ConstVal),
}

struct Frame {
    names: HashMap<String, Local>,
    /// Sauts `RETURN` à reporter en fin de bloc.
    returns: Vec<usize>,
}

/// Emplacement d'une expression adressable.
#[derive(Clone, Debug)]
enum Place<'e> {
    Slot { slot: u16, ty: Ty, read_only: bool },
    Indexed { base: u16, len: u16, lo: i32, elem: Ty, index: &'e Expr },
    Const(ConstVal),
}

#[derive(Clone, Debug)]
pub struct GlobalVar {
    pub name: String,
    pub slot: u16,
    pub ty: Ty,
    pub read_only: bool,
}

pub struct Gen<'a> {
    pub code: Vec<Instr>,
    depth: i32,
    pub max_depth: i32,
    next_slot: u32,
    pub globals: HashMap<String, GlobalVar>,
    defs: HashMap<String, (&'a Pou, String)>,
    layouts: HashMap<String, Rc<Layout>>,
    layout_busy: Vec<String>,
    fn_frames: HashMap<String, (u16, Rc<Layout>)>,
    inline_stack: Vec<String>,
    inlined: std::collections::HashSet<String>,
    frames: Vec<Frame>,
    loop_slots: Vec<u16>,
    pub diags: Vec<Diag>,
    block: String,
    pub symbols: Vec<Symbol>,
    pub init: Vec<(u16, u32)>,
    /// Instances de FB utilisateur : (type en majuscules, chemin).
    pub instances: Vec<(String, String)>,
}

/// Erreur déjà signalée dans `diags`.
#[derive(Debug)]
pub struct Reported;
type R<T> = Result<T, Reported>;

impl<'a> Gen<'a> {
    pub fn new() -> Gen<'a> {
        let mut g = Gen {
            code: Vec::new(),
            depth: 0,
            max_depth: 0,
            next_slot: sys::SLOTS as u32,
            globals: HashMap::new(),
            defs: HashMap::new(),
            layouts: HashMap::new(),
            layout_busy: Vec::new(),
            fn_frames: HashMap::new(),
            inline_stack: Vec::new(),
            inlined: Default::default(),
            frames: Vec::new(),
            loop_slots: Vec::new(),
            diags: Vec::new(),
            block: String::new(),
            symbols: Vec::new(),
            init: Vec::new(),
            instances: Vec::new(),
        };
        for (name, slot, ty) in [("FirstScan", sys::FIRST_SCAN, Ty::Bool), ("SysTime", sys::SYS_TIME, Ty::Time)] {
            g.globals.insert(
                name.to_ascii_uppercase(),
                GlobalVar { name: name.into(), slot, ty: ty.clone(), read_only: true },
            );
            g.symbols.push(Symbol { path: name.into(), block: None, slot, ty, io: None });
        }
        g
    }

    pub fn mem_slots(&self) -> u32 {
        self.next_slot
    }

    pub fn set_block(&mut self, name: &str) {
        self.block = name.into();
    }

    pub fn error(&mut self, pos: Pos, msg: impl Into<String>) -> Reported {
        self.diags.push(Diag { block: self.block.clone(), pos, severity: Severity::Error, msg: msg.into() });
        Reported
    }

    pub fn warn(&mut self, pos: Pos, msg: impl Into<String>) {
        self.diags.push(Diag { block: self.block.clone(), pos, severity: Severity::Warning, msg: msg.into() });
    }

    /// Enregistre un FUNCTION_BLOCK ou une FUNCTION (avec le nom du bloc projet qui le contient).
    pub fn define(&mut self, pou: &'a Pou, block: &str) {
        let key = pou.name.to_ascii_uppercase();
        if FbKind::from_name(&key).is_some() || is_builtin_fn(&key) {
            self.set_block(block);
            self.error(pou.pos, format!("« {} » est un nom réservé", pou.name));
            return;
        }
        if self.defs.contains_key(&key) {
            self.set_block(block);
            self.error(pou.pos, format!("« {} » est défini deux fois", pou.name));
            return;
        }
        self.defs.insert(key, (pou, block.into()));
    }

    // ---------- mémoire ----------

    fn alloc_raw(&mut self, size: u32, pos: Pos) -> R<u16> {
        let base = self.next_slot;
        self.next_slot += size;
        if self.next_slot > u16::MAX as u32 {
            return Err(self.error(pos, "mémoire de l'automate épuisée"));
        }
        Ok(base as u16)
    }

    pub fn add_global(&mut self, name: &str, ty: Ty, io: Option<IoKind>, pos: Pos) -> R<u16> {
        let key = name.to_ascii_uppercase();
        if self.globals.contains_key(&key) {
            return Err(self.error(pos, format!("la variable API « {name} » existe déjà")));
        }
        let size = self.size_of(&ty, pos)?;
        let slot = self.alloc_raw(size as u32, pos)?;
        self.globals.insert(key, GlobalVar { name: name.into(), slot, ty: ty.clone(), read_only: false });
        self.symbols.push(Symbol { path: name.into(), block: None, slot, ty, io });
        Ok(slot)
    }

    fn size_of(&mut self, ty: &Ty, pos: Pos) -> R<u16> {
        Ok(match ty {
            Ty::Array { elem, len, .. } => self.size_of(elem, pos)? * *len,
            Ty::Fb(FbType::Std(k)) => k.size(),
            Ty::Fb(FbType::User(n)) => self.layout(n, pos)?.size,
            _ => 1,
        })
    }

    /// Réserve la mémoire d'une variable, ses valeurs initiales et ses symboles.
    fn alloc_var(&mut self, ty: &Ty, init: Option<u32>, path: &str, block: &str, pos: Pos) -> R<u16> {
        let size = self.size_of(ty, pos)?;
        let base = self.alloc_raw(size as u32, pos)?;
        self.place_var(base, ty, init, path, block, pos)?;
        Ok(base)
    }

    fn place_var(&mut self, base: u16, ty: &Ty, init: Option<u32>, path: &str, block: &str, pos: Pos) -> R<()> {
        if let Some(v) = init
            && v != 0
        {
            self.init.push((base, v));
        }
        match ty {
            Ty::Fb(FbType::Std(k)) => {
                for (i, f) in k.fields().iter().enumerate() {
                    if f.dir != Dir::Hidden {
                        self.symbols.push(Symbol {
                            path: format!("{path}.{}", f.name),
                            block: Some(block.into()),
                            slot: base + i as u16,
                            ty: field_ty(f.ty),
                            io: None,
                        });
                    }
                }
            }
            Ty::Fb(FbType::User(n)) => {
                self.instances.push((n.clone(), path.to_string()));
                let layout = self.layout(n, pos)?;
                for f in &layout.fields {
                    self.place_var(base + f.offset, &f.ty, f.init, &format!("{path}.{}", f.name), block, pos)?;
                }
            }
            _ => self.symbols.push(Symbol {
                path: path.into(),
                block: Some(block.into()),
                slot: base,
                ty: ty.clone(),
                io: None,
            }),
        }
        Ok(())
    }

    // ---------- types ----------

    pub fn resolve_type(&mut self, t: &TypeRef, consts: &HashMap<String, ConstVal>) -> R<Ty> {
        match t {
            TypeRef::Named(name, pos) => {
                if let Some(ty) = Ty::elementary(name) {
                    return Ok(ty);
                }
                if let Some(k) = FbKind::from_name(name) {
                    return Ok(Ty::Fb(FbType::Std(k)));
                }
                let key = name.to_ascii_uppercase();
                match self.defs.get(&key) {
                    Some((pou, _)) if pou.kind == PouKind::FunctionBlock => Ok(Ty::Fb(FbType::User(key))),
                    Some(_) => Err(self.error(*pos, format!("« {name} » est une FUNCTION, pas un type"))),
                    None => {
                        let hint = match key.as_str() {
                            "LREAL" => " (utilisez REAL)",
                            "WORD" | "DWORD" | "BYTE" => " (utilisez INT ou DINT)",
                            "SINT" | "USINT" | "UINT" | "UDINT" | "LINT" => " (utilisez INT ou DINT)",
                            "STRING" => " (les chaînes ne sont pas supportées)",
                            _ => "",
                        };
                        Err(self.error(*pos, format!("type inconnu « {name} »{hint}")))
                    }
                }
            }
            TypeRef::Array { lo, hi, elem } => {
                let pos = lo.pos();
                let (Some(ConstVal::Int(l)), Some(ConstVal::Int(h))) = (const_eval(lo, consts), const_eval(hi, consts))
                else {
                    return Err(self.error(pos, "les bornes d'un ARRAY doivent être des entiers constants"));
                };
                let len = h - l + 1;
                if !(1..=4096).contains(&len) || i32::try_from(l).is_err() {
                    return Err(self.error(pos, "taille d'ARRAY invalide (1 à 4096 éléments)"));
                }
                let elem = self.resolve_type(elem, consts)?;
                if !elem.is_scalar() {
                    return Err(self.error(pos, "seuls les ARRAY de types élémentaires sont supportés"));
                }
                Ok(Ty::Array { elem: Box::new(elem), lo: l as i32, len: len as u16 })
            }
        }
    }

    fn pou_consts(&mut self, pou: &Pou) -> R<HashMap<String, ConstVal>> {
        let mut consts = HashMap::new();
        for v in pou.vars.iter().filter(|v| v.section == VarSection::Constant) {
            let ty = self.resolve_type(&v.ty, &consts)?;
            let Some(init) = &v.init else {
                return Err(self.error(v.pos, format!("la constante « {} » n'a pas de valeur", v.name)));
            };
            let Some(val) = const_eval(init, &consts) else {
                return Err(self.error(init.pos(), "valeur constante attendue"));
            };
            let val = match (val, &ty) {
                (ConstVal::Int(i), Ty::Real) => ConstVal::Real(i as f64),
                (v, _) => v,
            };
            if let Err(m) = val.bits(&ty) {
                return Err(self.error(v.pos, m));
            }
            consts.insert(v.name.to_ascii_uppercase(), val);
        }
        Ok(consts)
    }

    fn init_bits(&mut self, v: &VarDecl, ty: &Ty, consts: &HashMap<String, ConstVal>) -> R<Option<u32>> {
        let Some(e) = &v.init else { return Ok(None) };
        if !ty.is_scalar() {
            return Err(self.error(e.pos(), "valeur initiale possible seulement sur un type élémentaire"));
        }
        let Some(c) = const_eval(e, consts) else {
            return Err(self.error(e.pos(), "la valeur initiale doit être constante"));
        };
        c.bits(ty).map(Some).map_err(|m| self.error(e.pos(), m))
    }

    /// Disposition mémoire d'un FUNCTION_BLOCK ou d'une FUNCTION utilisateur.
    fn layout(&mut self, key: &str, pos: Pos) -> R<Rc<Layout>> {
        if let Some(l) = self.layouts.get(key) {
            return Ok(l.clone());
        }
        if self.layout_busy.iter().any(|k| k == key) {
            return Err(self.error(pos, format!("« {key} » se contient lui-même")));
        }
        let Some((pou, _)) = self.defs.get(key).cloned() else {
            return Err(self.error(pos, format!("bloc « {key} » introuvable")));
        };
        self.layout_busy.push(key.into());
        let r = self.build_layout(pou);
        self.layout_busy.pop();
        let l = Rc::new(r?);
        self.layouts.insert(key.into(), l.clone());
        Ok(l)
    }

    fn build_layout(&mut self, pou: &Pou) -> R<Layout> {
        let consts = self.pou_consts(pou)?;
        let mut fields: Vec<FieldInfo> = Vec::new();
        let mut by_key = HashMap::new();
        let mut offset = 0u16;
        let mut add = |g: &mut Gen, name: &str, ty: Ty, section, init, pos| -> R<()> {
            let key = name.to_ascii_uppercase();
            if by_key.contains_key(&key) || consts.contains_key(&key) {
                return Err(g.error(pos, format!("« {name} » est déclaré deux fois")));
            }
            let size = g.size_of(&ty, pos)?;
            by_key.insert(key, fields.len());
            fields.push(FieldInfo { name: name.into(), offset, ty, section, init });
            offset = offset.checked_add(size).ok_or_else(|| g.error(pos, "bloc trop gros"))?;
            Ok(())
        };
        for v in pou.vars.iter().filter(|v| v.section != VarSection::Constant) {
            let ty = self.resolve_type(&v.ty, &consts)?;
            if pou.kind == PouKind::Function && matches!(ty, Ty::Fb(_)) {
                return Err(self
                    .error(v.pos, "une FUNCTION ne peut pas contenir d'instance de FB (utilisez un FUNCTION_BLOCK)"));
            }
            let init = self.init_bits(v, &ty, &consts)?;
            add(self, &v.name, ty, v.section, init, v.pos)?;
        }
        if let Some(ret) = &pou.ret {
            let ty = self.resolve_type(ret, &consts)?;
            if !ty.is_scalar() {
                return Err(self.error(pou.pos, "une FUNCTION doit renvoyer un type élémentaire"));
            }
            add(self, &pou.name, ty, VarSection::Output, None, pou.pos)?;
        }
        Ok(Layout { size: offset, fields, by_key, consts })
    }

    fn frame_from_layout(&self, base: u16, layout: &Layout) -> Frame {
        let mut names: HashMap<String, Local> =
            layout.consts.iter().map(|(k, v)| (k.clone(), Local::Const(*v))).collect();
        for f in &layout.fields {
            names.insert(
                f.name.to_ascii_uppercase(),
                Local::Var { slot: base + f.offset, ty: f.ty.clone(), read_only: false },
            );
        }
        Frame { names, returns: Vec::new() }
    }

    // ---------- émission ----------

    fn emit(&mut self, i: Instr) -> u64 {
        self.depth += i.stack_effect();
        self.max_depth = self.max_depth.max(self.depth);
        self.code.push(i);
        cost::cost(&i) as u64
    }

    fn here(&self) -> u16 {
        self.code.len() as u16
    }

    fn patch(&mut self, at: usize, target: u16) {
        self.code[at] = match self.code[at] {
            Instr::Jmp(_) => Instr::Jmp(target),
            Instr::Jz(_) => Instr::Jz(target),
            other => other,
        };
    }

    // ---------- programmes ----------

    /// Compile un PROGRAM / ORGANIZATION_BLOCK et renvoie son WCET en cycles.
    pub fn program(&mut self, pou: &Pou, block: &str) -> R<u64> {
        self.set_block(block);
        let consts = self.pou_consts(pou)?;
        let mut names: HashMap<String, Local> = consts.iter().map(|(k, v)| (k.clone(), Local::Const(*v))).collect();
        let mut failed = false;
        for v in pou.vars.iter().filter(|v| v.section != VarSection::Constant) {
            if matches!(v.section, VarSection::Input | VarSection::Output) {
                self.warn(v.pos, "VAR_INPUT/VAR_OUTPUT dans un programme : traité comme VAR");
            }
            let key = v.name.to_ascii_uppercase();
            if names.contains_key(&key) {
                self.error(v.pos, format!("« {} » est déclaré deux fois", v.name));
                failed = true;
                continue;
            }
            let r = (|| {
                let ty = self.resolve_type(&v.ty, &consts)?;
                let init = self.init_bits(v, &ty, &consts)?;
                let slot = self.alloc_var(&ty, init, &format!("{}.{}", pou.name, v.name), block, v.pos)?;
                Ok((ty, slot))
            })();
            match r {
                Ok((ty, slot)) => {
                    names.insert(key, Local::Var { slot, ty, read_only: false });
                }
                Err(Reported) => failed = true,
            }
        }
        if failed {
            return Err(Reported);
        }
        self.frames.push(Frame { names, returns: Vec::new() });
        let r = self.stmts(&pou.body);
        let frame = self.frames.pop().unwrap();
        let end = self.here();
        for at in frame.returns {
            self.patch(at, end);
        }
        r
    }

    fn stmts(&mut self, list: &[Stmt]) -> R<u64> {
        let mut total = 0;
        let mut failed = false;
        for s in list {
            match self.stmt(s) {
                Ok(c) => total += c,
                Err(Reported) => failed = true,
            }
        }
        if failed { Err(Reported) } else { Ok(total) }
    }

    fn stmt(&mut self, s: &Stmt) -> R<u64> {
        match s {
            Stmt::Empty => Ok(0),
            Stmt::Assign { target, value, pos } => self.assign(target, value, *pos),
            Stmt::If { branches, otherwise, .. } => {
                let mut cond_cost = 0;
                let mut worst_branch = 0u64;
                let mut to_end = Vec::new();
                let mut failed = false;
                for (cond, body) in branches {
                    match self.expr_as(cond, &Ty::Bool) {
                        Ok(c) => cond_cost += c,
                        Err(_) => failed = true,
                    }
                    let jz = self.code.len();
                    cond_cost += self.emit(Instr::Jz(0));
                    let mut b = self.stmts(body).unwrap_or_else(|_| {
                        failed = true;
                        0
                    });
                    to_end.push(self.code.len());
                    b += self.emit(Instr::Jmp(0));
                    worst_branch = worst_branch.max(b);
                    let next = self.here();
                    self.patch(jz, next);
                }
                let e = self.stmts(otherwise).unwrap_or_else(|_| {
                    failed = true;
                    0
                });
                worst_branch = worst_branch.max(e);
                let end = self.here();
                for at in to_end {
                    self.patch(at, end);
                }
                if failed { Err(Reported) } else { Ok(cond_cost + worst_branch) }
            }
            Stmt::Case { selector, arms, otherwise, pos } => self.case(selector, arms, otherwise, *pos),
            Stmt::For { var, from, to, by, body, pos } => self.for_loop(var, from, to, by.as_ref(), body, *pos),
            Stmt::Call { callee, args, pos } => {
                let Expr::Name { name, scope, .. } = callee else { unreachable!() };
                self.call_stmt(name, *scope, args, *pos)
            }
            Stmt::Return { .. } => {
                let at = self.code.len();
                let c = self.emit(Instr::Jmp(0));
                self.frames.last_mut().unwrap().returns.push(at);
                Ok(c)
            }
            Stmt::Unbounded { keyword, pos } => Err(self.error(
                *pos,
                format!(
                    "{keyword} est interdit : son nombre de tours dépend des données, le temps de cycle ne serait plus garanti. \
                     Utilisez FOR avec des bornes constantes (et IF pour sortir de la logique)."
                ),
            )),
        }
    }

    fn assign(&mut self, target: &Expr, value: &Expr, pos: Pos) -> R<u64> {
        let place = self.place(target)?;
        match place {
            Place::Const(_) => Err(self.error(pos, "impossible d'écrire dans une constante")),
            Place::Slot { slot, ty, read_only } => {
                if read_only {
                    return Err(self.error(pos, "variable système en lecture seule"));
                }
                if self.loop_slots.contains(&slot) {
                    return Err(
                        self.error(pos, "la variable d'une boucle FOR ne peut pas être modifiée dans la boucle")
                    );
                }
                if !ty.is_scalar() {
                    return Err(self.error(pos, format!("affectation impossible sur un {}", ty.name())));
                }
                let c = self.expr_as(value, &ty)?;
                Ok(c + self.emit(Instr::Store(slot)))
            }
            Place::Indexed { base, len, lo, elem, index } => {
                let mut c = self.index(index, lo)?;
                c += self.expr_as(value, &elem)?;
                Ok(c + self.emit(Instr::StoreIdx { base, len }))
            }
        }
    }

    fn case(&mut self, selector: &Expr, arms: &[CaseArm], otherwise: &[Stmt], pos: Pos) -> R<u64> {
        let sel_ty = self.infer(selector)?;
        if !sel_ty.is_int() {
            return Err(self.error(pos, "le sélecteur de CASE doit être un entier"));
        }
        // Le sélecteur est évalué une seule fois dans un slot temporaire.
        let tmp = self.alloc_raw(1, pos)?;
        let mut c = self.expr_as(selector, &Ty::DInt)?;
        c += self.emit(Instr::Store(tmp));
        let consts = self.current_consts();
        let mut seen: Vec<(i64, i64)> = Vec::new();
        let mut tests = 0u64;
        let mut worst = 0u64;
        let mut to_end = Vec::new();
        let mut failed = false;
        for arm in arms {
            for (n, label) in arm.labels.iter().enumerate() {
                let (lo_e, hi_e) = match label {
                    CaseLabel::Value(v) => (v, v),
                    CaseLabel::Range(a, b) => (a, b),
                };
                let (Some(ConstVal::Int(lo)), Some(ConstVal::Int(hi))) =
                    (const_eval(lo_e, &consts), const_eval(hi_e, &consts))
                else {
                    failed = true;
                    self.error(lo_e.pos(), "les libellés de CASE doivent être des entiers constants");
                    continue;
                };
                if seen.iter().any(|&(a, b)| lo <= b && a <= hi) {
                    self.error(lo_e.pos(), "libellé de CASE en double");
                    failed = true;
                }
                seen.push((lo, hi));
                let lo32 = lo as i32;
                let hi32 = hi as i32;
                if lo == hi {
                    tests += self.emit(Instr::Load(tmp)) + self.emit(Instr::PushI(lo32)) + self.emit(Instr::EqI);
                } else {
                    tests += self.emit(Instr::Load(tmp)) + self.emit(Instr::PushI(lo32)) + self.emit(Instr::GeI);
                    tests += self.emit(Instr::Load(tmp)) + self.emit(Instr::PushI(hi32)) + self.emit(Instr::LeI);
                    tests += self.emit(Instr::And);
                }
                if n > 0 {
                    tests += self.emit(Instr::Or);
                }
            }
            let jz = self.code.len();
            tests += self.emit(Instr::Jz(0));
            let mut b = self.stmts(&arm.body).unwrap_or_else(|_| {
                failed = true;
                0
            });
            to_end.push(self.code.len());
            b += self.emit(Instr::Jmp(0));
            worst = worst.max(b);
            let next = self.here();
            self.patch(jz, next);
        }
        let e = self.stmts(otherwise).unwrap_or_else(|_| {
            failed = true;
            0
        });
        worst = worst.max(e);
        let end = self.here();
        for at in to_end {
            self.patch(at, end);
        }
        if failed { Err(Reported) } else { Ok(c + tests + worst) }
    }

    fn current_consts(&self) -> HashMap<String, ConstVal> {
        self.frames
            .last()
            .map(|f| {
                f.names
                    .iter()
                    .filter_map(|(k, v)| if let Local::Const(c) = v { Some((k.clone(), *c)) } else { None })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn for_loop(&mut self, var: &str, from: &Expr, to: &Expr, by: Option<&Expr>, body: &[Stmt], pos: Pos) -> R<u64> {
        let name = Expr::Name { name: var.into(), scope: NameScope::Local, pos };
        let (slot, ty) = match self.place(&name) {
            Ok(Place::Slot { slot, ty, read_only: false }) if ty.is_int() => (slot, ty),
            Ok(_) => {
                return Err(self.error(pos, "la variable de boucle doit être une variable locale INT ou DINT"));
            }
            Err(r) => return Err(r),
        };
        let consts = self.current_consts();
        let bound = |e: &Expr| match const_eval(e, &consts) {
            Some(ConstVal::Int(v)) => Some(v),
            _ => None,
        };
        let (Some(f), Some(t)) = (bound(from), bound(to)) else {
            return Err(self.error(
                pos,
                "les bornes de FOR doivent être constantes (littéral ou VAR CONSTANT) : c'est ce qui garantit le temps de cycle",
            ));
        };
        let step = match by {
            None => 1,
            Some(e) => match bound(e) {
                Some(0) => {
                    return Err(self.error(e.pos(), "le pas d'une boucle FOR ne peut pas être 0"));
                }
                Some(s) => s,
                None => return Err(self.error(e.pos(), "le pas de FOR doit être constant")),
            },
        };
        let limit = if ty == Ty::Int { (i16::MIN as i64, i16::MAX as i64) } else { (i32::MIN as i64, i32::MAX as i64) };
        if f < limit.0 || f > limit.1 || t < limit.0 || t > limit.1 {
            return Err(self.error(pos, format!("bornes hors de la plage {}", ty.name())));
        }
        // La dernière incrémentation ne doit pas déborder, sinon la boucle ne finirait pas.
        let iters = if step > 0 {
            if t >= f { (t - f) / step + 1 } else { 0 }
        } else if f >= t {
            (f - t) / -step + 1
        } else {
            0
        };
        let last = f + iters * step;
        if last < limit.0 || last > limit.1 {
            return Err(
                self.error(pos, format!("le compteur de boucle dépasserait la plage {} : utilisez DINT", ty.name()))
            );
        }

        let mut init = self.emit(Instr::PushI(f as i32));
        init += self.emit(Instr::Store(slot));
        let head = self.here();
        let mut cond = self.emit(Instr::Load(slot));
        cond += self.emit(Instr::PushI(t as i32));
        cond += self.emit(if step > 0 { Instr::LeI } else { Instr::GeI });
        let jz = self.code.len();
        cond += self.emit(Instr::Jz(0));
        self.loop_slots.push(slot);
        let body_cost = self.stmts(body);
        self.loop_slots.pop();
        let mut incr = self.emit(Instr::Load(slot));
        incr += self.emit(Instr::PushI(step as i32));
        incr += self.emit(Instr::AddI);
        incr += self.emit(Instr::Store(slot));
        incr += self.emit(Instr::Jmp(head));
        let end = self.here();
        self.patch(jz, end);
        let iters = iters as u64;
        Ok(init + (iters + 1) * cond + iters * (body_cost? + incr))
    }

    // ---------- noms et emplacements ----------

    fn lookup(&self, name: &str, scope: NameScope) -> Option<Local> {
        let key = name.to_ascii_uppercase();
        if scope != NameScope::Global
            && let Some(l) = self.frames.last().and_then(|f| f.names.get(&key))
        {
            return Some(l.clone());
        }
        if scope != NameScope::Local
            && let Some(g) = self.globals.get(&key)
        {
            return Some(Local::Var { slot: g.slot, ty: g.ty.clone(), read_only: g.read_only });
        }
        None
    }

    fn place<'e>(&mut self, e: &'e Expr) -> R<Place<'e>> {
        match e {
            Expr::Name { name, scope, pos } => match self.lookup(name, *scope) {
                Some(Local::Var { slot, ty, read_only }) => Ok(Place::Slot { slot, ty, read_only }),
                Some(Local::Const(c)) => Ok(Place::Const(c)),
                None => {
                    let what = match scope {
                        NameScope::Global => "variable API",
                        _ => "variable",
                    };
                    Err(self.error(*pos, format!("{what} « {name} » inconnue")))
                }
            },
            Expr::Field { base, field, pos } => {
                let Place::Slot { slot, ty, .. } = self.place(base)? else {
                    return Err(self.error(*pos, "accès « . » sur autre chose qu'une instance de FB"));
                };
                match ty {
                    Ty::Fb(FbType::Std(k)) => {
                        match k.fields().iter().position(|f| f.name.eq_ignore_ascii_case(field) && f.dir != Dir::Hidden)
                        {
                            Some(i) => Ok(Place::Slot {
                                slot: slot + i as u16,
                                ty: field_ty(k.fields()[i].ty),
                                read_only: false,
                            }),
                            None => Err(self.error(*pos, format!("{} n'a pas de paramètre « {field} »", k.name()))),
                        }
                    }
                    Ty::Fb(FbType::User(n)) => {
                        let layout = self.layout(&n, *pos)?;
                        match layout.field(field) {
                            Some(f) if matches!(f.section, VarSection::Input | VarSection::Output) => {
                                Ok(Place::Slot { slot: slot + f.offset, ty: f.ty.clone(), read_only: false })
                            }
                            Some(_) => Err(self.error(*pos, format!("« {field} » est interne au bloc {n}"))),
                            None => Err(self.error(*pos, format!("{n} n'a pas de paramètre « {field} »"))),
                        }
                    }
                    other => Err(self.error(*pos, format!("accès « .{field} » impossible sur un {}", other.name()))),
                }
            }
            Expr::Index { base, index, pos } => {
                let Place::Slot { slot, ty: Ty::Array { elem, lo, len }, .. } = self.place(base)? else {
                    return Err(self.error(*pos, "indexation « [ ] » sur autre chose qu'un ARRAY"));
                };
                let consts = self.current_consts();
                if let Some(ConstVal::Int(i)) = const_eval(index, &consts) {
                    let off = i - lo as i64;
                    if off < 0 || off >= len as i64 {
                        return Err(
                            self.error(*pos, format!("index {i} hors des bornes [{lo}..{}]", lo + len as i32 - 1))
                        );
                    }
                    return Ok(Place::Slot { slot: slot + off as u16, ty: *elem, read_only: false });
                }
                Ok(Place::Indexed { base: slot, len, lo, elem: *elem, index })
            }
            other => Err(self.error(other.pos(), "variable attendue")),
        }
    }

    /// Calcule l'index (base 0) d'un accès tableau.
    fn index(&mut self, index: &Expr, lo: i32) -> R<u64> {
        let mut c = self.expr_as(index, &Ty::DInt)?;
        if lo != 0 {
            c += self.emit(Instr::PushI(lo)) + self.emit(Instr::SubI);
        }
        Ok(c)
    }

    // ---------- expressions ----------

    /// Type d'une expression, sans générer de code.
    fn infer(&mut self, e: &Expr) -> R<Ty> {
        match e {
            Expr::Int(v, pos) => {
                if i16::try_from(*v).is_ok() {
                    Ok(Ty::Int)
                } else if i32::try_from(*v).is_ok() {
                    Ok(Ty::DInt)
                } else {
                    Err(self.error(*pos, format!("{v} dépasse la plage DINT")))
                }
            }
            Expr::Real(..) => Ok(Ty::Real),
            Expr::Bool(..) => Ok(Ty::Bool),
            Expr::Time(..) => Ok(Ty::Time),
            Expr::Name { .. } | Expr::Field { .. } | Expr::Index { .. } => Ok(match self.place(e)? {
                Place::Slot { ty, .. } => ty,
                Place::Indexed { elem, .. } => elem,
                Place::Const(c) => c.ty(),
            }),
            Expr::Unary { op: UnOp::Not, e, pos } => {
                let t = self.infer(e)?;
                if t != Ty::Bool {
                    return Err(self.error(*pos, format!("NOT attend un BOOL, pas un {}", t.name())));
                }
                Ok(Ty::Bool)
            }
            Expr::Unary { op: UnOp::Neg, e, pos } => {
                let t = self.infer(e)?;
                if !t.is_num() && t != Ty::Time {
                    return Err(self.error(*pos, format!("signe « - » impossible sur un {}", t.name())));
                }
                Ok(t)
            }
            Expr::Binary { op, l, r, pos } => {
                let (lt, rt) = (self.infer(l)?, self.infer(r)?);
                binary_types(*op, &lt, &rt).map(|(_, res)| res).map_err(|m| self.error(*pos, m))
            }
            Expr::Call { name, args, pos } => self.call_type(name, args, *pos),
        }
    }

    /// Compile `e` et convertit le résultat vers `to` (conversions implicites
    /// sans perte seulement).
    fn expr_as(&mut self, e: &Expr, to: &Ty) -> R<u64> {
        if let Expr::Int(v, pos) = e {
            match to {
                Ty::Real => return Ok(self.emit(Instr::PushF(*v as f32))),
                Ty::Int if i16::try_from(*v).is_err() => {
                    return Err(self.error(*pos, format!("{v} hors de la plage INT (-32768..32767)")));
                }
                _ => {}
            }
        }
        let from = self.infer(e)?;
        let c = self.expr(e)?;
        Ok(c + self.convert(&from, to, e.pos())?)
    }

    fn convert(&mut self, from: &Ty, to: &Ty, pos: Pos) -> R<u64> {
        if from == to {
            return Ok(0);
        }
        match (from, to) {
            (Ty::Int, Ty::DInt) => Ok(0),
            (Ty::Int | Ty::DInt, Ty::Real) => Ok(self.emit(Instr::I2F)),
            _ => {
                let hint = match (from, to) {
                    (Ty::Real, Ty::Int | Ty::DInt) => {
                        format!(" : utilisez REAL_TO_{}(…) ou TRUNC(…)", to.name())
                    }
                    (Ty::DInt, Ty::Int) => " : utilisez DINT_TO_INT(…)".into(),
                    (Ty::Time, Ty::Int | Ty::DInt) | (Ty::Int | Ty::DInt, Ty::Time) => {
                        format!(" : utilisez {}_TO_{}(…)", from.name(), to.name())
                    }
                    _ => String::new(),
                };
                Err(self.error(pos, format!("{} attendu, mais l'expression est un {}{hint}", to.name(), from.name())))
            }
        }
    }

    /// Compile une expression ; elle laisse une valeur sur la pile.
    fn expr(&mut self, e: &Expr) -> R<u64> {
        match e {
            Expr::Int(v, _) => Ok(self.emit(Instr::PushI(*v as i32))),
            Expr::Real(v, _) => Ok(self.emit(Instr::PushF(*v as f32))),
            Expr::Bool(v, _) => Ok(self.emit(Instr::PushI(*v as i32))),
            Expr::Time(v, pos) => {
                let v = i32::try_from(*v).map_err(|_| self.error(*pos, "durée trop longue (24 jours max)"))?;
                Ok(self.emit(Instr::PushI(v)))
            }
            Expr::Name { .. } | Expr::Field { .. } | Expr::Index { .. } => match self.place(e)? {
                Place::Slot { slot, ty, .. } => {
                    if !ty.is_scalar() {
                        return Err(
                            self.error(e.pos(), format!("un {} ne peut pas être utilisé comme valeur", ty.name()))
                        );
                    }
                    Ok(self.emit(Instr::Load(slot)))
                }
                Place::Indexed { base, len, lo, index, .. } => {
                    let c = self.index(index, lo)?;
                    Ok(c + self.emit(Instr::LoadIdx { base, len }))
                }
                Place::Const(c) => Ok(match c {
                    ConstVal::Int(v) | ConstVal::Time(v) => self.emit(Instr::PushI(v as i32)),
                    ConstVal::Real(v) => self.emit(Instr::PushF(v as f32)),
                    ConstVal::Bool(v) => self.emit(Instr::PushI(v as i32)),
                }),
            },
            Expr::Unary { op, e: inner, .. } => {
                let t = self.infer(e)?;
                let mut c = self.expr(inner)?;
                c += match (op, &t) {
                    (UnOp::Not, _) => self.emit(Instr::Not),
                    (UnOp::Neg, Ty::Real) => self.emit(Instr::NegF),
                    (UnOp::Neg, Ty::Int) => self.emit(Instr::NegI) + self.emit(Instr::WrapI16),
                    (UnOp::Neg, _) => self.emit(Instr::NegI),
                };
                Ok(c)
            }
            Expr::Binary { op, l, r, pos } => {
                let (lt, rt) = (self.infer(l)?, self.infer(r)?);
                let (operand, result) = binary_types(*op, &lt, &rt).map_err(|m| self.error(*pos, m))?;
                let mut c;
                if lt == Ty::Time && matches!(op, BinOp::Mul | BinOp::Div) {
                    // TIME * entier : calcul entier sur les millisecondes
                    c = self.expr(l)?;
                    c += self.expr_as(r, &Ty::DInt)?;
                } else {
                    c = self.expr_as(l, &operand)?;
                    c += self.expr_as(r, &operand)?;
                }
                c += self.emit(bin_instr(*op, &operand));
                if result == Ty::Int && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
                    c += self.emit(Instr::WrapI16);
                }
                Ok(c)
            }
            Expr::Call { name, args, pos } => {
                let c = self.call_expr(name, args, *pos)?;
                Ok(c)
            }
        }
    }

    // ---------- appels ----------

    fn call_type(&mut self, name: &str, args: &[Arg], pos: Pos) -> R<Ty> {
        let key = name.to_ascii_uppercase();
        if let Some((_, to)) = conversion(&key) {
            return Ok(to);
        }
        match key.as_str() {
            "TRUNC" | "ROUND" => return Ok(Ty::DInt),
            "NORM_X" | "SCALE_X" => return Ok(Ty::Real),
            "ABS" | "MIN" | "MAX" | "LIMIT" | "SEL" => {
                let vals = self.builtin_args(&key, args, pos)?;
                let skip = usize::from(key == "SEL");
                let mut t = self.infer(vals[skip])?;
                for v in &vals[skip + 1..] {
                    let vt = self.infer(v)?;
                    t = promote(&t, &vt).ok_or_else(|| {
                        self.error(pos, format!("{key} : types {} et {} incompatibles", t.name(), vt.name()))
                    })?;
                }
                return Ok(t);
            }
            _ => {}
        }
        match self.defs.get(&key) {
            Some((pou, _)) if pou.kind == PouKind::Function => {
                let layout = self.layout(&key, pos)?;
                match layout.field(name) {
                    Some(f) if f.section == VarSection::Output && f.name.eq_ignore_ascii_case(name) => Ok(f.ty.clone()),
                    _ => Err(self.error(pos, format!("la FUNCTION « {name} » ne renvoie pas de valeur"))),
                }
            }
            Some(_) => Err(self.error(pos, format!("« {name} » est un FUNCTION_BLOCK : appelez une instance"))),
            None => Err(self.error(pos, format!("fonction « {name} » inconnue"))),
        }
    }

    fn builtin_args<'e>(&mut self, key: &str, args: &'e [Arg], pos: Pos) -> R<Vec<&'e Expr>> {
        let params: &[&str] = match key {
            "ABS" | "TRUNC" | "ROUND" => &["IN"],
            "LIMIT" => &["MN", "IN", "MX"],
            "SEL" => &["G", "IN0", "IN1"],
            "NORM_X" | "SCALE_X" => &["MIN", "VALUE", "MAX"],
            "MIN" | "MAX" => {
                if args.len() < 2 || args.iter().any(|a| a.output) {
                    return Err(self.error(pos, format!("{key} attend au moins 2 valeurs")));
                }
                return Ok(args.iter().map(|a| &a.value).collect());
            }
            _ => &["IN"],
        };
        positional(args, params).map_err(|m| self.error(pos, format!("{key} : {m}")))
    }

    fn call_expr(&mut self, name: &str, args: &[Arg], pos: Pos) -> R<u64> {
        let key = name.to_ascii_uppercase();
        if let Some((from, to)) = conversion(&key) {
            let vals = self.builtin_args(&key, args, pos)?;
            let c = self.expr_as(vals[0], &from)?;
            let mut extra = 0;
            for i in conversion_instrs(&from, &to) {
                extra += self.emit(*i);
            }
            return Ok(c + extra);
        }
        match key.as_str() {
            "TRUNC" | "ROUND" => {
                let vals = self.builtin_args(&key, args, pos)?;
                let c = self.expr_as(vals[0], &Ty::Real)?;
                return Ok(c + self.emit(if key == "TRUNC" { Instr::F2ITrunc } else { Instr::F2IRound }));
            }
            "ABS" => {
                let t = self.call_type(name, args, pos)?;
                let vals = self.builtin_args(&key, args, pos)?;
                let mut c = self.expr_as(vals[0], &t)?;
                c += match t {
                    Ty::Real => self.emit(Instr::AbsF),
                    Ty::Int => self.emit(Instr::AbsI) + self.emit(Instr::WrapI16),
                    Ty::DInt => self.emit(Instr::AbsI),
                    other => {
                        return Err(self.error(pos, format!("ABS impossible sur un {}", other.name())));
                    }
                };
                return Ok(c);
            }
            "MIN" | "MAX" | "LIMIT" => {
                let t = self.call_type(name, args, pos)?;
                if !t.is_num() && t != Ty::Time {
                    return Err(self.error(pos, format!("{key} impossible sur un {}", t.name())));
                }
                let (min_i, max_i) =
                    if t == Ty::Real { (Instr::MinF, Instr::MaxF) } else { (Instr::MinI, Instr::MaxI) };
                let vals = self.builtin_args(&key, args, pos)?;
                let mut c;
                if key == "LIMIT" {
                    // LIMIT(MN, IN, MX) = MIN(MAX(IN, MN), MX)
                    c = self.expr_as(vals[1], &t)?;
                    c += self.expr_as(vals[0], &t)?;
                    c += self.emit(max_i);
                    c += self.expr_as(vals[2], &t)?;
                    c += self.emit(min_i);
                } else {
                    let op = if key == "MIN" { min_i } else { max_i };
                    c = self.expr_as(vals[0], &t)?;
                    for v in &vals[1..] {
                        c += self.expr_as(v, &t)?;
                        c += self.emit(op);
                    }
                }
                return Ok(c);
            }
            "SEL" => {
                let t = self.call_type(name, args, pos)?;
                let vals = self.builtin_args(&key, args, pos)?;
                let mut c = self.expr_as(vals[0], &Ty::Bool)?;
                let jz = self.code.len();
                c += self.emit(Instr::Jz(0));
                let depth = self.depth;
                let in1 = self.expr_as(vals[2], &t)?;
                let jmp = self.code.len();
                let in1 = in1 + self.emit(Instr::Jmp(0));
                self.depth = depth;
                let else_at = self.here();
                self.patch(jz, else_at);
                let in0 = self.expr_as(vals[1], &t)?;
                let end = self.here();
                self.patch(jmp, end);
                return Ok(c + in1.max(in0));
            }
            "NORM_X" | "SCALE_X" => {
                // NORM_X = (VALUE - MIN) / (MAX - MIN) ; SCALE_X = VALUE * (MAX - MIN) + MIN
                let vals = self.builtin_args(&key, args, pos)?;
                let mut c = 0;
                if key == "NORM_X" {
                    c += self.expr_as(vals[1], &Ty::Real)?;
                    c += self.expr_as(vals[0], &Ty::Real)?;
                    c += self.emit(Instr::SubF);
                    c += self.expr_as(vals[2], &Ty::Real)?;
                    c += self.expr_as(vals[0], &Ty::Real)?;
                    c += self.emit(Instr::SubF);
                    c += self.emit(Instr::DivF);
                } else {
                    c += self.expr_as(vals[1], &Ty::Real)?;
                    c += self.expr_as(vals[2], &Ty::Real)?;
                    c += self.expr_as(vals[0], &Ty::Real)?;
                    c += self.emit(Instr::SubF);
                    c += self.emit(Instr::MulF);
                    c += self.expr_as(vals[0], &Ty::Real)?;
                    c += self.emit(Instr::AddF);
                }
                return Ok(c);
            }
            _ => {}
        }
        self.call_type(name, args, pos)?;
        self.call_function(&key, args, pos, true)
    }

    fn call_stmt(&mut self, name: &str, scope: NameScope, args: &[Arg], pos: Pos) -> R<u64> {
        let key = name.to_ascii_uppercase();
        if scope != NameScope::Global
            && self.lookup(name, scope).is_none()
            && let Some((pou, _)) = self.defs.get(&key)
        {
            if pou.kind == PouKind::Function {
                return self.call_function(&key, args, pos, false);
            }
            return Err(self.error(
                pos,
                format!("« {name} » est un type de FB : déclarez une instance (VAR mon_{name} : {name}; END_VAR)"),
            ));
        }
        let inst = Expr::Name { name: name.into(), scope, pos };
        let Place::Slot { slot: base, ty, .. } = self.place(&inst)? else {
            return Err(self.error(pos, format!("« {name} » n'est pas une instance de FB")));
        };
        match ty {
            Ty::Fb(FbType::Std(kind)) => {
                let mut c = 0;
                let mut outs = Vec::new();
                for a in args {
                    let Some(pname) = &a.name else {
                        return Err(
                            self.error(a.pos, format!("{} : paramètres nommés attendus (IN := …)", kind.name()))
                        );
                    };
                    let Some(i) =
                        kind.fields().iter().position(|f| f.name.eq_ignore_ascii_case(pname) && f.dir != Dir::Hidden)
                    else {
                        return Err(self.error(a.pos, format!("{} n'a pas de paramètre « {pname} »", kind.name())));
                    };
                    let f = kind.fields()[i];
                    match (f.dir, a.output) {
                        (Dir::In, false) => {
                            c += self.expr_as(&a.value, &field_ty(f.ty))?;
                            c += self.emit(Instr::Store(base + i as u16));
                        }
                        (Dir::Out, true) => outs.push((base + i as u16, field_ty(f.ty), &a.value)),
                        (Dir::In, true) => {
                            return Err(self.error(a.pos, format!("« {pname} » est une entrée : utilisez :=")));
                        }
                        _ => {
                            return Err(self.error(a.pos, format!("« {pname} » est une sortie : utilisez =>")));
                        }
                    }
                }
                c += self.emit(Instr::Fb { kind, base });
                for (slot, ty, target) in outs {
                    c += self.copy_out(slot, &ty, target)?;
                }
                Ok(c)
            }
            Ty::Fb(FbType::User(fb)) => {
                if self.inline_stack.contains(&fb) {
                    return Err(self.error(pos, format!("appel récursif de {fb} : interdit (pile non bornée)")));
                }
                let layout = self.layout(&fb, pos)?;
                let mut c = 0;
                let mut outs = Vec::new();
                let inputs: Vec<&FieldInfo> = layout.fields.iter().filter(|f| f.section == VarSection::Input).collect();
                for (n, a) in args.iter().enumerate() {
                    let field = match &a.name {
                        Some(p) => layout.field(p),
                        None => inputs.get(n).copied(),
                    };
                    let Some(f) = field else {
                        return Err(self.error(a.pos, format!("paramètre inconnu pour {fb}")));
                    };
                    match (f.section, a.output) {
                        (VarSection::Input, false) => {
                            c += self.expr_as(&a.value, &f.ty)?;
                            c += self.emit(Instr::Store(base + f.offset));
                        }
                        (VarSection::Output, true) => outs.push((base + f.offset, f.ty.clone(), &a.value)),
                        _ => {
                            return Err(self.error(a.pos, format!("« {} » ne peut pas être passé ainsi", f.name)));
                        }
                    }
                }
                c += self.inline_call(&fb, base, &layout, pos)?;
                for (slot, ty, target) in outs {
                    c += self.copy_out(slot, &ty, target)?;
                }
                Ok(c)
            }
            other => Err(self.error(pos, format!("« {name} » est un {}, pas une instance de FB", other.name()))),
        }
    }

    fn copy_out(&mut self, slot: u16, ty: &Ty, target: &Expr) -> R<u64> {
        match self.place(target)? {
            Place::Slot { slot: dst, ty: dty, read_only: false } if dty.is_scalar() => {
                let c = self.emit(Instr::Load(slot)) + self.convert(ty, &dty, target.pos())?;
                Ok(c + self.emit(Instr::Store(dst)))
            }
            Place::Indexed { base, len, lo, elem, index } => {
                let mut c = self.index(index, lo)?;
                c += self.emit(Instr::Load(slot)) + self.convert(ty, &elem, target.pos())?;
                Ok(c + self.emit(Instr::StoreIdx { base, len }))
            }
            _ => Err(self.error(target.pos(), "une sortie doit être reliée à une variable modifiable")),
        }
    }

    /// Développe le corps d'un FB ou d'une FUNCTION utilisateur.
    fn inline(&mut self, key: &str, base: u16, layout: &Layout) -> R<u64> {
        let (pou, block) = self.defs.get(key).cloned().expect("défini");
        let saved_block = std::mem::replace(&mut self.block, block);
        let saved_loops = std::mem::take(&mut self.loop_slots);
        self.inline_stack.push(key.into());
        self.frames.push(self.frame_from_layout(base, layout));
        let r = self.stmts(&pou.body);
        let frame = self.frames.pop().unwrap();
        self.inline_stack.pop();
        self.loop_slots = saved_loops;
        self.block = saved_block;
        let end = self.here();
        for at in frame.returns {
            self.patch(at, end);
        }
        self.inlined.insert(key.into());
        r
    }

    /// Développe un bloc appelé et signale l'appel si le bloc a des erreurs.
    fn inline_call(&mut self, key: &str, base: u16, layout: &Layout, call_pos: Pos) -> R<u64> {
        self.inline(key, base, layout).map_err(|_| {
            let name = self.defs[key].0.name.clone();
            self.error(call_pos, format!("le bloc {name} appelé ici contient des erreurs"))
        })
    }

    fn call_function(&mut self, key: &str, args: &[Arg], pos: Pos, want_value: bool) -> R<u64> {
        if self.inline_stack.iter().any(|k| k == key) {
            return Err(self.error(pos, format!("appel récursif de {key} : interdit (pile non bornée)")));
        }
        let (base, layout) = match self.fn_frames.get(key) {
            Some(f) => f.clone(),
            None => {
                let layout = self.layout(key, pos)?;
                let base = self.alloc_raw(layout.size as u32, pos)?;
                let block = self.defs[key].1.clone();
                for f in &layout.fields {
                    let path = format!("{}.{}", self.defs[key].0.name, f.name);
                    self.place_var(base + f.offset, &f.ty, None, &path, &block, pos)?;
                }
                self.fn_frames.insert(key.into(), (base, layout.clone()));
                (base, layout)
            }
        };
        let inputs: Vec<FieldInfo> = layout.fields.iter().filter(|f| f.section == VarSection::Input).cloned().collect();
        let mut given: Vec<Option<&Expr>> = vec![None; inputs.len()];
        let mut outs = Vec::new();
        for (n, a) in args.iter().enumerate() {
            if a.output {
                let f = a.name.as_ref().and_then(|p| layout.field(p)).filter(|f| f.section == VarSection::Output);
                let Some(f) = f else {
                    return Err(self.error(a.pos, "sortie inconnue"));
                };
                outs.push((base + f.offset, f.ty.clone(), &a.value));
                continue;
            }
            let idx = match &a.name {
                Some(p) => inputs.iter().position(|f| f.name.eq_ignore_ascii_case(p)),
                None => Some(n).filter(|n| *n < inputs.len()),
            };
            let Some(i) = idx else {
                return Err(self.error(a.pos, format!("paramètre inconnu pour {key}")));
            };
            given[i] = Some(&a.value);
        }
        // Tous les arguments sont évalués avant d'écrire les paramètres : un
        // appel imbriqué de la même FUNCTION ne peut donc pas les écraser.
        let mut c = 0;
        for (f, g) in inputs.iter().zip(&given) {
            c += match g {
                Some(e) => self.expr_as(e, &f.ty)?,
                None => self.emit(Instr::PushI(f.init.unwrap_or(0) as i32)),
            };
        }
        for f in inputs.iter().rev() {
            c += self.emit(Instr::Store(base + f.offset));
        }
        // Variables locales remises à leur valeur initiale à chaque appel.
        for f in layout.fields.iter().filter(|f| f.section != VarSection::Input) {
            let size = self.size_of(&f.ty, pos)?;
            for k in 0..size {
                c += self.emit(Instr::PushI(if k == 0 { f.init.unwrap_or(0) as i32 } else { 0 }));
                c += self.emit(Instr::Store(base + f.offset + k));
            }
        }
        c += self.inline_call(key, base, &layout, pos)?;
        for (slot, ty, target) in outs {
            c += self.copy_out(slot, &ty, target)?;
        }
        if want_value {
            let name = &self.defs[key].0.name;
            let ret = layout.field(name).expect("valeur de retour");
            c += self.emit(Instr::Load(base + ret.offset));
        }
        Ok(c)
    }
}

impl Gen<'_> {
    /// Vérifie les FB et FUNCTION jamais appelés : leur code est compilé sur
    /// une instance fictive puis jeté, pour signaler quand même leurs erreurs.
    pub fn check_unused(&mut self) {
        let mut keys: Vec<String> = self.defs.keys().filter(|k| !self.inlined.contains(*k)).cloned().collect();
        keys.sort();
        for key in keys {
            let (code, slot, syms, init, depth) =
                (self.code.len(), self.next_slot, self.symbols.len(), self.init.len(), self.max_depth);
            let (pou, block) = self.defs[&key].clone();
            self.set_block(&block);
            if let Ok(layout) = self.layout(&key, pou.pos)
                && let Ok(base) = self.alloc_raw(layout.size as u32, pou.pos)
            {
                let _ = self.inline(&key, base, &layout);
            }
            self.code.truncate(code);
            self.next_slot = slot;
            self.symbols.truncate(syms);
            self.init.truncate(init);
            self.max_depth = depth;
            self.depth = 0;
        }
    }
}

impl Default for Gen<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn field_ty(t: FieldTy) -> Ty {
    match t {
        FieldTy::Bool => Ty::Bool,
        FieldTy::Int => Ty::Int,
        FieldTy::Time => Ty::Time,
    }
}

fn promote(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a, b) {
        _ if a == b && a.is_scalar() => Some(a.clone()),
        (Ty::Real, t) | (t, Ty::Real) if t.is_num() => Some(Ty::Real),
        (Ty::DInt, Ty::Int) | (Ty::Int, Ty::DInt) => Some(Ty::DInt),
        _ => None,
    }
}

/// Types d'un opérateur binaire : `(type des opérandes, type du résultat)`.
fn binary_types(op: BinOp, l: &Ty, r: &Ty) -> Result<(Ty, Ty), String> {
    use BinOp::*;
    let incompatible = || format!("opération impossible entre {} et {}", l.name(), r.name());
    match op {
        And | Or | Xor => {
            if *l == Ty::Bool && *r == Ty::Bool {
                Ok((Ty::Bool, Ty::Bool))
            } else {
                Err(format!("AND, OR et XOR attendent des BOOL (ici {} et {})", l.name(), r.name()))
            }
        }
        Eq | Ne | Lt | Le | Gt | Ge => {
            if *l == Ty::Bool && *r == Ty::Bool && matches!(op, Eq | Ne) {
                return Ok((Ty::Bool, Ty::Bool));
            }
            if *l == Ty::Time && *r == Ty::Time {
                return Ok((Ty::Time, Ty::Bool));
            }
            match promote(l, r) {
                Some(t) if t.is_num() => Ok((t, Ty::Bool)),
                _ => Err(incompatible()),
            }
        }
        Add | Sub => {
            if *l == Ty::Time && *r == Ty::Time {
                return Ok((Ty::Time, Ty::Time));
            }
            match promote(l, r) {
                Some(t) if t.is_num() => Ok((t.clone(), t)),
                _ => Err(incompatible()),
            }
        }
        Mul | Div => {
            if *l == Ty::Time && r.is_int() {
                return Ok((Ty::DInt, Ty::Time));
            }
            match promote(l, r) {
                Some(t) if t.is_num() => Ok((t.clone(), t)),
                _ => Err(incompatible()),
            }
        }
        Mod => match promote(l, r) {
            Some(t) if t.is_int() => Ok((t.clone(), t)),
            _ => Err("MOD attend des entiers".into()),
        },
    }
}

fn bin_instr(op: BinOp, t: &Ty) -> Instr {
    use BinOp::*;
    let real = *t == Ty::Real;
    match op {
        Or => Instr::Or,
        Xor => Instr::Xor,
        And => Instr::And,
        Eq => {
            if real {
                Instr::EqF
            } else {
                Instr::EqI
            }
        }
        Ne => {
            if real {
                Instr::NeF
            } else {
                Instr::NeI
            }
        }
        Lt => {
            if real {
                Instr::LtF
            } else {
                Instr::LtI
            }
        }
        Le => {
            if real {
                Instr::LeF
            } else {
                Instr::LeI
            }
        }
        Gt => {
            if real {
                Instr::GtF
            } else {
                Instr::GtI
            }
        }
        Ge => {
            if real {
                Instr::GeF
            } else {
                Instr::GeI
            }
        }
        Add => {
            if real {
                Instr::AddF
            } else {
                Instr::AddI
            }
        }
        Sub => {
            if real {
                Instr::SubF
            } else {
                Instr::SubI
            }
        }
        Mul => {
            if real {
                Instr::MulF
            } else {
                Instr::MulI
            }
        }
        Div => {
            if real {
                Instr::DivF
            } else {
                Instr::DivI
            }
        }
        Mod => Instr::ModI,
    }
}

const BUILTIN_FNS: &[&str] = &["ABS", "MIN", "MAX", "LIMIT", "SEL", "TRUNC", "ROUND", "NORM_X", "SCALE_X"];

pub fn is_builtin_fn(key: &str) -> bool {
    BUILTIN_FNS.contains(&key) || conversion(key).is_some()
}

/// `INT_TO_REAL` → `(INT, REAL)`.
fn conversion(key: &str) -> Option<(Ty, Ty)> {
    let (a, b) = key.split_once("_TO_")?;
    let (from, to) = (Ty::elementary(a)?, Ty::elementary(b)?);
    (from != to).then_some((from, to))
}

fn conversion_instrs(from: &Ty, to: &Ty) -> &'static [Instr] {
    use Ty::*;
    match (from, to) {
        (DInt | Time, Int) => &[Instr::WrapI16],
        (Int | DInt | Time | Bool, Real) => &[Instr::I2F],
        (Real, DInt | Time) => &[Instr::F2IRound],
        (Real, Int) => &[Instr::F2IRound, Instr::WrapI16],
        (Int | DInt | Time | Real, Bool) => &[Instr::ToBool],
        _ => &[],
    }
}

/// Associe les arguments (positionnels ou nommés) aux paramètres attendus.
fn positional<'e>(args: &'e [Arg], params: &[&str]) -> Result<Vec<&'e Expr>, String> {
    if args.iter().any(|a| a.output) {
        return Err("pas de sortie => sur une fonction standard".into());
    }
    if args.iter().all(|a| a.name.is_some()) && !args.is_empty() {
        let mut out = Vec::new();
        for p in params {
            let a = args
                .iter()
                .find(|a| a.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(p)))
                .ok_or_else(|| format!("paramètre {p} manquant"))?;
            out.push(&a.value);
        }
        if args.len() != params.len() {
            return Err("paramètre en trop".into());
        }
        return Ok(out);
    }
    if args.len() != params.len() || args.iter().any(|a| a.name.is_some()) {
        return Err(format!("{} paramètre(s) attendu(s) : {}", params.len(), params.join(", ")));
    }
    Ok(args.iter().map(|a| &a.value).collect())
}
