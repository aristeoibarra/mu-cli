use crate::error::{json_result, MuError, Result};
use crate::{db, music};
use clap::Subcommand;
use rusqlite::{params, Connection};

#[derive(Subcommand)]
pub enum PlaylistAction {
    /// Create a new playlist
    Create { name: String },
    /// Add a track to a playlist
    Add {
        /// Playlist name
        playlist: String,
        /// Track ID or title substring
        track: String,
    },
    /// Remove a playlist
    Remove { name: String },
    /// Remove a track from a playlist
    RemoveTrack {
        /// Playlist name
        playlist: String,
        /// Track ID or title substring
        track: String,
    },
    /// List all playlists
    List,
    /// Sync playlists with Apple Music
    Sync,
}

/// Validate playlist name: reject empty, too long, control chars, backslashes.
fn validate_playlist_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(MuError::Validation("playlist name cannot be empty".into()));
    }
    if trimmed.len() > 255 {
        return Err(MuError::Validation(
            "playlist name too long (max 255 chars)".into(),
        ));
    }
    if trimmed.chars().any(|c| c.is_control() || c == '\\') {
        return Err(MuError::Validation(
            "playlist name contains invalid characters".into(),
        ));
    }
    Ok(())
}

pub fn handle_playlist_action(db_path: &std::path::Path, action: PlaylistAction) -> Result<()> {
    let conn = db::open(db_path)?;
    match action {
        PlaylistAction::Create { name } => playlist_create(&conn, &name),
        PlaylistAction::Add { playlist, track } => playlist_add(&conn, &playlist, &track),
        PlaylistAction::Remove { name } => playlist_remove(&conn, &name),
        PlaylistAction::RemoveTrack { playlist, track } => {
            playlist_remove_track(&conn, &playlist, &track)
        }
        PlaylistAction::List => playlist_list(&conn),
        PlaylistAction::Sync => playlist_sync(&conn),
    }
}

fn playlist_create(conn: &Connection, name: &str) -> Result<()> {
    validate_playlist_name(name)?;
    let mut warnings = Vec::new();
    conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name])?;
    if let Err(e) = music::create_playlist(name) {
        warnings.push(format!("failed to create Apple Music playlist: {e}"));
    }
    println!(
        "{}",
        json_result(serde_json::json!({"ok": true, "playlist": name}), &warnings,)
    );
    Ok(())
}

fn playlist_add(conn: &Connection, playlist: &str, track: &str) -> Result<()> {
    validate_playlist_name(playlist)?;
    let mut warnings = Vec::new();
    let pl_id = db::resolve_playlist_id(conn, playlist).ok_or(MuError::PlaylistNotFound)?;
    let (tid, title, _) = db::resolve_track(conn, track).ok_or(MuError::TrackNotFound)?;

    let pos = db::next_playlist_position(conn, pl_id);
    conn.execute(
        "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
        params![pl_id, tid, pos],
    )?;

    if let Err(e) = music::add_track_to_playlist_smart(
        db::get_apple_music_id(conn, tid).as_deref(),
        &title,
        playlist,
    ) {
        warnings.push(format!("failed to add track to Apple Music playlist: {e}"));
    }

    println!(
        "{}",
        json_result(
            serde_json::json!({"ok": true, "track_id": tid, "playlist": playlist}),
            &warnings,
        )
    );
    Ok(())
}

fn playlist_remove(conn: &Connection, name: &str) -> Result<()> {
    let mut warnings = Vec::new();
    conn.execute("DELETE FROM playlists WHERE name = ?1", params![name])?;
    if conn.changes() == 0 {
        return Err(MuError::PlaylistNotFound);
    }
    if let Err(e) = music::delete_playlist(name) {
        warnings.push(format!("failed to delete Apple Music playlist: {e}"));
    }
    println!(
        "{}",
        json_result(serde_json::json!({"ok": true, "removed": name}), &warnings,)
    );
    Ok(())
}

fn playlist_remove_track(conn: &Connection, playlist: &str, track: &str) -> Result<()> {
    let mut warnings = Vec::new();
    let pl_id = db::resolve_playlist_id(conn, playlist).ok_or(MuError::PlaylistNotFound)?;
    let tid = db::resolve_track_id(conn, track).ok_or(MuError::TrackNotFound)?;

    // Remove from Apple Music playlist
    if let Some(pid) = db::get_apple_music_id(conn, tid) {
        if let Err(e) = music::remove_track_from_playlist(&pid, playlist) {
            warnings.push(format!(
                "failed to remove track from Apple Music playlist: {e}"
            ));
        }
    }

    conn.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
        params![pl_id, tid],
    )?;
    println!(
        "{}",
        json_result(
            serde_json::json!({ "ok": true, "removed_track_id": tid, "from_playlist": playlist }),
            &warnings,
        )
    );
    Ok(())
}

