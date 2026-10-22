//! Compilateur Structured Text de Takt.
//!
//! Entrée : un [`project::Project`]. Sortie : l'image binaire à télécharger
//! dans l'automate, la table des symboles pour la visualisation en ligne, et
//! le WCET prouvé du cycle comparé au temps de cycle configuré.

pub mod ast;
pub mod board;
pub mod codegen;
pub mod lexer;
pub mod parser;
pub mod project;
pub mod types;

use takt_vm::Instr;
use takt_vm::cost;
use takt_vm::exec::STACK_MAX;
use takt_vm::image::{self, Header, IoEntry, IoKind};
use takt_vm::runtime::MAX_FORCE;

use ast::PouKind;
use board::{AddrSize, Address};
pub use lexer::Pos;
use project::Project;
use types::Ty;

/// Capacité du firmware RP2350 (voir `firmware/takt-rp2350`).
pub const TARGET_MAX_CODE: usize = 4096;
pub const TARGET_MAX_MEM: usize = 4096;
pub const TARGET_MAX_IMAGE: usize = 40960;

/// Au-delà de cette part du temps de cycle, le compilateur prévient.
pub const WCET_WARN_RATIO: f64 = 0.8;

/// Nom de bloc utilisé pour les messages hors blocs de programme.
pub const BLOCK_TAGS: &str = "Variables API";
pub const BLOCK_DEVICE: &str = "Appareil";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    pub block: String,
    pub pos: Pos,
    pub severity: Severity,
    pub msg: String,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    /// `"Moteur"` pour une variable API, `Main.t1.ET` pour une variable de bloc.
    pub path: String,
    pub block: Option<String>,
    pub slot: u16,
    pub ty: Ty,
    pub io: Option<IoKind>,
}

