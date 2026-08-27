use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("network: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    Hash {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("version {0} is not in the manifest")]
    UnknownVersion(String),
    #[error("{0}")]
    Other(String),
}

// Commands return this straight to the UI, so it has to serialise as a plain string.
impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
