use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Image(#[from] image::ImageError),
    #[error("{0}")]
    Msg(String),
}

impl Error {
    pub fn msg(s: impl fmt::Display) -> Self {
        Self::Msg(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
