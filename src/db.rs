use rusqlite::{Connection, Result};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    // Create config table for global settings
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;

    // Set default max storage (5GB = 5368709120 bytes)
    conn.execute(
        "INSERT OR IGNORE INTO config (key, value) VALUES ('max_podcast_storage_bytes', '5368709120')",
        [],
    )?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tracks (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            artist TEXT,
            duration_secs INTEGER,
            file_path TEXT NOT NULL,
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

        CREATE TABLE IF NOT EXISTS podcasts (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            author TEXT,
            feed_url TEXT UNIQUE NOT NULL,
            description TEXT,
            artwork_url TEXT,
            last_checked DATETIME,
            auto_download BOOLEAN DEFAULT 1,
            notify_new_episodes BOOLEAN DEFAULT 1,
            max_episodes INTEGER DEFAULT 5,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS episodes (
            id INTEGER PRIMARY KEY,
            podcast_id INTEGER NOT NULL REFERENCES podcasts(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            description TEXT,
            pub_date DATETIME NOT NULL,
            duration_secs INTEGER,
            file_path TEXT,
            file_size_bytes INTEGER DEFAULT 0,
            source_url TEXT NOT NULL,
            guid TEXT UNIQUE NOT NULL,
            playback_status TEXT DEFAULT 'new',
            playback_progress REAL DEFAULT 0.0,
            is_downloaded BOOLEAN DEFAULT 0,
            completed_at DATETIME,
            marked_for_deletion BOOLEAN DEFAULT 0,
            added_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_episodes_podcast 
            ON episodes(podcast_id, pub_date DESC);
        CREATE INDEX IF NOT EXISTS idx_episodes_status 
            ON episodes(playback_status, is_downloaded);
        CREATE INDEX IF NOT EXISTS idx_episodes_cleanup 
            ON episodes(marked_for_deletion, completed_at);
        ",
    )?;
    Ok(())
}

pub fn data_dir() -> std::path::PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
        .join("mu");
    std::fs::create_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join("tracks")).ok();
    std::fs::create_dir_all(dir.join("podcasts")).ok();
    dir
}

/// Calculate total storage used by podcasts in bytes
pub fn calculate_podcast_storage(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(file_size_bytes), 0) FROM episodes WHERE is_downloaded = 1",
        [],
        |row| row.get(0),
    )
}

/// Get podcast ID by title or feed URL
pub fn find_podcast_id(conn: &Connection, identifier: &str) -> Result<i64> {
    // Try by title first
    if let Ok(id) = conn.query_row(
        "SELECT id FROM podcasts WHERE title = ?1",
        [identifier],
        |row| row.get(0),
    ) {
        return Ok(id);
    }

    // Try by feed URL
    conn.query_row(
        "SELECT id FROM podcasts WHERE feed_url = ?1",
        [identifier],
        |row| row.get(0),
    )
}

/// Get max podcast storage limit in bytes
pub fn get_max_storage(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT value FROM config WHERE key = 'max_podcast_storage_bytes'",
        [],
        |row| {
            let value_str: String = row.get(0)?;
            Ok(value_str.parse::<i64>().unwrap_or(5368709120)) // 5GB default
        },
    )
}

/// Set max podcast storage limit in bytes
pub fn set_max_storage(conn: &Connection, bytes: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('max_podcast_storage_bytes', ?1)",
        [bytes.to_string()],
    )?;
    Ok(())
}
