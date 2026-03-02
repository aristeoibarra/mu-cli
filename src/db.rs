use rusqlite::{params, Connection, Result};
use std::path::Path;

/// Track row from database: (id, title, artist, album, `file_path`)
pub type TrackRow = (i64, String, Option<String>, Option<String>, String);

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tracks (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            artist TEXT,
            album TEXT,
            duration_secs INTEGER,
            file_path TEXT NOT NULL,
            artwork_path TEXT,
            source_url TEXT,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS playlists (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE NOT NULL
        );

        CREATE TABLE IF NOT EXISTS playlist_tracks (
            playlist_id INTEGER REFERENCES playlists(id) ON DELETE CASCADE,
            track_id INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
            position INTEGER,
            PRIMARY KEY (playlist_id, track_id)
        );
        ",
    )?;

    // Add artwork_path column if it doesn't exist (migration)
    let has_artwork: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='artwork_path'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if !has_artwork {
        conn.execute("ALTER TABLE tracks ADD COLUMN artwork_path TEXT", [])?;
    }

    // Add album column if it doesn't exist (migration)
    let has_album: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='album'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if !has_album {
        conn.execute("ALTER TABLE tracks ADD COLUMN album TEXT", [])?;
    }

    // Add apple_music_id column if it doesn't exist (migration)
    let has_apple_music_id: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='apple_music_id'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if !has_apple_music_id {
        conn.execute("ALTER TABLE tracks ADD COLUMN apple_music_id TEXT", [])?;
    }

    // Add favorite column if it doesn't exist (migration)
    let has_favorite: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='favorite'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if !has_favorite {
        conn.execute(
            "ALTER TABLE tracks ADD COLUMN favorite BOOLEAN DEFAULT 0",
            [],
        )?;
    }

    // Drop old podcast tables if they exist
    conn.execute_batch(
        "
        DROP TABLE IF EXISTS episodes;
        DROP TABLE IF EXISTS podcasts;
        DROP TABLE IF EXISTS config;
        ",
    )?;

    // Unique index on source_url to prevent duplicate downloads
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_tracks_source_url ON tracks(source_url) WHERE source_url IS NOT NULL;",
    )?;

    Ok(())
}

pub fn data_dir() -> crate::error::Result<std::path::PathBuf> {
    let base = dirs::data_local_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local/share")));
    let dir = base
        .ok_or_else(|| {
            crate::error::MuError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not determine data directory",
            ))
        })?
        .join("mu");
    std::fs::create_dir_all(&dir)?;
    std::fs::create_dir_all(dir.join("tracks"))?;
    std::fs::create_dir_all(dir.join("artwork"))?;
    Ok(dir)
}

pub fn find_track_by_url(conn: &Connection, url: &str) -> Option<(i64, String)> {
    conn.query_row(
        "SELECT id, title FROM tracks WHERE source_url = ?1",
        params![url],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .ok()
}

// --- Track & playlist resolution helpers ---

pub fn resolve_track(conn: &Connection, track: &str) -> Option<(i64, String, Option<String>)> {
    track
        .parse::<i64>()
        .ok()
        .and_then(|id| {
            conn.query_row(
                "SELECT id, title, file_path FROM tracks WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()
        })
        .or_else(|| {
            conn.query_row(
                "SELECT id, title, file_path FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                params![track],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()
        })
}

pub fn resolve_track_for_remove(
    conn: &Connection,
    track: &str,
) -> Option<(i64, String, Option<String>, Option<String>)> {
    track
        .parse::<i64>()
        .ok()
        .and_then(|id| {
            conn.query_row(
                "SELECT id, file_path, artwork_path, apple_music_id FROM tracks WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok()
        })
        .or_else(|| {
            conn.query_row(
                "SELECT id, file_path, artwork_path, apple_music_id FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                params![track],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok()
        })
}

pub fn resolve_track_id(conn: &Connection, track: &str) -> Option<i64> {
    track.parse::<i64>().ok().or_else(|| {
        conn.query_row(
            "SELECT id FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
            params![track],
            |row| row.get(0),
        )
        .ok()
    })
}

pub fn resolve_playlist_id(conn: &Connection, name: &str) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM playlists WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )
    .ok()
}

pub fn next_playlist_position(conn: &Connection, playlist_id: i64) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |row| row.get(0),
    )
    .unwrap_or(1)
}

pub fn resolve_track_row(conn: &Connection, track: &str) -> Option<TrackRow> {
    track
        .parse::<i64>()
        .ok()
        .and_then(|id| {
            conn.query_row(
                "SELECT id, title, artist, album, file_path FROM tracks WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .ok()
        })
        .or_else(|| {
            conn.query_row(
                "SELECT id, title, artist, album, file_path FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                params![track],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .ok()
        })
}

pub fn get_apple_music_id(conn: &Connection, track_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT apple_music_id FROM tracks WHERE id = ?1",
        params![track_id],
        |row| row.get(0),
    )
    .ok()
    .flatten()
}

pub fn all_track_rows(conn: &Connection) -> crate::error::Result<Vec<TrackRow>> {
    let mut stmt =
        conn.prepare("SELECT id, title, artist, album, file_path FROM tracks ORDER BY id")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}
