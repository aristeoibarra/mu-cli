use crate::error::{MuError, Result};
use crate::{db, music};
use rusqlite::params;
use std::path::Path;

pub fn handle_status(db_path: &Path) -> Result<()> {
    let status = music::get_status()?;
    let favorite = status
        .track
        .as_ref()
        .and_then(|track_name| {
            let conn = db::open(db_path).ok()?;
            conn.query_row(
                "SELECT COALESCE(favorite, 0) FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                params![track_name],
                |row| row.get::<_, bool>(0),
            )
            .ok()
        })
        .unwrap_or(false);
    println!(
        "{}",
        serde_json::json!({
            "track": status.track,
            "artist": status.artist,
            "album": status.album,
            "state": status.state,
            "position_secs": status.position_secs,
            "duration_secs": status.duration_secs,
            "favorite": favorite,
        })
    );
    Ok(())
}

pub fn handle_list(db_path: &Path, playlist: Option<&str>) -> Result<()> {
    let conn = db::open(db_path)?;
    if let Some(pl_name) = playlist {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.title, t.artist, t.album, t.duration_secs, t.artwork_path, COALESCE(t.favorite, 0) FROM tracks t
             JOIN playlist_tracks pt ON pt.track_id = t.id
             JOIN playlists p ON p.id = pt.playlist_id
             WHERE p.name = ?1
             ORDER BY pt.position",
        )?;
        let rows: Vec<serde_json::Value> = stmt
            .query_map(params![pl_name], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, i64>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "artist": row.get::<_, Option<String>>(2)?,
                    "album": row.get::<_, Option<String>>(3)?,
                    "duration": row.get::<_, Option<i64>>(4)?,
                    "artwork": row.get::<_, Option<String>>(5)?,
                    "favorite": row.get::<_, bool>(6)?,
                }))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        println!(
            "{}",
            serde_json::json!({"playlist": pl_name, "tracks": rows})
        );
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, title, artist, album, duration_secs, artwork_path, COALESCE(favorite, 0) FROM tracks ORDER BY id",
        )?;
        let rows: Vec<serde_json::Value> = stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, i64>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "artist": row.get::<_, Option<String>>(2)?,
                    "album": row.get::<_, Option<String>>(3)?,
                    "duration": row.get::<_, Option<i64>>(4)?,
                    "artwork": row.get::<_, Option<String>>(5)?,
                    "favorite": row.get::<_, bool>(6)?,
                }))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        println!("{}", serde_json::json!({"tracks": rows}));
    }
    Ok(())
}

pub fn handle_remove(db_path: &Path, track: &str) -> Result<()> {
    let conn = db::open(db_path)?;
    let (tid, file_path, artwork_path, apple_music_id) =
        db::resolve_track_for_remove(&conn, track).ok_or(MuError::TrackNotFound)?;

    // Remove from Apple Music (by persistent ID, fallback to file path)
    if let Some(ref pid) = apple_music_id {
        if let Err(e) = music::delete_track(pid) {
            eprintln!("Warning: failed to remove from Apple Music: {e}");
        }
    } else if let Err(e) = music::delete_track_by_path(&file_path) {
        eprintln!("Warning: failed to remove from Apple Music: {e}");
    }

    conn.execute(
        "DELETE FROM playlist_tracks WHERE track_id = ?1",
        params![tid],
    )?;
    conn.execute("DELETE FROM tracks WHERE id = ?1", params![tid])?;
    let _ = std::fs::remove_file(&file_path);
    if let Some(art) = &artwork_path {
        let _ = std::fs::remove_file(art);
    }
    println!(
        "{}",
        serde_json::json!({"ok": true, "removed_id": tid, "file_deleted": file_path})
    );
    Ok(())
}

