//! GRAPH (SFC / Grafcet) : étapes, actions et transitions.
//!
//! La séquence devient une machine à états : la variable `Etape` contient le
//! numéro de l'étape active et `T_Etape` le temps passé dedans (utilisable
//! dans les conditions : `T_Etape >= T#2s`). La première étape de la liste
//! est l'étape initiale.

use serde::{Deserialize, Serialize};

use super::{Block, Generated, Hidden, Origin, Section};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sfc {
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub number: u16,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub qualifier: Qualifier,
    /// Opérande BOOL pour N, S, R ; instruction ST complète pour `Code`.
    pub operand: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Qualifier {
    /// Non mémorisée : vrai tant que l'étape est active.
    N,
    /// Mise à 1 mémorisée.
    S,
    /// Remise à 0.
    R,
    /// Instruction ST exécutée à chaque cycle tant que l'étape est active.
    Code,
}

impl Qualifier {
    pub const ALL: [Qualifier; 4] = [Qualifier::N, Qualifier::S, Qualifier::R, Qualifier::Code];

    pub fn label(self) -> &'static str {
        match self {
            Qualifier::N => "N",
            Qualifier::S => "S",
            Qualifier::R => "R",
            Qualifier::Code => "ST",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub from: u16,
    pub to: u16,
    /// Condition en ST ; vide = jamais franchie.
    #[serde(default)]
    pub condition: String,
}

/// Nom de la variable qui contient le numéro de l'étape active.
pub const STEP_VAR: &str = "Etape";
/// Nom de la variable qui contient le temps passé dans l'étape active.
pub const STEP_TIME_VAR: &str = "T_Etape";

impl Sfc {
    /// Séquence de départ : deux étapes qui bouclent.
    pub fn starter() -> Sfc {
        Sfc {
            steps: vec![
                Step { number: 1, name: "Repos".into(), actions: vec![] },
                Step { number: 2, name: "Étape 2".into(), actions: vec![] },
            ],
            transitions: vec![
                Transition { from: 1, to: 2, condition: String::new() },
                Transition { from: 2, to: 1, condition: "T_Etape >= T#1s".into() },
            ],
        }
    }

    pub fn initial(&self) -> u16 {
        self.steps.first().map(|s| s.number).unwrap_or(1)
    }

    pub fn step_index(&self, number: u16) -> Option<usize> {
        self.steps.iter().position(|s| s.number == number)
    }

    /// Prochain numéro d'étape libre.
    pub fn next_number(&self) -> u16 {
        self.steps.iter().map(|s| s.number).max().unwrap_or(0) + 1
    }

    /// Erreurs de structure, que le ST généré ne permettrait pas de voir.
    pub fn check(&self) -> Vec<(Origin, String)> {
        let mut errs = Vec::new();
        if self.steps.is_empty() {
            errs.push((Origin::Generated, "le graphe n'a aucune étape".into()));
        }
        for (i, s) in self.steps.iter().enumerate() {
            if self.steps[..i].iter().any(|o| o.number == s.number) {
                errs.push((Origin::Step(i), format!("le numéro d'étape S{} est utilisé deux fois", s.number)));
            }
        }
        for (k, t) in self.transitions.iter().enumerate() {
            for n in [t.from, t.to] {
                if self.step_index(n).is_none() {
                    errs.push((Origin::Transition(k), format!("l'étape S{n} n'existe pas")));
                }
            }
        }
        errs
    }
}

pub fn translate(block: &Block, out: &mut Generated, hidden: &mut Vec<Hidden>) {
    let g = &block.sfc;
    let init = g.initial();
    let mut add = |name: &str, ty: &str, init: String| {
        if !block.declares(name) {
            hidden.push(Hidden { section: Section::Static, name: name.into(), ty: ty.into(), init });
        }
    };
    add(STEP_VAR, "INT", init.to_string());
    add(STEP_TIME_VAR, "TIME", String::new());
    add("_etape_prec", "INT", "-1".into());
    add("_t_debut", "TIME", String::new());

    let synth = Origin::Generated;
    out.push("    // GRAPH : gestion de l'étape active", synth);
    out.push(format!("    IF FirstScan THEN #{STEP_VAR} := {init}; END_IF;"), synth);
    out.push(
        format!("    IF #{STEP_VAR} <> #_etape_prec THEN #_t_debut := SysTime; #_etape_prec := #{STEP_VAR}; END_IF;"),
        synth,
    );
    out.push(format!("    #{STEP_TIME_VAR} := SysTime - #_t_debut;"), synth);

    // Actions N : vraies tant qu'une des étapes qui les portent est active
    let mut n_ops: Vec<(String, Vec<u16>, usize)> = Vec::new();
    for (i, s) in g.steps.iter().enumerate() {
        for a in s.actions.iter().filter(|a| a.qualifier == Qualifier::N && !a.operand.trim().is_empty()) {
            let op = a.operand.trim().to_string();
            match n_ops.iter_mut().find(|(o, _, _)| o.eq_ignore_ascii_case(&op)) {
                Some((_, steps, _)) => steps.push(s.number),
                None => n_ops.push((op, vec![s.number], i)),
            }
        }
    }
    for (op, steps, first) in &n_ops {
        let cond: Vec<String> = steps.iter().map(|n| format!("#{STEP_VAR} = {n}")).collect();
        out.push(format!("    {op} := {};", cond.join(" OR ")), Origin::Step(*first));
    }

    out.push(format!("    CASE #{STEP_VAR} OF"), synth);
    for (i, s) in g.steps.iter().enumerate() {
        let o = Origin::Step(i);
        out.push(format!("        {}: // S{} {}", s.number, s.number, s.name.trim()), o);
        for a in &s.actions {
            let op = a.operand.trim();
            if op.is_empty() {
                continue;
            }
            match a.qualifier {
                Qualifier::N => {}
                Qualifier::S => out.push(format!("            {op} := TRUE;"), o),
                Qualifier::R => out.push(format!("            {op} := FALSE;"), o),
                Qualifier::Code => {
                    let stmt = if op.ends_with(';') { op.to_string() } else { format!("{op};") };
                    out.push(format!("            {stmt}"), o);
                }
            }
        }
        let outgoing: Vec<(usize, &Transition)> =
            g.transitions.iter().enumerate().filter(|(_, t)| t.from == s.number).collect();
        for (n, (k, t)) in outgoing.iter().enumerate() {
            let cond = if t.condition.trim().is_empty() { "FALSE" } else { t.condition.trim() };
            let kw = if n == 0 { "IF" } else { "ELSIF" };
            out.push(format!("            {kw} {cond} THEN #{STEP_VAR} := {};", t.to), Origin::Transition(*k));
        }
        if !outgoing.is_empty() {
            out.push("            END_IF;", o);
        }
    }
    out.push("    END_CASE;", synth);
}
