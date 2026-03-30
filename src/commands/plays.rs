use crate::error::Result;
use crate::{db, music};
use clap::Subcommand;
use rusqlite::params;
use std::path::Path;

#[derive(Clone, Copy, Subcommand)]
pub enum PlaysAction {
    /// Sync play counts from Apple Music to local DB
    Sync,
    /// List tracks by play count (descending)
    List,
}

pub fn handle_plays_action(db_path: &Path, action: PlaysAction) -> Result<()> {
    let conn = db::open(db_path)?;
    match action {
        PlaysAction::Sync => plays_sync(&conn),
        PlaysAction::List => plays_list(&conn),
    }
}

fn plays_sync(conn: &rusqlite::Connection) -> Result<()> {
    let play_counts = music::get_play_counts()?;
    let count_map: std::collections::HashMap<&str, i64> = play_counts
        .iter()
        .map(|(id, c)| (id.as_str(), *c))
        .collect();

    let mut stmt =
        conn.prepare("SELECT id, apple_music_id FROM tracks WHERE apple_music_id IS NOT NULL")?;
    let tracks: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut updated = 0i64;
    for (id, apple_music_id) in &tracks {
        if let Some(&count) = count_map.get(apple_music_id.as_str()) {
            let changed = conn.execute(
                "UPDATE tracks SET play_count = ?1 WHERE id = ?2 AND COALESCE(play_count, 0) != ?1",
                params![count, id],
            )?;
            if changed > 0 {
                updated += 1;
            }
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "tracks_checked": tracks.len(),
            "tracks_updated": updated,
        })
    );
    Ok(())
}

fn plays_list(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, title, artist, album, COALESCE(play_count, 0) FROM tracks WHERE COALESCE(play_count, 0) > 0 ORDER BY play_count DESC",
    )?;
    let rows: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "title": row.get::<_, String>(1)?,
                "artist": row.get::<_, Option<String>>(2)?,
                "album": row.get::<_, Option<String>>(3)?,
                "play_count": row.get::<_, i64>(4)?,
            }))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    println!("{}", serde_json::json!({"tracks": rows}));
    Ok(())
}
