//! Langages de programmation des blocs : SCL/ST, CONT (ladder), GRAPH (SFC)
//! et BASIC.
//!
//! Tous les langages sont traduits en Structured Text avant la compilation.
//! Les règles de déterminisme et la preuve du temps de cycle s'appliquent
//! donc à l'identique, et l'IDE peut montrer le ST généré. Chaque ligne
//! générée garde son [`Origin`] pour renvoyer les erreurs au bon réseau, à la
//! bonne étape ou à la bonne ligne BASIC.

pub mod basic;
pub mod ladder;
pub mod sfc;

use serde::{Deserialize, Serialize};

use crate::ast::PouKind;
use crate::project::Block;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// SCL / Structured Text : la source contient tout le bloc.
    #[default]
    St,
    /// CONT (ladder) : réseaux de contacts et de bobines.
    Ladder,
    /// GRAPH (SFC) : étapes et transitions.
    Sfc,
    /// BASIC structuré.
    Basic,
}

impl Language {
    pub const ALL: [Language; 4] = [Language::Ladder, Language::St, Language::Sfc, Language::Basic];

    /// Nom affiché, celui de TIA Portal en français quand il existe.
    pub fn label(self) -> &'static str {
        match self {
            Language::St => "SCL",
            Language::Ladder => "CONT",
            Language::Sfc => "GRAPH",
            Language::Basic => "BASIC",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Language::St => "Texte structuré (IEC 61131-3 ST)",
            Language::Ladder => "Schéma à contacts (ladder)",
            Language::Sfc => "Grafcet : étapes et transitions (SFC)",
            Language::Basic => "BASIC structuré",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockType {
    #[default]
    Ob,
    Fb,
    Fc,
}

impl BlockType {
    pub fn pou_kind(self) -> PouKind {
        match self {
            BlockType::Ob => PouKind::Program,
            BlockType::Fb => PouKind::FunctionBlock,
            BlockType::Fc => PouKind::Function,
        }
    }

    pub fn from_pou(k: PouKind) -> BlockType {
        match k {
            PouKind::Program => BlockType::Ob,
            PouKind::FunctionBlock => BlockType::Fb,
            PouKind::Function => BlockType::Fc,
        }
    }

    fn keywords(self) -> (&'static str, &'static str) {
        match self {
            BlockType::Ob => ("ORGANIZATION_BLOCK", "END_ORGANIZATION_BLOCK"),
            BlockType::Fb => ("FUNCTION_BLOCK", "END_FUNCTION_BLOCK"),
            BlockType::Fc => ("FUNCTION", "END_FUNCTION"),
        }
    }

    /// Sections d'interface proposées, comme dans TIA.
    pub fn sections(self) -> &'static [Section] {
        match self {
            BlockType::Ob => &[Section::Static, Section::Temp, Section::Constant],
            BlockType::Fb => &[Section::Input, Section::Output, Section::Static, Section::Temp, Section::Constant],
            BlockType::Fc => &[Section::Input, Section::Output, Section::Temp, Section::Constant],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Input,
    Output,
    #[default]
    Static,
    Temp,
    Constant,
}

impl Section {
    pub fn label(self) -> &'static str {
        match self {
            Section::Input => "Input",
            Section::Output => "Output",
            Section::Static => "Static",
            Section::Temp => "Temp",
            Section::Constant => "Constant",
        }
    }

    fn keyword(self) -> &'static str {
        match self {
            Section::Input => "VAR_INPUT",
            Section::Output => "VAR_OUTPUT",
            Section::Static => "VAR",
            Section::Temp => "VAR_TEMP",
            Section::Constant => "VAR CONSTANT",
        }
    }
}

/// Une ligne de la table d'interface d'un bloc (CONT, GRAPH, BASIC).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IfaceVar {
    pub section: Section,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub init: String,
    #[serde(default)]
    pub comment: String,
}

impl IfaceVar {
    pub fn new(section: Section, name: &str, ty: &str) -> IfaceVar {
        IfaceVar { section, name: name.into(), ty: ty.into(), ..Default::default() }
    }
}

/// Provenance d'une ligne de ST générée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// En-tête ou interface du bloc.
    Interface,
    /// Réseau CONT (index à partir de 0).
    Network(usize),
    /// Étape GRAPH (index).
    Step(usize),
    /// Transition GRAPH (index).
    Transition(usize),
    /// Ligne de la source BASIC (à partir de 1).
    Line(u32),
    /// Code ajouté par la traduction (variables internes, gestion des étapes).
    Generated,
}

impl Origin {
    /// Libellé pour les messages : « réseau 3 », « étape S2 »…
    pub fn describe(self, block: &Block) -> Option<String> {
        match self {
            Origin::Network(i) => {
                let title = block.networks.get(i).map(|n| n.title.trim()).unwrap_or("");
                Some(if title.is_empty() {
                    format!("réseau {}", i + 1)
                } else {
                    format!("réseau {} « {title} »", i + 1)
                })
            }
            Origin::Step(i) => block.sfc.steps.get(i).map(|s| format!("étape S{} « {} »", s.number, s.name)),
            Origin::Transition(i) => {
                block.sfc.transitions.get(i).map(|t| format!("transition S{} → S{}", t.from, t.to))
            }
            Origin::Interface => Some("interface".into()),
            Origin::Line(_) | Origin::Generated => None,
        }
    }
}

