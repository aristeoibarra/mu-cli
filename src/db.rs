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
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version == 0 {
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
                added_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                apple_music_id TEXT,
                favorite BOOLEAN DEFAULT 0,
                play_count INTEGER DEFAULT 0
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

        // Add columns if they don't exist (for pre-existing databases)
        let columns = ["artwork_path", "album", "apple_music_id"];
        for col in columns {
            let has_col: bool = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='{col}'"),
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !has_col {
                let type_str = "TEXT";
                conn.execute(
                    &format!("ALTER TABLE tracks ADD COLUMN {col} {type_str}"),
                    [],
                )?;
            }
        }

        // favorite column with default
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

        // play_count column with default
        let has_play_count: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tracks') WHERE name='play_count'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);
        if !has_play_count {
            conn.execute(
                "ALTER TABLE tracks ADD COLUMN play_count INTEGER DEFAULT 0",
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

        conn.execute_batch("PRAGMA user_version = 1;")?;
    }

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

/// Resolve a track by ID or title substring. Returns (id, title, `file_path`).
pub fn resolve_track(conn: &Connection, track: &str) -> Option<(i64, String, Option<String>)> {
    if let Ok(id) = track.parse::<i64>() {
        if let Ok(row) = conn.query_row(
            "SELECT id, title, file_path FROM tracks WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ) {
            return Some(row);
        }
    }
    conn.query_row(
        "SELECT id, title, file_path FROM tracks WHERE title = ?1",
        params![track],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .or_else(|_| {
        conn.query_row(
            "SELECT id, title, file_path FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
            params![track],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
    })
    .ok()
}

pub fn resolve_track_for_remove(
    conn: &Connection,
    track: &str,
) -> Option<(i64, String, Option<String>, Option<String>)> {
    if let Ok(id) = track.parse::<i64>() {
        if let Ok(row) = conn.query_row(
            "SELECT id, file_path, artwork_path, apple_music_id FROM tracks WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ) {
            return Some(row);
        }
    }
    conn.query_row(
        "SELECT id, file_path, artwork_path, apple_music_id FROM tracks WHERE title = ?1",
        params![track],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .or_else(|_| {
        conn.query_row(
            "SELECT id, file_path, artwork_path, apple_music_id FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
            params![track],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
    })
    .ok()
}

pub fn resolve_track_id(conn: &Connection, track: &str) -> Option<i64> {
    if let Ok(id) = track.parse::<i64>() {
        return Some(id);
    }
    conn.query_row(
        "SELECT id FROM tracks WHERE title = ?1",
        params![track],
        |row| row.get(0),
    )
    .or_else(|_| {
        conn.query_row(
            "SELECT id FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
            params![track],
            |row| row.get(0),
        )
    })
    .ok()
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
    if let Ok(id) = track.parse::<i64>() {
        if let Ok(row) = conn.query_row(
            "SELECT id, title, artist, album, file_path FROM tracks WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        ) {
            return Some(row);
        }
    }
    conn.query_row(
        "SELECT id, title, artist, album, file_path FROM tracks WHERE title = ?1",
        params![track],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )
    .or_else(|_| {
        conn.query_row(
            "SELECT id, title, artist, album, file_path FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
            params![track],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
    })
    .ok()
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
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn resolve_track_by_id() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO tracks (id, title, file_path) VALUES (1, 'Test Song', '/tmp/test.m4a')",
            [],
        )
        .unwrap();
        let result = resolve_track(&conn, "1");
        assert!(result.is_some());
        let (id, title, _) = result.unwrap();
        assert_eq!(id, 1);
        assert_eq!(title, "Test Song");
    }

    #[test]
    fn resolve_track_by_title() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO tracks (id, title, file_path) VALUES (1, 'My Great Song', '/tmp/test.m4a')",
            [],
        )
        .unwrap();
        let result = resolve_track(&conn, "Great Song");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, 1);
    }

    #[test]
    fn resolve_track_not_found() {
        let conn = test_db();
        assert!(resolve_track(&conn, "nonexistent").is_none());
    }

    #[test]
    fn find_track_by_url_works() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO tracks (id, title, file_path, source_url) VALUES (1, 'Song', '/tmp/s.m4a', 'https://youtube.com/watch?v=abc')",
            [],
        )
        .unwrap();
        let result = find_track_by_url(&conn, "https://youtube.com/watch?v=abc");
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "Song");
    }

    #[test]
    fn migrate_idempotent() {
        let conn = test_db();
        // Running migrate again should not fail
        assert!(migrate(&conn).is_ok());
    }
}
