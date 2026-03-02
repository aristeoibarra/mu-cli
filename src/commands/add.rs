use crate::error::Result;
use crate::{db, downloader, music};
use rusqlite::params;
use std::path::Path;

pub fn handle_add(db_path: &Path, query: &str, playlist: Option<String>) -> Result<()> {
    let conn = db::open(db_path)?;
    let result = downloader::download(query, &conn)?;

    let file_path = Path::new(&result.file);
    let persistent_id = match music::import_with_metadata(
        file_path,
        result.artist.as_deref(),
        result.album.as_deref(),
        Some("Music"),
    ) {
        Ok(import) => {
            if let Some(ref pid) = import.persistent_id {
                conn.execute(
                    "UPDATE tracks SET apple_music_id = ?1 WHERE id = ?2",
                    params![pid, result.id],
                )
                .ok();
            }
            import.persistent_id
        }
        Err(e) => {
            eprintln!("Warning: Failed to import to Apple Music: {e}");
            None
        }
    };

    if let Some(pl_name) = playlist {
        if let Some(pl_id) = db::resolve_playlist_id(&conn, &pl_name) {
            let pos = db::next_playlist_position(&conn, pl_id);
            conn.execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![pl_id, result.id, pos],
            )
            .ok();
        }
        if let Some(ref pid) = persistent_id {
            let _ = music::add_track_to_playlist_by_id(pid, &pl_name);
        } else {
            let _ = music::add_track_to_playlist(&result.title, &pl_name);
        }
    }

    println!("{}", serde_json::to_string(&result).unwrap());
    Ok(())
}