impl Symbol {
    /// Nom local dans son bloc (`t1.ET` pour `Main.t1.ET`).
    pub fn local_name(&self) -> &str {
        match &self.block {
            Some(_) => self.path.split_once('.').map(|(_, r)| r).unwrap_or(&self.path),
            None => &self.path,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BlockWcet {
    pub name: String,
    pub cycles: u64,
}

#[derive(Clone, Debug)]
pub struct Wcet {
    pub blocks: Vec<BlockWcet>,
    pub program_cycles: u64,
    pub overhead_cycles: u64,
    pub total_us: u64,
    pub cycle_us: u32,
}

impl Wcet {
    pub fn ratio(&self) -> f64 {
        self.total_us as f64 / self.cycle_us.max(1) as f64
    }
}

#[derive(Clone, Debug)]
pub struct Compiled {
    pub image: Vec<u8>,
    pub crc: u32,
    pub code: Vec<Instr>,
    pub mem_slots: u16,
    pub max_stack: u16,
    pub wcet: Wcet,
    pub symbols: Vec<Symbol>,
    pub io: Vec<IoEntry>,
    pub init: Vec<(u16, u32)>,
}

impl Compiled {
    /// Trouve une variable par son chemin, avec index de tableau éventuel :
    /// `"Moteur"`, `Moteur`, `Main.t1.ET`, `Main.tab[3]`.
    pub fn resolve(&self, path: &str) -> Option<(u16, Ty)> {
        let p = path.trim().replace('"', "");
        let (base, index) = match p.strip_suffix(']').and_then(|s| s.rsplit_once('[')) {
            Some((b, i)) => (b.trim().to_string(), Some(i.trim().parse::<i64>().ok()?)),
            None => (p.clone(), None),
        };
        let sym = self.symbols.iter().find(|s| s.path.eq_ignore_ascii_case(&base))?;
        match (&sym.ty, index) {
            (Ty::Array { elem, lo, len }, Some(i)) => {
                let off = i - *lo as i64;
                (0..*len as i64).contains(&off).then(|| (sym.slot + off as u16, (**elem).clone()))
            }
            (ty, None) if ty.is_scalar() => Some((sym.slot, ty.clone())),
            _ => None,
        }
    }

    pub fn block_symbols<'s>(&'s self, block: &'s str) -> impl Iterator<Item = &'s Symbol> + 's {
        self.symbols.iter().filter(move |s| s.block.as_deref() == Some(block))
    }
}

pub struct Output {
    pub compiled: Option<Compiled>,
    pub diags: Vec<Diag>,
}

impl Output {
    pub fn errors(&self) -> usize {
        self.diags.iter().filter(|d| d.severity == Severity::Error).count()
    }

    pub fn warnings(&self) -> usize {
        self.diags.iter().filter(|d| d.severity == Severity::Warning).count()
    }
}

fn diag(block: &str, line: u32, severity: Severity, msg: impl Into<String>) -> Diag {
    Diag { block: block.into(), pos: Pos { line, col: 1 }, severity, msg: msg.into() }
}

/// Type de bloc d'une source (pour l'icône dans le navigateur de projet).
pub fn block_kind(source: &str) -> Option<PouKind> {
    parser::parse(source).ok().and_then(|p| p.first().map(|p| p.kind))
}

pub fn compile(p: &Project) -> Output {
    let mut diags = Vec::new();
    let Some(board) = board::find(&p.device.board) else {
        diags.push(diag(BLOCK_DEVICE, 0, Severity::Error, format!("carte inconnue « {} »", p.device.board)));
        return Output { compiled: None, diags };
    };
    let cycle_us = p.device.cycle_us;
    if !(100..=1_000_000).contains(&cycle_us) {
        diags.push(diag(BLOCK_DEVICE, 0, Severity::Error, "le temps de cycle doit être entre 100 µs et 1 s"));
    }

    // 1. Analyse syntaxique de tous les blocs
    let mut parsed = Vec::new();
    for b in &p.blocks {
        match parser::parse(&b.source) {
            Ok(pous) => parsed.push((b.name.clone(), pous)),
            Err(e) => diags.push(Diag { block: b.name.clone(), pos: e.pos, severity: Severity::Error, msg: e.msg }),
        }
    }

    let mut g = codegen::Gen::new();

    // 2. Variables API et mémoire image
    let mut io = Vec::new();
    for (row, t) in p.tags.iter().enumerate() {
        let line = row as u32 + 1;
        let pos = Pos { line, col: 1 };
        g.set_block(BLOCK_TAGS);
        if t.name.trim().is_empty() {
            g.error(pos, "variable sans nom");
            continue;
        }
        let ty = match parser::parse_type(&t.ty) {
            Ok(tr) => match g.resolve_type(&tr, &Default::default()) {
                Ok(ty) if ty.is_scalar() || matches!(ty, Ty::Array { .. }) => ty,
                Ok(ty) => {
                    g.error(pos, format!("« {} » : une variable API ne peut pas être un {}", t.name, ty.name()));
                    continue;
                }
                Err(_) => continue,
            },
            Err(e) => {
                g.error(pos, format!("« {} » : type invalide ({})", t.name, e.msg));
                continue;
            }
        };
        if !ty.is_scalar() && !t.address.trim().is_empty() {
            g.error(pos, format!("« {} » : un ARRAY ne peut pas avoir d'adresse", t.name));
            continue;
        }
        let mut io_kind = None;
        let mut channel = 0;
        if !t.address.trim().is_empty() {
            match board::parse_address(&t.address) {
                Ok((addr, size)) => {
                    let ok = match size {
                        AddrSize::Bit => ty == Ty::Bool,
                        AddrSize::Word => ty == Ty::Int,
                        AddrSize::DWord => matches!(ty, Ty::DInt | Ty::Real | Ty::Time),
                    };
                    if !ok {
                        g.error(
                            pos,
                            format!(
                                "« {} » : le type {} ne correspond pas à l'adresse {}",
                                t.name,
                                ty.name(),
                                t.address
                            ),
                        );
                        continue;
                    }
                    if let Address::Io { kind, channel: ch } = addr {
                        if board.pin(kind, ch).is_none() {
                            g.error(pos, format!("« {} » : {} n'existe pas sur {}", t.name, t.address, board.name));
                            continue;
                        }
                        if io.iter().any(|(k, c, _): &(IoKind, u16, u16)| *k == kind && *c == ch) {
                            g.error(pos, format!("« {} » : l'adresse {} est déjà utilisée", t.name, t.address));
                            continue;
                        }
                        io_kind = Some(kind);
                        channel = ch;
                    }
                }
                Err(m) => {
                    g.error(pos, m);
                    continue;
                }
            }
        }
        if let Ok(slot) = g.add_global(&t.name, ty, io_kind, pos)
            && let Some(kind) = io_kind
        {
            io.push((kind, channel, slot));
        }
    }

    // 3. Définitions (FB, FUNCTION), puis programmes dans l'ordre du projet
    for (block, pous) in &parsed {
        for pou in pous {
            if pou.kind != PouKind::Program {
                g.define(pou, block);
            }
        }
    }
    let mut blocks_wcet = Vec::new();
    let mut n_programs = 0;
    for (block, pous) in &parsed {
        for pou in pous.iter().filter(|p| p.kind == PouKind::Program) {
            n_programs += 1;
            if let Ok(cycles) = g.program(pou, block) {
                blocks_wcet.push(BlockWcet { name: pou.name.clone(), cycles });
            }
        }
    }
    g.check_unused();
    let halt = cost::cost(&Instr::Halt) as u64;
    g.code.push(Instr::Halt);
    if n_programs == 0 && !parsed.is_empty() {
        g.set_block(BLOCK_DEVICE);
        g.warn(Pos::default(), "aucun ORGANIZATION_BLOCK / PROGRAM : l'automate ne fera rien");
    }

    diags.append(&mut g.diags);
    diags.dedup();
    let mut seen = Vec::new();
    diags.retain(|d| {
        let dup = seen.contains(d);
        if !dup {
            seen.push(d.clone());
        }
        !dup
    });

    // 4. Preuve du temps de cycle
    let program_cycles: u64 = blocks_wcet.iter().map(|b| b.cycles).sum::<u64>() + halt;
    let io_entries: Vec<IoEntry> =
        io.iter().map(|&(kind, ch, slot)| IoEntry { slot, kind, pin: board.pin(kind, ch).unwrap() }).collect();
    let overhead_cycles = cost::overhead(&io_entries, MAX_FORCE as u32);
    let total_us = cost::cycles_to_us(program_cycles + overhead_cycles);
    let wcet = Wcet { blocks: blocks_wcet, program_cycles, overhead_cycles, total_us, cycle_us };
    let mut fatal = diags.iter().any(|d| d.severity == Severity::Error);
    if !fatal {
        let dev = |sev, msg: String| diag(BLOCK_DEVICE, 0, sev, msg);
        if program_cycles > u32::MAX as u64 {
            diags.push(dev(Severity::Error, "programme beaucoup trop long pour un cycle".into()));
        } else if total_us > cycle_us as u64 {
            diags.push(dev(
                Severity::Error,
                format!(
                    "temps de cycle NON garanti : le pire cas est de {total_us} µs pour un cycle de {cycle_us} µs. \
                     Allongez le temps de cycle ou allégez le programme."
                ),
            ));
        } else if wcet.ratio() > WCET_WARN_RATIO {
            diags.push(dev(
                Severity::Warning,
                format!("le pire cas ({total_us} µs) occupe plus de 80 % du cycle de {cycle_us} µs"),
            ));
        }
        if g.max_depth as usize > STACK_MAX {
            diags.push(dev(
                Severity::Error,
                format!("expressions trop imbriquées (pile {} > {STACK_MAX})", g.max_depth),
            ));
        }
        if g.code.len() > TARGET_MAX_CODE {
            diags.push(dev(
                Severity::Error,
                format!("programme trop long : {} instructions (max {TARGET_MAX_CODE})", g.code.len()),
            ));
        }
        if g.mem_slots() as usize > TARGET_MAX_MEM {
            diags.push(dev(
                Severity::Error,
                format!("trop de variables : {} mots (max {TARGET_MAX_MEM})", g.mem_slots()),
            ));
        }
        fatal = diags.iter().any(|d| d.severity == Severity::Error);
    }
    if fatal {
        return Output { compiled: None, diags };
    }

    // 5. Image binaire
    let header = Header {
        cycle_us,
        mem_slots: g.mem_slots() as u16,
        max_stack: g.max_depth as u16,
        wcet_cycles: program_cycles as u32,
        n_io: io_entries.len() as u16,
        n_init: g.init.len() as u16,
        n_code: g.code.len() as u32,
    };
    let mut bytes = Vec::with_capacity(image::image_len(&header));
    let mut h = [0u8; image::HEADER_SIZE];
    image::write_header(&header, &mut h);
    bytes.extend_from_slice(&h);
    for e in &io_entries {
        bytes.extend_from_slice(&e.encode());
    }
    for &(slot, v) in &g.init {
        bytes.extend_from_slice(&slot.to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for i in &g.code {
        bytes.extend_from_slice(&i.encode());
    }
    let crc = image::crc32(&bytes);
    bytes.extend_from_slice(&crc.to_le_bytes());
    if bytes.len() > TARGET_MAX_IMAGE {
        diags.push(diag(BLOCK_DEVICE, 0, Severity::Error, format!("image trop grosse ({} octets)", bytes.len())));
        return Output { compiled: None, diags };
    }
    diags.push(diag(
        BLOCK_DEVICE,
        0,
        Severity::Info,
        format!(
            "compilation réussie : {} instructions, {} mots mémoire, temps de cycle garanti {} µs ≤ {} µs",
            g.code.len(),
            header.mem_slots,
            total_us,
            cycle_us
        ),
    ));
    let compiled = Compiled {
        image: bytes,
        crc,
        code: g.code,
        mem_slots: header.mem_slots,
        max_stack: header.max_stack,
        wcet,
        symbols: g.symbols,
        io: io_entries,
        init: g.init,
    };
    Output { compiled: Some(compiled), diags }
}
