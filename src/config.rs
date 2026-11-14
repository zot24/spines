use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    pub music: Option<PathBuf>,
    pub books: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = required("SPINES_DATA")?;
        let bind = env::var("SPINES_BIND").unwrap_or_else(|_| "0.0.0.0:8787".into());
        let bind: SocketAddr = bind
            .parse()
            .map_err(|_| Error::msg(format!("SPINES_BIND is not an address: {bind}")))?;
        Ok(Self {
            data_dir,
            bind,
            music: optional_path("SPINES_MUSIC"),
            books: optional_path("SPINES_BOOKS"),
        })
    }

    pub fn for_roots(data_dir: PathBuf, music: Option<PathBuf>, books: Option<PathBuf>) -> Self {
        Self {
            data_dir,
            bind: "127.0.0.1:0".parse().unwrap(),
            music,
            books,
        }
    }
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
