use crate::error::{json_result, MuError, Result};
use crate::{db, downloader, music};
use rusqlite::params;
use std::path::Path;

pub fn handle_add(db_path: &Path, query: &str, playlist: Option<String>) -> Result<()> {
    let conn = db::open(db_path)?;
    let result = downloader::download(query, &conn)?;
    let mut warnings = Vec::new();

    let file_path = Path::new(&result.file);
    let persistent_id = {
        let mut last_err = None;
        let mut pid = None;
        for attempt in 0..4 {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt);
                std::thread::sleep(delay);
            }
            match music::import_with_metadata(
                file_path,
                result.artist.as_deref(),
                result.album.as_deref(),
            ) {
                Ok(import) => {
                    if let Some(ref p) = import.persistent_id {
                        if let Err(e) = conn.execute(
                            "UPDATE tracks SET apple_music_id = ?1 WHERE id = ?2",
                            params![p, result.id],
                        ) {
                            warnings.push(format!("failed to save apple_music_id: {e}"));
                        }
                    }
                    pid = import.persistent_id;
                    last_err = None;
                    break;
                }
                Err(e) => {
                    let is_retryable =
                        matches!(&e, MuError::AppleScript(msg) if msg.contains("(-54)"));
                    last_err = Some(e);
                    if !is_retryable {
                        break;
                    }
                }
            }
        }
        if let Some(e) = last_err {
            warnings.push(format!("failed to import to Apple Music: {e}"));
        }
        pid
    };

    if let Some(pl_name) = playlist {
        if let Some(pl_id) = db::resolve_playlist_id(&conn, &pl_name) {
            let pos = db::next_playlist_position(&conn, pl_id);
            if let Err(e) = conn.execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![pl_id, result.id, pos],
            ) {
                warnings.push(format!("failed to add track to playlist in DB: {e}"));
            }
        }
        if let Err(e) =
            music::add_track_to_playlist_smart(persistent_id.as_deref(), &result.title, &pl_name)
        {
            warnings.push(format!("failed to add track to Apple Music playlist: {e}"));
        }
    }

    println!(
        "{}",
        json_result(
            serde_json::json!({
                "ok": true,
                "id": result.id,
                "title": result.title,
                "artist": result.artist,
                "album": result.album,
                "file": result.file,
                "artwork": result.artwork,
            }),
            &warnings,
        )
    );
    Ok(())
}