fn playlist_list(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT p.name, COUNT(pt.track_id) FROM playlists p
         LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
         GROUP BY p.id ORDER BY p.name",
    )?;
    let rows: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "name": row.get::<_, String>(0)?,
                "tracks": row.get::<_, i64>(1)?,
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    println!("{}", serde_json::json!({"playlists": rows}));
    Ok(())
}

fn playlist_sync(conn: &Connection) -> Result<()> {
    let mut warnings = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT p.name, t.title, t.apple_music_id FROM playlists p
         JOIN playlist_tracks pt ON pt.playlist_id = p.id
         JOIN tracks t ON t.id = pt.track_id
         ORDER BY p.name, pt.position",
    )?;

    let rows: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(MuError::from)?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Group tracks by playlist
    let mut playlist_tracks: std::collections::HashMap<String, Vec<(String, Option<String>)>> =
        std::collections::HashMap::new();
    for (playlist_name, title, apple_music_id) in &rows {
        playlist_tracks
            .entry(playlist_name.clone())
            .or_default()
            .push((title.clone(), apple_music_id.clone()));
    }

    // Also include empty playlists
    let mut pl_stmt = conn.prepare("SELECT name FROM playlists")?;
    let all_playlists: Vec<String> = pl_stmt
        .query_map([], |row| row.get(0))
        .map_err(MuError::from)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for name in &all_playlists {
        playlist_tracks.entry(name.clone()).or_default();
    }

    let mut tracks_added = 0;
    let mut tracks_removed = 0;

    for (playlist_name, local_tracks) in &playlist_tracks {
        let _ = music::create_playlist(playlist_name);

        // Get existing track IDs in Apple Music playlist to avoid duplicates
        let existing_ids: std::collections::HashSet<String> =
            music::get_playlist_track_ids(playlist_name)
                .unwrap_or_default()
                .into_iter()
                .collect();

        // Collect local persistent IDs for this playlist
        let local_ids: std::collections::HashSet<String> = local_tracks
            .iter()
            .filter_map(|(_, pid)| pid.clone())
            .collect();

        // Add missing tracks to Apple Music (skip if already present)
        for (title, apple_music_id) in local_tracks {
            if let Some(pid) = apple_music_id {
                if !existing_ids.contains(pid) {
                    if let Err(e) = music::add_track_to_playlist_by_id(pid, playlist_name) {
                        warnings.push(format!(
                            "failed to add track to playlist '{playlist_name}': {e}"
                        ));
                    } else {
                        tracks_added += 1;
                    }
                }
            } else if let Err(e) = music::add_track_to_playlist(title, playlist_name) {
                warnings.push(format!(
                    "failed to add track '{title}' to playlist '{playlist_name}': {e}"
                ));
            } else {
                tracks_added += 1;
            }
        }

        // Remove extra tracks and duplicates from Apple Music playlist
        if let Ok(am_ids) = music::get_playlist_track_ids(playlist_name) {
            let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for am_id in &am_ids {
                let count = seen.entry(am_id).or_insert(0);
                *count += 1;
                // Remove if: not in local DB, OR is a duplicate (seen more than once)
                if !local_ids.contains(am_id) || *count > 1 {
                    if let Err(e) = music::remove_track_from_playlist(am_id, playlist_name) {
                        warnings.push(format!(
                            "failed to remove extra track from playlist '{playlist_name}': {e}"
                        ));
                    } else {
                        tracks_removed += 1;
                    }
                }
            }
        }
    }

    println!(
        "{}",
        json_result(
            serde_json::json!({
                "ok": true,
                "playlists_synced": playlist_tracks.len(),
                "tracks_added": tracks_added,
                "tracks_removed": tracks_removed,
            }),
            &warnings,
        )
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_playlist_name_normal() {
        assert!(validate_playlist_name("My Playlist").is_ok());
    }

    #[test]
    fn validate_playlist_name_empty() {
        assert!(validate_playlist_name("").is_err());
        assert!(validate_playlist_name("   ").is_err());
    }

    #[test]
    fn validate_playlist_name_control_chars() {
        assert!(validate_playlist_name("bad\x00name").is_err());
        assert!(validate_playlist_name("bad\nnewline").is_err());
        assert!(validate_playlist_name("back\\slash").is_err());
    }

    #[test]
    fn validate_playlist_name_too_long() {
        let long_name = "a".repeat(256);
        assert!(validate_playlist_name(&long_name).is_err());
        let ok_name = "a".repeat(255);
        assert!(validate_playlist_name(&ok_name).is_ok());
    }
}
