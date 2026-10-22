//! Projet Takt : appareil, variables API, blocs de programme, tables de
//! visualisation. Enregistré en JSON (`.takt`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub device: Device,
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub watch_tables: Vec<WatchTable>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    /// Nom de l'automate dans le projet (« API_1 »).
    pub name: String,
    /// Identifiant de la carte, voir [`crate::board::BOARDS`].
    pub board: String,
    /// Temps de cycle en µs.
    pub cycle_us: u32,
}

/// Une ligne de la table des variables API.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// `%I0.0`, `%Q0.1`, `%IW0`, `%QW2`, `%M10.0`, `%MW4`, `%MD8`, ou vide.
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub comment: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub name: String,
    /// Source Structured Text complète du bloc.
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WatchTable {
    pub name: String,
    #[serde(default)]
    pub rows: Vec<WatchRow>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WatchRow {
    /// Chemin de la variable : `"Moteur"`, `Main.t1.ET`, `Main.tab[2]`.
    pub path: String,
    /// Valeur de forçage ou de modification.
    #[serde(default)]
    pub modify: String,
    #[serde(default)]
    pub force: bool,
}

impl Project {
    pub fn load(path: &std::path::Path) -> Result<Project, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("lecture de {} : {e}", path.display()))?;
        Project::from_json(&text)
    }

    pub fn from_json(text: &str) -> Result<Project, String> {
        serde_json::from_str(text).map_err(|e| format!("projet invalide : {e}"))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("écriture de {} : {e}", path.display()))
    }

    /// Projet vide avec un OB principal, comme à la création dans TIA.
    pub fn new_empty(name: &str) -> Project {
        Project {
            name: name.into(),
            device: Device { name: "API_1".into(), board: "pico2".into(), cycle_us: 1000 },
            tags: vec![],
            blocks: vec![Block {
                name: "Main".into(),
                source: "ORGANIZATION_BLOCK Main\n// Bloc cyclique principal (OB1)\nVAR\nEND_VAR\nBEGIN\n\nEND_ORGANIZATION_BLOCK\n".into(),
            }],
            watch_tables: vec![WatchTable { name: "Table de visualisation_1".into(), rows: vec![] }],
        }
    }
}