/// ST généré, ligne par ligne, avec la provenance de chaque ligne.
#[derive(Clone, Debug, Default)]
pub struct Generated {
    pub lines: Vec<(String, Origin)>,
}

impl Generated {
    pub fn push(&mut self, line: impl Into<String>, origin: Origin) {
        self.lines.push((line.into(), origin));
    }

    pub fn source(&self) -> String {
        let mut s = String::new();
        for (l, _) in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    pub fn origin(&self, line: u32) -> Origin {
        self.lines.get(line.saturating_sub(1) as usize).map(|(_, o)| *o).unwrap_or(Origin::Generated)
    }
}

/// Variable interne ajoutée par la traduction (instance de front, mémoire de
/// branche, numéro d'étape…).
pub struct Hidden {
    pub section: Section,
    pub name: String,
    pub ty: String,
    pub init: String,
}

impl Block {
    /// Bloc en SCL/ST : la source contient tout.
    pub fn st(name: &str, source: &str) -> Block {
        Block { name: name.into(), language: Language::St, source: source.into(), ..Block::empty(name) }
    }

    pub fn empty(name: &str) -> Block {
        Block {
            name: name.into(),
            language: Language::St,
            source: String::new(),
            kind: BlockType::Ob,
            return_type: String::new(),
            interface: Vec::new(),
            networks: Vec::new(),
            sfc: Default::default(),
        }
    }

    /// Nouveau bloc vide dans le langage choisi.
    pub fn new(name: &str, kind: BlockType, language: Language) -> Block {
        let mut b = Block::empty(name);
        b.kind = kind;
        b.language = language;
        if kind == BlockType::Fc {
            b.return_type = "VOID".into();
        }
        match language {
            Language::St => {
                b.source = match kind {
                    BlockType::Ob => {
                        format!("ORGANIZATION_BLOCK {name}\nVAR\nEND_VAR\nBEGIN\n\nEND_ORGANIZATION_BLOCK\n")
                    }
                    BlockType::Fb => format!(
                        "FUNCTION_BLOCK {name}\nVAR_INPUT\nEND_VAR\nVAR_OUTPUT\nEND_VAR\nVAR\nEND_VAR\nBEGIN\n\nEND_FUNCTION_BLOCK\n"
                    ),
                    BlockType::Fc => format!("FUNCTION {name} : VOID\nVAR_INPUT\nEND_VAR\nBEGIN\n\nEND_FUNCTION\n"),
                }
            }
            Language::Ladder => b.networks.push(ladder::Network::default()),
            Language::Sfc => b.sfc = sfc::Sfc::starter(),
            Language::Basic => b.source = "' Programme BASIC\n".into(),
        }
        b
    }

    /// Type de bloc (OB, FB, FC). En ST, c'est l'en-tête de la source qui fait foi.
    pub fn block_type(&self) -> Option<BlockType> {
        match self.language {
            Language::St => crate::block_kind(&self.source).map(BlockType::from_pou),
            _ => Some(self.kind),
        }
    }

    /// Traduit le bloc en Structured Text.
    pub fn to_st(&self) -> Generated {
        if self.language == Language::St {
            let mut g = Generated::default();
            for (i, l) in self.source.lines().enumerate() {
                g.push(l, Origin::Line(i as u32 + 1));
            }
            return g;
        }
        let mut body = Generated::default();
        let mut hidden = Vec::new();
        match self.language {
            Language::Ladder => ladder::translate(self, &mut body, &mut hidden),
            Language::Sfc => sfc::translate(self, &mut body, &mut hidden),
            Language::Basic => basic::translate(self, &mut body, &mut hidden),
            Language::St => unreachable!(),
        }
        let mut g = Generated::default();
        let (start, end) = self.kind.keywords();
        let ret = match self.kind {
            BlockType::Fc => {
                let r = self.return_type.trim();
                format!(" : {}", if r.is_empty() { "VOID" } else { r })
            }
            _ => String::new(),
        };
        g.push(format!("{start} {}{ret}", self.name), Origin::Interface);
        let sections = [Section::Input, Section::Output, Section::Static, Section::Temp, Section::Constant];
        for sec in sections {
            let vars: Vec<&IfaceVar> =
                self.interface.iter().filter(|v| v.section == sec && !v.name.trim().is_empty()).collect();
            let extra: Vec<&Hidden> = hidden.iter().filter(|h| h.section == sec).collect();
            if vars.is_empty() && extra.is_empty() {
                continue;
            }
            g.push(sec.keyword(), Origin::Interface);
            for v in vars {
                let init = if v.init.trim().is_empty() { String::new() } else { format!(" := {}", v.init.trim()) };
                let comment =
                    if v.comment.trim().is_empty() { String::new() } else { format!(" // {}", v.comment.trim()) };
                g.push(format!("    {} : {}{init};{comment}", v.name.trim(), v.ty.trim()), Origin::Interface);
            }
            for h in extra {
                let init = if h.init.is_empty() { String::new() } else { format!(" := {}", h.init) };
                g.push(format!("    {} : {}{init};", h.name, h.ty), Origin::Generated);
            }
            g.push("END_VAR", Origin::Interface);
        }
        g.push("BEGIN", Origin::Interface);
        g.lines.extend(body.lines);
        g.push(end, Origin::Interface);
        g
    }

    /// Variable de l'interface ou variable interne déclarée par la traduction ?
    pub fn declares(&self, name: &str) -> bool {
        self.interface.iter().any(|v| v.name.trim().eq_ignore_ascii_case(name))
    }
}
