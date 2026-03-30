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

    #[error("validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, MuError>;

pub fn json_error(msg: &str) -> String {
    serde_json::json!({"error": msg}).to_string()
}

pub fn json_ok(msg: &str) -> String {
    serde_json::json!({"ok": true, "message": msg}).to_string()
}

pub fn json_result(value: serde_json::Value, warnings: &[String]) -> String {
    if warnings.is_empty() {
        value.to_string()
    } else {
        let mut obj = value;
        if let Some(map) = obj.as_object_mut() {
            map.insert(
                "warnings".to_string(),
                serde_json::Value::Array(
                    warnings
                        .iter()
                        .map(|w| serde_json::Value::String(w.clone()))
                        .collect(),
                ),
            );
        }
        obj.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_error_format() {
        let result = json_error("something failed");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["error"], "something failed");
    }

    #[test]
    fn json_ok_format() {
        let result = json_ok("done");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["message"], "done");
    }

    #[test]
    fn json_result_without_warnings() {
        let result = json_result(serde_json::json!({"ok": true}), &[]);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["ok"], true);
        assert!(parsed.get("warnings").is_none());
    }

    #[test]
    fn json_result_with_warnings() {
        let warnings = vec!["warn1".to_string(), "warn2".to_string()];
        let result = json_result(serde_json::json!({"ok": true}), &warnings);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["ok"], true);
        let w = parsed["warnings"].as_array().unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0], "warn1");
        assert_eq!(w[1], "warn2");
    }
}
