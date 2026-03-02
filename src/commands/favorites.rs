use crate::error::{MuError, Result};
use crate::{db, music};
use clap::Subcommand;
use rusqlite::params;
use std::path::Path;

#[derive(Subcommand)]
pub enum FavAction {
    /// Toggle favorite on a track
    Toggle {
        /// Track ID or title substring
        track: String,
    },
    /// Sync favorites from Apple Music to local DB
    Sync,
    /// List favorite tracks
    List,
}

pub fn handle_fav_action(db_path: &Path, action: FavAction) -> Result<()> {
    let conn = db::open(db_path)?;
    match action {
        FavAction::Toggle { track } => fav_toggle(&conn, &track),
        FavAction::Sync => fav_sync(&conn),
        FavAction::List => fav_list(&conn),
    }
}

fn fav_toggle(conn: &rusqlite::Connection, track: &str) -> Result<()> {
    let (tid, title, _) = db::resolve_track(conn, track).ok_or(MuError::TrackNotFound)?;

    let current: bool = conn
        .query_row(
            "SELECT COALESCE(favorite, 0) FROM tracks WHERE id = ?1",
            params![tid],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    let new_val = !current;
    conn.execute(
        "UPDATE tracks SET favorite = ?1 WHERE id = ?2",
        params![new_val, tid],
    )?;

    // Sync to Apple Music if we have a persistent ID
    if let Some(pid) = db::get_apple_music_id(conn, tid) {
        if let Err(e) = music::set_track_loved(&pid, new_val) {
            eprintln!("Warning: failed to sync favorite to Apple Music: {e}");
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "track_id": tid,
            "title": title,
            "favorite": new_val,
        })
    );
    Ok(())
}

fn fav_sync(conn: &rusqlite::Connection) -> Result<()> {
    let loved_ids = music::get_loved_track_ids()?;
    let loved_set: std::collections::HashSet<&str> =
        loved_ids.iter().map(String::as_str).collect();

    // Get all tracks with apple_music_id
    let mut stmt = conn.prepare(
        "SELECT id, apple_music_id, COALESCE(favorite, 0) FROM tracks WHERE apple_music_id IS NOT NULL",
    )?;
    let tracks: Vec<(i64, String, bool)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(MuError::from)?
        .filter_map(std::result::Result::ok)
        .collect();

    let mut added = 0i64;
    let mut removed = 0i64;

    for (id, apple_music_id, was_favorite) in &tracks {
        let is_loved = loved_set.contains(apple_music_id.as_str());
        if is_loved && !was_favorite {
            conn.execute(
                "UPDATE tracks SET favorite = 1 WHERE id = ?1",
                params![id],
            )?;
            added += 1;
        } else if !is_loved && *was_favorite {
            conn.execute(
                "UPDATE tracks SET favorite = 0 WHERE id = ?1",
                params![id],
            )?;
            removed += 1;
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "favorites_added": added,
            "favorites_removed": removed,
        })
    );
    Ok(())
}

fn fav_list(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, title, artist, album, duration_secs, artwork_path FROM tracks WHERE favorite = 1 ORDER BY id",
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
            }))
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    println!("{}", serde_json::json!({"favorites": rows}));
    Ok(())
}
