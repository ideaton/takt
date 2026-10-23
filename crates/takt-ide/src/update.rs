//! Mises à jour par les « Releases » GitHub du projet : l'appli regarde la
//! dernière version publiée, et sur demande télécharge son installateur et le
//! lance en mode mise à jour (il ferme l'appli, la remplace et la relance).

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

const LATEST: &str = "https://api.github.com/repos/ideaton/takt/releases/latest";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub file: String,
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
    Ok(Some(Release {
        version: tag,
        notes: json["body"].as_str().unwrap_or_default().to_string(),
        url: asset["browser_download_url"].as_str().unwrap_or_default().to_string(),
        file: asset["name"].as_str().unwrap_or("TaktStudio-setup.exe").to_string(),
    }))
}

fn download(r: &Release) -> Result<PathBuf, String> {
    if !r.url.starts_with("https://github.com/ideaton/takt/releases/download/") {
        return Err("adresse de téléchargement inattendue".into());
    }
    let mut resp = agent().get(&r.url).call().map_err(|e| format!("téléchargement : {e}"))?;
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(200 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("téléchargement : {e}"))?;
    if bytes.len() < 1024 || &bytes[..2] != b"MZ" {
        return Err("le fichier téléchargé n'est pas un installateur".into());
    }
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
