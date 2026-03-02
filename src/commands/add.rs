use crate::error::{json_result, Result};
use crate::{db, downloader, music};
use rusqlite::params;
use std::path::Path;

pub fn handle_add(db_path: &Path, query: &str, playlist: Option<String>) -> Result<()> {
    let conn = db::open(db_path)?;
    let result = downloader::download(query, &conn)?;
    let mut warnings = Vec::new();

    let file_path = Path::new(&result.file);
    let persistent_id = match music::import_with_metadata(
        file_path,
        result.artist.as_deref(),
        result.album.as_deref(),
    ) {
        Ok(import) => {
            if let Some(ref pid) = import.persistent_id {
                if let Err(e) = conn.execute(
                    "UPDATE tracks SET apple_music_id = ?1 WHERE id = ?2",
                    params![pid, result.id],
                ) {
                    warnings.push(format!("failed to save apple_music_id: {e}"));
                }
            }
            import.persistent_id
        }
        Err(e) => {
            warnings.push(format!("failed to import to Apple Music: {e}"));
            None
        }
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
        if let Err(e) = music::add_track_to_playlist_smart(
            persistent_id.as_deref(),
            &result.title,
            &pl_name,
        ) {
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
