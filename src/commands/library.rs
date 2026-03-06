use crate::error::{json_result, MuError, Result};
use crate::{db, music};
use rusqlite::params;
use std::path::Path;

pub fn handle_status(db_path: &Path) -> Result<()> {
    let status = music::get_status()?;
    let (favorite, play_count) = status
        .track
        .as_ref()
        .and_then(|track_name| {
            let conn = db::open(db_path).ok()?;
            conn.query_row(
                "SELECT COALESCE(favorite, 0), COALESCE(play_count, 0) FROM tracks WHERE title = ?1",
                params![track_name],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, i64>(1)?)),
            )
            .or_else(|_| {
                conn.query_row(
                    "SELECT COALESCE(favorite, 0), COALESCE(play_count, 0) FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                    params![track_name],
                    |row| Ok((row.get::<_, bool>(0)?, row.get::<_, i64>(1)?)),
                )
            })
            .ok()
        })
        .unwrap_or((false, 0));
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
            "play_count": play_count,
        })
    );
    Ok(())
}

pub fn handle_list(db_path: &Path, playlist: Option<&str>) -> Result<()> {
    let conn = db::open(db_path)?;
    if let Some(pl_name) = playlist {
        let mut stmt = conn.prepare(
            "SELECT t.id, t.title, t.artist, t.album, t.duration_secs, t.artwork_path, COALESCE(t.favorite, 0), COALESCE(t.play_count, 0) FROM tracks t
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
                    "play_count": row.get::<_, i64>(7)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        println!(
            "{}",
            serde_json::json!({"playlist": pl_name, "tracks": rows})
        );
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, title, artist, album, duration_secs, artwork_path, COALESCE(favorite, 0), COALESCE(play_count, 0) FROM tracks ORDER BY id",
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
                    "play_count": row.get::<_, i64>(7)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        println!("{}", serde_json::json!({"tracks": rows}));
    }
    Ok(())
}

pub fn handle_remove(db_path: &Path, track: &str) -> Result<()> {
    let conn = db::open(db_path)?;
    let mut warnings = Vec::new();
    let (tid, file_path, artwork_path, apple_music_id) =
        db::resolve_track_for_remove(&conn, track).ok_or(MuError::TrackNotFound)?;

    // Remove from Apple Music (by persistent ID, fallback to file path)
    if let Some(ref pid) = apple_music_id {
        if let Err(e) = music::delete_track(pid) {
            warnings.push(format!("failed to remove from Apple Music: {e}"));
        }
    } else if let Err(e) = music::delete_track_by_path(&file_path) {
        warnings.push(format!("failed to remove from Apple Music: {e}"));
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
        json_result(
            serde_json::json!({"ok": true, "removed_id": tid, "file_deleted": file_path}),
            &warnings,
        )
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

    let (imported, skipped, failed, warnings) = import_tracks_inner(&conn, &tracks, true);
    println!(
        "{}",
        json_result(
            serde_json::json!({
                "ok": true,
                "total": tracks.len(),
                "imported": imported,
                "skipped": skipped,
                "failed": failed,
            }),
            &warnings,
        )
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

    let (reimported, _, failed, warnings) = import_tracks_inner(&conn, &tracks, false);
    println!(
        "{}",
        json_result(
            serde_json::json!({ "ok": true, "reimported": reimported, "failed": failed, "total": tracks.len() }),
            &warnings,
        )
    );
    Ok(())
}

/// Shared import loop for migrate and reimport.
/// When `check_existing` is true, skips tracks already in Apple Music (migrate behavior).
fn import_tracks_inner(
    conn: &rusqlite::Connection,
    tracks: &[db::TrackRow],
    check_existing: bool,
) -> (i64, i64, i64, Vec<String>) {
    let mut imported = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut warnings = Vec::new();

    for (id, title, artist, album, file_path) in tracks {
        let path = Path::new(file_path);

        if !path.exists() {
            failed += 1;
            warnings.push(format!("file not found: {file_path}"));
            continue;
        }

        if check_existing && music::is_track_in_library(path) {
            skipped += 1;
            continue;
        }

        // Delete old Apple Music track before re-importing to avoid duplicates
        if !check_existing {
            if let Some(old_pid) = db::get_apple_music_id(conn, *id) {
                let _ = music::delete_track(&old_pid);
            }
        }

        let mut last_err = None;
        let mut success = false;
        for attempt in 0..4 {
            if attempt > 0 {
                let delay = std::time::Duration::from_secs(1 << attempt); // 2s, 4s, 8s
                std::thread::sleep(delay);
            }
            match music::import_with_metadata(path, artist.as_deref(), album.as_deref()) {
                Ok(import) => {
                    if let Some(ref pid) = import.persistent_id {
                        if let Err(e) = conn.execute(
                            "UPDATE tracks SET apple_music_id = ?1 WHERE id = ?2",
                            params![pid, id],
                        ) {
                            warnings.push(format!("failed to save apple_music_id for '{title}': {e}"));
                        }
                    }
                    imported += 1;
                    success = true;
                    break;
                }
                Err(e) => {
                    let is_retryable = matches!(&e, MuError::AppleScript(msg) if msg.contains("(-54)"));
                    last_err = Some(e);
                    if !is_retryable {
                        break;
                    }
                }
            }
        }
        if !success {
            failed += 1;
            if let Some(e) = last_err {
                warnings.push(format!("failed to import '{title}': {e}"));
            }
        }
    }

    (imported, skipped, failed, warnings)
}

pub fn handle_sync(db_path: &Path) -> Result<()> {
    let conn = db::open(db_path)?;

    // Sync favorites
    let loved_ids = music::get_loved_track_ids()?;
    let loved_set: std::collections::HashSet<&str> =
        loved_ids.iter().map(String::as_str).collect();

    let mut stmt = conn.prepare(
        "SELECT id, apple_music_id, COALESCE(favorite, 0) FROM tracks WHERE apple_music_id IS NOT NULL",
    )?;
    let tracks: Vec<(i64, String, bool)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(MuError::from)?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut fav_added = 0i64;
    let mut fav_removed = 0i64;
    for (id, apple_music_id, was_favorite) in &tracks {
        let is_loved = loved_set.contains(apple_music_id.as_str());
        if is_loved && !was_favorite {
            conn.execute("UPDATE tracks SET favorite = 1 WHERE id = ?1", params![id])?;
            fav_added += 1;
        } else if !is_loved && *was_favorite {
            conn.execute("UPDATE tracks SET favorite = 0 WHERE id = ?1", params![id])?;
            fav_removed += 1;
        }
    }

    // Sync play counts
    let play_counts = music::get_play_counts()?;
    let count_map: std::collections::HashMap<&str, i64> =
        play_counts.iter().map(|(id, c)| (id.as_str(), *c)).collect();

    let mut stmt = conn.prepare(
        "SELECT id, apple_music_id FROM tracks WHERE apple_music_id IS NOT NULL",
    )?;
    let id_tracks: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut plays_updated = 0i64;
    for (id, apple_music_id) in &id_tracks {
        if let Some(&count) = count_map.get(apple_music_id.as_str()) {
            let changed = conn.execute(
                "UPDATE tracks SET play_count = ?1 WHERE id = ?2 AND COALESCE(play_count, 0) != ?1",
                params![count, id],
            )?;
            if changed > 0 {
                plays_updated += 1;
            }
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "favorites_added": fav_added,
            "favorites_removed": fav_removed,
            "plays_updated": plays_updated,
        })
    );
    Ok(())
}
