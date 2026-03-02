use crate::error::Result;
use crate::{db, downloader, music};
use rusqlite::params;
use std::path::Path;

pub fn handle_add(db_path: &Path, query: &str, playlist: Option<String>) -> Result<()> {
    let conn = db::open(db_path)?;
    let result = downloader::download(query, &conn)?;

    let file_path = Path::new(&result.file);
    if let Err(e) = music::import_with_metadata(
        file_path,
        result.artist.as_deref(),
        result.album.as_deref(),
        Some("Music"),
    ) {
        eprintln!("Warning: Failed to import to Apple Music: {e}");
    }

    if let Some(pl_name) = playlist {
        if let Some(pl_id) = db::resolve_playlist_id(&conn, &pl_name) {
            let pos = db::next_playlist_position(&conn, pl_id);
            conn.execute(
                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![pl_id, result.id, pos],
            )
            .ok();
        }
        let _ = music::add_track_to_playlist(&result.title, &pl_name);
    }

    println!("{}", serde_json::to_string(&result).unwrap());
    Ok(())
}
