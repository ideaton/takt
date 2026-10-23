//! Réglages de l'utilisateur, gardés entre deux lancements :
//! `%APPDATA%\Takt Studio\reglages.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Projets récents, le plus récent en premier.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    #[serde(default)]
    pub dark: bool,
    /// Recherche d'une nouvelle version au lancement.
    #[serde(default = "yes")]
    pub check_updates: bool,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config { recent: Vec::new(), dark: false, check_updates: true }
    }
}

fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Takt Studio").join("reglages.json"))
}

impl Config {
    /// Les tests n'utilisent jamais les réglages de l'utilisateur.
    pub fn load() -> Config {
        if cfg!(test) {
            return Config::default();
        }
        file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Some(f) = file() {
            let _ = std::fs::create_dir_all(f.parent().unwrap());
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(f, t);
            }
        }
    }

    pub fn add_recent(&mut self, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT);
        self.save();
    }

    /// Projets récents qui existent encore.
    pub fn existing_recent(&self) -> Vec<PathBuf> {
        self.recent.iter().filter(|p| p.exists()).cloned().collect()
    }
}
