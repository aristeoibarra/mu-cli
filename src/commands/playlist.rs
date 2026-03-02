use crate::error::{MuError, Result};
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
    conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name])?;
    let _ = music::create_playlist(name);
    println!("{}", serde_json::json!({"ok": true, "playlist": name}));
    Ok(())
}

fn playlist_add(conn: &Connection, playlist: &str, track: &str) -> Result<()> {
    let pl_id = db::resolve_playlist_id(conn, playlist).ok_or(MuError::PlaylistNotFound)?;
    let (tid, title, _) = db::resolve_track(conn, track).ok_or(MuError::TrackNotFound)?;

    let pos = db::next_playlist_position(conn, pl_id);
    conn.execute(
        "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
        params![pl_id, tid, pos],
    )?;

    // Use persistent ID if available, fallback to name matching
    if let Some(pid) = db::get_apple_music_id(conn, tid) {
        let _ = music::add_track_to_playlist_by_id(&pid, playlist);
    } else {
        let _ = music::add_track_to_playlist(&title, playlist);
    }

    println!(
        "{}",
        serde_json::json!({"ok": true, "track_id": tid, "playlist": playlist})
    );
    Ok(())
}

fn playlist_remove(conn: &Connection, name: &str) -> Result<()> {
    conn.execute("DELETE FROM playlists WHERE name = ?1", params![name])?;
    let _ = music::delete_playlist(name);
    println!("{}", serde_json::json!({"ok": true, "removed": name}));
    Ok(())
}

fn playlist_remove_track(conn: &Connection, playlist: &str, track: &str) -> Result<()> {
    let pl_id = db::resolve_playlist_id(conn, playlist).ok_or(MuError::PlaylistNotFound)?;
    let tid = db::resolve_track_id(conn, track).ok_or(MuError::TrackNotFound)?;

    // Remove from Apple Music playlist
    if let Some(pid) = db::get_apple_music_id(conn, tid) {
        let _ = music::remove_track_from_playlist(&pid, playlist);
    }

    conn.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
        params![pl_id, tid],
    )?;
    println!(
        "{}",
        serde_json::json!({ "ok": true, "removed_track_id": tid, "from_playlist": playlist })
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
        .filter_map(std::result::Result::ok)
        .collect();
    println!("{}", serde_json::json!({"playlists": rows}));
    Ok(())
}

fn playlist_sync(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT p.name, t.title, t.apple_music_id FROM playlists p
         JOIN playlist_tracks pt ON pt.playlist_id = p.id
         JOIN tracks t ON t.id = pt.track_id
         ORDER BY p.name, pt.position",
    )?;

    let rows: Vec<(String, String, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(MuError::from)?
        .filter_map(std::result::Result::ok)
        .collect();

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
        .filter_map(std::result::Result::ok)
        .collect();
    for name in &all_playlists {
        playlist_tracks.entry(name.clone()).or_default();
    }

    let mut tracks_added = 0;
    let mut tracks_removed = 0;

    for (playlist_name, local_tracks) in &playlist_tracks {
        let _ = music::create_playlist(playlist_name);

        // Collect local persistent IDs for this playlist
        let local_ids: std::collections::HashSet<String> = local_tracks
            .iter()
            .filter_map(|(_, pid)| pid.clone())
            .collect();

        // Add missing tracks to Apple Music
        for (title, apple_music_id) in local_tracks {
            if let Some(pid) = apple_music_id {
                if music::add_track_to_playlist_by_id(pid, playlist_name).is_ok() {
                    tracks_added += 1;
                }
            } else if music::add_track_to_playlist(title, playlist_name).is_ok() {
                tracks_added += 1;
            }
        }

        // Remove extra tracks from Apple Music playlist (bidirectional sync)
        if let Ok(am_ids) = music::get_playlist_track_ids(playlist_name) {
            for am_id in &am_ids {
                if !local_ids.contains(am_id)
                    && music::remove_track_from_playlist(am_id, playlist_name).is_ok()
                {
                    tracks_removed += 1;
                }
            }
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "playlists_synced": playlist_tracks.len(),
            "tracks_added": tracks_added,
            "tracks_removed": tracks_removed,
        })
    );
    Ok(())
}
