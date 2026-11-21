use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    /// Directory a chosen music folder must stay inside.
    pub media: Option<PathBuf>,
    /// Prefill for the first-open screen. Not scanned until the choice is saved.
    pub music_default: Option<PathBuf>,
    pub books: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = required("SPINES_DATA")?;
        let bind = env::var("SPINES_BIND").unwrap_or_else(|_| "0.0.0.0:8787".into());
        let bind: SocketAddr = bind
            .parse()
            .map_err(|_| Error::msg(format!("SPINES_BIND is not an address: {bind}")))?;
        let music_default = optional_path("SPINES_MUSIC");
        let media = optional_path("SPINES_MEDIA").or_else(|| music_default.clone());
        Ok(Self {
            data_dir,
            bind,
            media,
            music_default,
            books: optional_path("SPINES_BOOKS"),
        })
    }

    pub fn for_roots(data_dir: PathBuf, music: Option<PathBuf>, books: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            bind: "127.0.0.1:0".parse().unwrap(),
            media: music.clone(),
            music_default: music,
            books,
        }
    }
}

/// Accept a music folder only when it exists, is readable, and stays inside `media`.
pub fn choose_music(media: &std::path::Path, raw: &str) -> Result<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err(Error::msg("Enter a folder inside the media mount."));
    }
    let media_c = media
        .canonicalize()
        .map_err(|_| Error::msg("The media mount is not readable."))?;
    let candidate = std::path::Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        media_c.join(candidate)
    };
    let chosen = joined
        .canonicalize()
        .map_err(|_| Error::msg("That folder is not there."))?;
    if !chosen.starts_with(&media_c) {
        return Err(Error::msg("That folder is outside the media mount."));
    }
    if !chosen.is_dir() {
        return Err(Error::msg("That path is not a folder."));
    }
    std::fs::read_dir(&chosen).map_err(|_| Error::msg("That folder is not readable."))?;
    Ok(chosen)
}

fn required(key: &str) -> Result<PathBuf> {
    let value = env::var(key).map_err(|_| Error::msg(format!("{key} is required")))?;
    if value.trim().is_empty() {
        return Err(Error::msg(format!("{key} is required")));
    }
    Ok(PathBuf::from(value))
}

fn optional_path(key: &str) -> Option<PathBuf> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}
