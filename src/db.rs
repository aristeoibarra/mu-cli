use rusqlite::{Connection, Result};
use std::path::Path;

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

    // Drop old podcast tables if they exist
    conn.execute_batch(
        "
        DROP TABLE IF EXISTS episodes;
        DROP TABLE IF EXISTS podcasts;
        DROP TABLE IF EXISTS config;
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
    std::fs::create_dir_all(dir.join("artwork")).ok();
    dir
}