pub fn handle_migrate(db_path: &Path, dry_run: bool) -> Result<()> {
    let conn = db::open(db_path)?;
    let tracks = db::all_track_rows(&conn)?;

    if dry_run {
        println!(
            "{}",
            serde_json::json!({
                "dry_run": true,
                "tracks_to_migrate": tracks.len(),
                "tracks": tracks.iter().map(|(id, title, artist, album, _)| {
                    serde_json::json!({ "id": id, "title": title, "artist": artist, "album": album })
                }).collect::<Vec<_>>(),
            })
        );
        return Ok(());
    }

    let (imported, skipped, failed) = import_tracks(&conn, &tracks);
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "total": tracks.len(),
            "imported": imported,
            "skipped": skipped,
            "failed": failed,
        })
    );
    Ok(())
}

pub fn handle_info() -> Result<()> {
    let stats = music::get_library_stats()?;
    let playlists = music::list_playlists().unwrap_or_default();
    #[allow(clippy::cast_possible_truncation)]
    let hours = (stats.total_duration_secs / 3600.0) as i64;
    #[allow(clippy::cast_possible_truncation)]
    let mins = ((stats.total_duration_secs % 3600.0) / 60.0) as i64;

    println!(
        "{}",
        serde_json::json!({
            "track_count": stats.track_count,
            "total_duration": format!("{hours}h {mins}m"),
            "total_duration_secs": stats.total_duration_secs,
            "playlists": playlists,
        })
    );
    Ok(())
}

pub fn handle_reimport(db_path: &Path, track: Option<&str>) -> Result<()> {
    let conn = db::open(db_path)?;

    let tracks = if let Some(t) = track {
        let row = db::resolve_track_row(&conn, t).ok_or(MuError::TrackNotFound)?;
        vec![row]
    } else {
        db::all_track_rows(&conn)?
    };

    let mut reimported = 0;
    let mut failed = 0;

    for (id, title, artist, album, file_path) in &tracks {
        let path = Path::new(file_path);

        if !path.exists() {
            failed += 1;
            eprintln!("File not found: {file_path}");
            continue;
        }

        match music::import_with_metadata(path, artist.as_deref(), album.as_deref(), Some("Music"))
        {
            Ok(import) => {
                if let Some(ref pid) = import.persistent_id {
                    if let Err(e) = conn.execute(
                        "UPDATE tracks SET apple_music_id = ?1 WHERE id = ?2",
                        params![pid, id],
                    ) {
                        eprintln!("Warning: failed to save apple_music_id for '{title}': {e}");
                    }
                }
                reimported += 1;
                eprintln!("Reimported: {title} by {artist:?}");
            }
            Err(e) => {
                failed += 1;
                eprintln!("Failed: {title}: {e}");
            }
        }
    }

    println!(
        "{}",
        serde_json::json!({ "ok": true, "reimported": reimported, "failed": failed, "total": tracks.len() })
    );
    Ok(())
}

fn import_tracks(conn: &rusqlite::Connection, tracks: &[db::TrackRow]) -> (i64, i64, i64) {
    let mut imported = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for (id, title, artist, album, file_path) in tracks {
        let path = Path::new(file_path);

        if !path.exists() {
            failed += 1;
            eprintln!("File not found: {file_path}");
            continue;
        }

        if music::is_track_in_library(path) {
            skipped += 1;
            continue;
        }

        match music::import_with_metadata(path, artist.as_deref(), album.as_deref(), Some("Music"))
        {
            Ok(import) => {
                if let Some(ref pid) = import.persistent_id {
                    if let Err(e) = conn.execute(
                        "UPDATE tracks SET apple_music_id = ?1 WHERE id = ?2",
                        params![pid, id],
                    ) {
                        eprintln!("Warning: failed to save apple_music_id for '{title}': {e}");
                    }
                }
                imported += 1;
                eprintln!("Imported: {title}");
            }
            Err(e) => {
                failed += 1;
                eprintln!("Failed to import {title}: {e}");
            }
        }
    }

    (imported, skipped, failed)
}
