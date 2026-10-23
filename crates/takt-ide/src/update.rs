//! Mises à jour par les « Releases » GitHub du projet : l'appli regarde la
//! dernière version publiée, et sur demande télécharge son installateur et le
//! lance en mode mise à jour (il ferme l'appli, la remplace et la relance).
//!
//! L'installateur n'est lancé que s'il vient des Releases du dépôt et que son
//! empreinte SHA-256 correspond au fichier `.sha256` publié à côté.

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use sha2::{Digest, Sha256};

const LATEST: &str = "https://api.github.com/repos/ideaton/takt/releases/latest";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub file: String,
    /// Adresse du fichier `<installateur>.sha256`.
    pub sha_url: String,
}

pub enum State {
    Idle,
    Checking(Receiver<Result<Option<Release>, String>>),
    UpToDate,
    Available(Release),
    Downloading(Receiver<Result<PathBuf, String>>, Release),
    Failed(String),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .user_agent(format!("TaktStudio/{VERSION}"))
        .build()
        .into()
}

/// Version publiée plus récente que celle-ci, s'il y en a une.
fn check() -> Result<Option<Release>, String> {
    let mut resp = agent()
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GitHub injoignable : {e}"))?;
    let json: serde_json::Value = resp.body_mut().read_json().map_err(|e| format!("réponse illisible : {e}"))?;
    let tag = json["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_string();
    let (Ok(latest), Ok(current)) = (semver::Version::parse(&tag), semver::Version::parse(VERSION)) else {
        return Ok(None);
    };
    if latest <= current {
        return Ok(None);
    }
    let asset = json["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|n| n.ends_with("-setup.exe"))));
    let Some(asset) = asset else {
        return Ok(None);
    };
    let file = asset["name"].as_str().unwrap_or("TaktStudio-setup.exe").to_string();
    let sha_name = format!("{file}.sha256");
    let sha_url = json["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(sha_name.as_str())))
        .and_then(|x| x["browser_download_url"].as_str())
        .unwrap_or_default()
        .to_string();
    Ok(Some(Release {
        version: tag,
        notes: json["body"].as_str().unwrap_or_default().to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        file,
        sha_url,
    }))
}

const RELEASES: &str = "https://github.com/ideaton/takt/releases/download/";

fn fetch(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    if !url.starts_with(RELEASES) {
        return Err("adresse de téléchargement inattendue".into());
    }
    let mut resp = agent().get(url).call().map_err(|e| format!("téléchargement : {e}"))?;
    let mut bytes = Vec::new();
    resp.body_mut().as_reader().take(limit).read_to_end(&mut bytes).map_err(|e| format!("téléchargement : {e}"))?;
    Ok(bytes)
}

/// Vérifie l'empreinte d'un fichier contre le contenu d'un `.sha256`
/// (format `sha256sum` : l'empreinte en hexadécimal, puis le nom du fichier).
pub fn verify(bytes: &[u8], sha_file: &str) -> Result<(), String> {
    let expected = sha_file.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    let actual: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if expected.len() != 64 || expected != actual {
        return Err("empreinte SHA-256 différente : installateur refusé".into());
    }
    Ok(())
}

fn download(r: &Release) -> Result<PathBuf, String> {
    if r.sha_url.is_empty() {
        return Err("cette version ne publie pas d'empreinte SHA-256 : mise à jour refusée".into());
    }
    let sha = String::from_utf8(fetch(&r.sha_url, 4096)?).map_err(|_| "empreinte illisible".to_string())?;
    let bytes = fetch(&r.url, 200 * 1024 * 1024)?;
    if bytes.len() < 1024 || &bytes[..2] != b"MZ" {
        return Err("le fichier téléchargé n'est pas un installateur".into());
    }
    verify(&bytes, &sha)?;
    let dir = std::env::temp_dir().join("TaktStudio-maj");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&r.file);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn start_check() -> State {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(check());
    });
    State::Checking(rx)
}

pub fn start_download(r: Release) -> State {
    let (tx, rx) = channel();
    let rel = r.clone();
    std::thread::spawn(move || {
        let _ = tx.send(download(&rel));
    });
    State::Downloading(rx, r)
}

/// Lance l'installateur en mode mise à jour ; il ferme l'appli lui-même.
pub fn install(path: &PathBuf) -> Result<(), String> {
    std::process::Command::new(path).arg("/UPDATE").spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Fait avancer les tâches en arrière-plan. Renvoie l'installateur prêt à lancer.
pub fn poll(state: &mut State) -> Option<PathBuf> {
    let next = match state {
        State::Checking(rx) => match rx.try_recv() {
            Ok(Ok(Some(r))) => Some(State::Available(r)),
            Ok(Ok(None)) => Some(State::UpToDate),
            Ok(Err(e)) => Some(State::Failed(e)),
            Err(_) => None,
        },
        State::Downloading(rx, _) => match rx.try_recv() {
            Ok(Ok(path)) => {
                *state = State::Idle;
                return Some(path);
            }
            Ok(Err(e)) => Some(State::Failed(e)),
            Err(_) => None,
        },
        _ => None,
    };
    if let Some(n) = next {
        *state = n;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empreinte() {
        let sha = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  TaktStudio-1.0.0-setup.exe";
        assert!(verify(b"hello", sha).is_ok());
        assert!(verify(b"hellO", sha).is_err());
        assert!(verify(b"hello", "").is_err());
    }
}
