//! CONT (ladder) : réseaux de contacts, branches, boîtes et bobines.
//!
//! Chaque réseau calcule son RLO (« résultat logique », comme dans TIA) de la
//! gauche vers la droite, puis l'applique aux bobines :
//!
//! ```text
//! ┤ Marche ├─┬─┤/Arret├──( Moteur )      _rlo0 := TRUE;
//!  ┤Moteur ├─┘                            ... branches → _rlo0 := _rlo2 OR _rlo3;
//!                                         _rlo0 := _rlo0 AND NOT Arret;
//!                                         Moteur := _rlo0;
//! ```

use serde::{Deserialize, Serialize};

use super::{Block, Generated, Hidden, Origin, Section};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Network {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub comment: String,
    /// Éléments en série, de la barre d'alimentation vers les bobines.
    #[serde(default)]
    pub rung: Vec<Elem>,
    /// Bobines et affectations, toutes alimentées par le RLO final.
    #[serde(default)]
    pub coils: Vec<Coil>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Elem {
    Contact {
        kind: ContactKind,
        operand: String,
    },
    /// Comparateur utilisé comme contact (CMP == de TIA).
    Compare {
        op: CmpOp,
        a: String,
        b: String,
    },
    Timer {
        kind: TimerKind,
        instance: String,
        pt: String,
    },
    /// Compteur : l'entrée de comptage est le RLO, `reset` est un opérande BOOL.
    Counter {
        kind: CounterKind,
        instance: String,
        reset: String,
        pv: String,
    },
    /// Branches parallèles (OU), chacune étant une série.
    Parallel {
        branches: Vec<Vec<Elem>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactKind {
    /// Contact à fermeture ─┤ ├─
    No,
    /// Contact à ouverture ─┤/├─
    Nc,
    /// Front montant ─┤P├─
    Pos,
    /// Front descendant ─┤N├─
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub const ALL: [CmpOp; 6] = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];

    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "=",
            CmpOp::Ne => "<>",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    /// Comme dans TIA : `==`, `<>`, `<`…
    pub fn label(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            other => other.symbol(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimerKind {
    Ton,
    Tof,
    Tp,
}

impl TimerKind {
    pub fn name(self) -> &'static str {
        match self {
            TimerKind::Ton => "TON",
            TimerKind::Tof => "TOF",
            TimerKind::Tp => "TP",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CounterKind {
    Ctu,
    Ctd,
}

impl CounterKind {
    pub fn name(self) -> &'static str {
        match self {
            CounterKind::Ctu => "CTU",
            CounterKind::Ctd => "CTD",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coil {
    pub kind: CoilKind,
    pub operand: String,
    /// Valeur source pour `Move`.
    #[serde(default)]
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoilKind {
    /// ─( )─ : recopie le RLO.
    Assign,
    /// ─(/)─ : recopie l'inverse du RLO.
    Negate,
    /// ─(S)─ : mise à 1 si RLO.
    Set,
    /// ─(R)─ : mise à 0 si RLO.
    Reset,
    /// MOVE : copie `value` dans l'opérande si RLO.
    Move,
}

impl CoilKind {
    pub fn symbol(self) -> &'static str {
        match self {
            CoilKind::Assign => " ",
            CoilKind::Negate => "/",
            CoilKind::Set => "S",
            CoilKind::Reset => "R",
            CoilKind::Move => "MOVE",
        }
    }
}

struct Ctx<'a> {
    net: usize,
    tmp: usize,
    max_tmp: usize,
    seq: usize,
    block: &'a Block,
    hidden: &'a mut Vec<Hidden>,
    out: &'a mut Generated,
}

impl Ctx<'_> {
    fn line(&mut self, s: String) {
        self.out.push(format!("    {s}"), Origin::Network(self.net));
    }

    fn tmp(&mut self) -> String {
        let t = format!("_rlo{}", self.tmp);
        self.tmp += 1;
        self.max_tmp = self.max_tmp.max(self.tmp);
        t
    }

    /// Instance d'un bloc standard : celle nommée par l'utilisateur (déclarée
    /// automatiquement si elle n'est pas dans l'interface), ou une interne.
    fn instance(&mut self, given: &str, ty: &str, prefix: &str) -> String {
        let name = given.trim().trim_start_matches('#').to_string();
        let name = if name.is_empty() {
            self.seq += 1;
            format!("_{prefix}{}_{}", self.net + 1, self.seq)
        } else {
            name
        };
        if !self.block.declares(&name) && !self.hidden.iter().any(|h| h.name.eq_ignore_ascii_case(&name)) {
            self.hidden.push(Hidden {
                section: Section::Static,
                name: name.clone(),
                ty: ty.into(),
                init: String::new(),
            });
        }
        format!("#{name}")
    }

    fn series(&mut self, elems: &[Elem], rlo: &str) {
        for e in elems {
            match e {
                Elem::Contact { kind, operand } => {
                    let op = or_false(operand);
                    match kind {
                        ContactKind::No => self.line(format!("{rlo} := {rlo} AND {op};")),
                        ContactKind::Nc => self.line(format!("{rlo} := {rlo} AND NOT {op};")),
                        ContactKind::Pos | ContactKind::Neg => {
                            let (ty, prefix) =
                                if *kind == ContactKind::Pos { ("R_TRIG", "P") } else { ("F_TRIG", "N") };
                            let inst = self.instance("", ty, prefix);
                            self.line(format!("{inst}(CLK := {op});"));
                            self.line(format!("{rlo} := {rlo} AND {inst}.Q;"));
                        }
                    }
                }
                Elem::Compare { op, a, b } => {
                    self.line(format!("{rlo} := {rlo} AND ({} {} {});", or_zero(a), op.symbol(), or_zero(b)));
                }
                Elem::Timer { kind, instance, pt } => {
                    let inst = self.instance(instance, kind.name(), "T");
                    let pt = if pt.trim().is_empty() { "T#0ms" } else { pt.trim() };
                    self.line(format!("{inst}(IN := {rlo}, PT := {pt});"));
                    self.line(format!("{rlo} := {inst}.Q;"));
                }
                Elem::Counter { kind, instance, reset, pv } => {
                    let inst = self.instance(instance, kind.name(), "C");
                    let reset = or_false(reset);
                    let pv = if pv.trim().is_empty() { "0" } else { pv.trim() };
                    let (cnt, rst) = if *kind == CounterKind::Ctu { ("CU", "R") } else { ("CD", "LD") };
                    self.line(format!("{inst}({cnt} := {rlo}, {rst} := {reset}, PV := {pv});"));
                    self.line(format!("{rlo} := {inst}.Q;"));
                }
                Elem::Parallel { branches } => {
                    let start = self.tmp();
                    let acc = self.tmp();
                    self.line(format!("{start} := {rlo};"));
                    self.line(format!("{acc} := FALSE;"));
                    for br in branches {
                        let b = self.tmp();
                        self.line(format!("{b} := {start};"));
                        self.series(br, &b);
                        self.line(format!("{acc} := {acc} OR {b};"));
                    }
                    self.line(format!("{rlo} := {acc};"));
                }
            }
        }
    }
}

fn or_false(op: &str) -> String {
    if op.trim().is_empty() { "FALSE".into() } else { op.trim().into() }
}

fn or_zero(op: &str) -> String {
    if op.trim().is_empty() { "0".into() } else { op.trim().into() }
}

pub fn translate(block: &Block, out: &mut Generated, hidden: &mut Vec<Hidden>) {
    let mut ctx = Ctx { net: 0, tmp: 0, max_tmp: 0, seq: 0, block, hidden, out };
    for (n, net) in block.networks.iter().enumerate() {
        ctx.net = n;
        ctx.tmp = 0;
        ctx.seq = 0;
        let title = net.title.trim();
        let o = Origin::Network(n);
        ctx.out.push(
            format!("    // Réseau {}{}", n + 1, if title.is_empty() { String::new() } else { format!(" : {title}") }),
            o,
        );
        if net.rung.is_empty() && net.coils.is_empty() {
            continue;
        }
        let rlo = ctx.tmp();
        ctx.line(format!("{rlo} := TRUE;"));
        ctx.series(&net.rung, &rlo);
        for c in &net.coils {
            let op = c.operand.trim();
            if op.is_empty() {
                continue;
            }
            match c.kind {
                CoilKind::Assign => ctx.line(format!("{op} := {rlo};")),
                CoilKind::Negate => ctx.line(format!("{op} := NOT {rlo};")),
                CoilKind::Set => ctx.line(format!("IF {rlo} THEN {op} := TRUE; END_IF;")),
                CoilKind::Reset => ctx.line(format!("IF {rlo} THEN {op} := FALSE; END_IF;")),
                CoilKind::Move => ctx.line(format!("IF {rlo} THEN {op} := {}; END_IF;", or_zero(&c.value))),
            }
        }
    }
    for t in 0..ctx.max_tmp {
        ctx.hidden.push(Hidden {
            section: Section::Temp,
            name: format!("_rlo{t}"),
            ty: "BOOL".into(),
            init: String::new(),
        });
    }
}
