use thiserror::Error;

#[derive(Debug, Error)]
pub enum MuError {
    #[error("{0}")]
    Db(#[from] rusqlite::Error),

    #[error("AppleScript error: {0}")]
    AppleScript(String),

    #[error("{0}")]
    Download(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("track not found")]
    TrackNotFound,

    #[error("playlist not found")]
    PlaylistNotFound,

    #[error("track already exists (id: {id}, title: '{title}')")]
    DuplicateTrack { id: i64, title: String },

    #[error("{0}")]
    ExternalTool(String),
}

pub type Result<T> = std::result::Result<T, MuError>;

pub fn json_error(msg: &str) -> String {
    serde_json::json!({"error": msg}).to_string()
}

pub fn json_ok(msg: &str) -> String {
    serde_json::json!({"ok": true, "message": msg}).to_string()
}
