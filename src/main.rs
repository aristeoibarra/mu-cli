mod commands;
mod db;
mod downloader;
mod music;

use clap::{Parser, Subcommand};
use rusqlite::{params, Connection};

/// Track row from database: (id, title, artist, album, `file_path`)
type TrackRow = (i64, String, Option<String>, Option<String>, String);

#[derive(Parser)]
#[command(
    name = "mu",
    about = "Local music player CLI - Downloads and imports to Apple Music"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a track and import to Apple Music
    Add {
        /// Song name or URL
        query: String,
        /// Add directly to a playlist
        #[arg(short, long)]
        playlist: Option<String>,
    },
    /// Play tracks in Apple Music
    Play {
        /// Playlist name (plays all library if omitted)
        playlist: Option<String>,
        /// Track name to play
        #[arg(short, long)]
        track: Option<String>,
    },
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Skip to next track
    Next,
    /// Go to previous track
    Previous,
    /// Stop playback
    Stop,
    /// Show current status
    Status,
    /// List tracks in local database
    List {
        /// Playlist name to list tracks from
        playlist: Option<String>,
    },
    /// Manage playlists
    Playlist {
        #[command(subcommand)]
        action: PlaylistAction,
    },
    /// Remove a track from library
    Remove {
        /// Track ID or title substring
        track: String,
    },
    /// Migrate existing tracks to Apple Music
    Migrate {
        /// Only show what would be migrated
        #[arg(long)]
        dry_run: bool,
    },
    /// Show Apple Music library info
    Info,
    /// Re-import track with updated metadata
    Reimport {
        /// Track ID or title substring (all if omitted)
        track: Option<String>,
    },
}

#[derive(Subcommand)]
enum PlaylistAction {
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

fn json_error(msg: &str) -> String {
    serde_json::json!({"error": msg}).to_string()
}

fn json_ok(msg: &str) -> String {
    serde_json::json!({"ok": true, "message": msg}).to_string()
}

fn fail(msg: &str) -> ! {
    println!("{}", json_error(msg));
    std::process::exit(1);
}

fn handle_simple_command(result: Result<(), String>, ok_msg: &str) {
    match result {
        Ok(()) => println!("{}", json_ok(ok_msg)),
        Err(e) => fail(&e),
    }
}

// --- Track resolution helpers ---

fn resolve_track(conn: &Connection, track: &str) -> Option<(i64, String, Option<String>)> {
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

fn resolve_track_for_remove(conn: &Connection, track: &str) -> Option<(i64, String, Option<String>)> {
    track
        .parse::<i64>()
        .ok()
        .and_then(|id| {
            conn.query_row(
                "SELECT id, file_path, artwork_path FROM tracks WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()
        })
        .or_else(|| {
            conn.query_row(
                "SELECT id, file_path, artwork_path FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                params![track],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()
        })
}

fn resolve_track_id(conn: &Connection, track: &str) -> Option<i64> {
    track.parse::<i64>().ok().or_else(|| {
        conn.query_row(
            "SELECT id FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
            params![track],
            |row| row.get(0),
        )
        .ok()
    })
}

fn resolve_playlist_id(conn: &Connection, name: &str) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM playlists WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )
    .ok()
}

fn next_playlist_position(conn: &Connection, playlist_id: i64) -> i64 {
    conn.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
        params![playlist_id],
        |row| row.get(0),
    )
    .unwrap_or(1)
}

fn resolve_track_row(conn: &Connection, track: &str) -> Option<TrackRow> {
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

fn all_track_rows(conn: &Connection) -> Vec<TrackRow> {
    let mut stmt = conn
        .prepare("SELECT id, title, artist, album, file_path FROM tracks ORDER BY id")
        .expect("query failed");
    stmt.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })
    .expect("query failed")
    .filter_map(Result::ok)
    .collect()
}

// --- Command handlers ---

fn main() {
    let cli = Cli::parse();
    let data_dir = db::data_dir();
    let db_path = data_dir.join("mu.db");

    match cli.command {
        Commands::Add { query, playlist } => handle_add(&db_path, &query, playlist),
        Commands::Play { playlist, track } => {
            let result = if let Some(ref name) = track {
                music::play_track(name)
            } else {
                music::play_playlist(playlist.as_deref())
            };
            handle_simple_command(result, "Playing in Apple Music");
        }
        Commands::Pause => handle_simple_command(music::pause(), "Paused"),
        Commands::Resume => handle_simple_command(music::resume(), "Resumed"),
        Commands::Next => handle_simple_command(music::next_track(), "Next track"),
        Commands::Previous => handle_simple_command(music::previous_track(), "Previous track"),
        Commands::Stop => handle_simple_command(music::stop(), "Stopped"),
        Commands::Status => handle_status(),
        Commands::List { playlist } => handle_list(&db_path, playlist.as_deref()),
        Commands::Playlist { action } => {
            let conn = db::open(&db_path).expect("db open failed");
            handle_playlist_action(&conn, action);
        }
        Commands::Remove { track } => handle_remove(&db_path, &track),
        Commands::Migrate { dry_run } => handle_migrate(&db_path, dry_run),
        Commands::Info => handle_info(),
        Commands::Reimport { track } => handle_reimport(&db_path, track.as_deref()),
    }
}

fn handle_add(db_path: &std::path::Path, query: &str, playlist: Option<String>) {
    let conn = db::open(db_path).expect("db open failed");
    match downloader::download(query, &conn) {
        Ok(result) => {
            let file_path = std::path::Path::new(&result.file);
            if let Err(e) = music::import_with_metadata(
                file_path,
                result.artist.as_deref(),
                result.album.as_deref(),
                Some("Music"),
            ) {
                eprintln!("Warning: Failed to import to Apple Music: {e}");
            }

            if let Some(pl_name) = playlist {
                if let Some(pl_id) = resolve_playlist_id(&conn, &pl_name) {
                    let pos = next_playlist_position(&conn, pl_id);
                    conn.execute(
                        "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                        params![pl_id, result.id, pos],
                    )
                    .ok();
                }
                let _ = music::add_track_to_playlist(&result.title, &pl_name);
            }
            println!("{}", serde_json::to_string(&result).unwrap());
        }
        Err(e) => fail(&e),
    }
}

fn handle_status() {
    match music::get_status() {
        Ok(status) => {
            println!(
                "{}",
                serde_json::json!({
                    "track": status.track,
                    "artist": status.artist,
                    "album": status.album,
                    "state": status.state,
                    "position_secs": status.position_secs,
                    "duration_secs": status.duration_secs,
                })
            );
        }
        Err(e) => fail(&e),
    }
}

fn handle_list(db_path: &std::path::Path, playlist: Option<&str>) {
    let conn = db::open(db_path).expect("db open failed");
    if let Some(pl_name) = playlist {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.title, t.artist, t.album, t.duration_secs, t.artwork_path FROM tracks t
                 JOIN playlist_tracks pt ON pt.track_id = t.id
                 JOIN playlists p ON p.id = pt.playlist_id
                 WHERE p.name = ?1
                 ORDER BY pt.position",
            )
            .expect("query failed");
        let rows: Vec<serde_json::Value> = stmt
            .query_map(params![pl_name], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, i64>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "artist": row.get::<_, Option<String>>(2)?,
                    "album": row.get::<_, Option<String>>(3)?,
                    "duration": row.get::<_, Option<i64>>(4)?,
                    "artwork": row.get::<_, Option<String>>(5)?,
                }))
            })
            .expect("query failed")
            .filter_map(Result::ok)
            .collect();
        println!(
            "{}",
            serde_json::json!({"playlist": pl_name, "tracks": rows})
        );
    } else {
        let mut stmt = conn
            .prepare("SELECT id, title, artist, album, duration_secs, artwork_path FROM tracks ORDER BY id")
            .expect("query failed");
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
            })
            .expect("query failed")
            .filter_map(Result::ok)
            .collect();
        println!("{}", serde_json::json!({"tracks": rows}));
    }
}

fn handle_remove(db_path: &std::path::Path, track: &str) {
    let conn = db::open(db_path).expect("db open failed");
    let Some((tid, file_path, artwork_path)) = resolve_track_for_remove(&conn, track) else {
        fail("track not found");
    };

    conn.execute("DELETE FROM playlist_tracks WHERE track_id = ?1", params![tid])
        .ok();
    conn.execute("DELETE FROM tracks WHERE id = ?1", params![tid])
        .ok();
    let _ = std::fs::remove_file(&file_path);
    if let Some(art) = &artwork_path {
        let _ = std::fs::remove_file(art);
    }
    println!(
        "{}",
        serde_json::json!({"ok": true, "removed_id": tid, "file_deleted": file_path})
    );
}

fn handle_migrate(db_path: &std::path::Path, dry_run: bool) {
    let conn = db::open(db_path).expect("db open failed");
    let tracks = all_track_rows(&conn);

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
        return;
    }

    let (imported, skipped, failed) = import_tracks(&tracks);
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
}

fn handle_info() {
    match music::get_library_stats() {
        Ok(stats) => {
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
        }
        Err(e) => fail(&e),
    }
}

fn handle_reimport(db_path: &std::path::Path, track: Option<&str>) {
    let conn = db::open(db_path).expect("db open failed");

    let tracks = if let Some(t) = track {
        let Some(row) = resolve_track_row(&conn, t) else {
            fail("track not found");
        };
        vec![row]
    } else {
        all_track_rows(&conn)
    };

    let mut reimported = 0;
    let mut failed = 0;

    for (_id, title, artist, album, file_path) in &tracks {
        let path = std::path::Path::new(file_path);

        if !path.exists() {
            failed += 1;
            eprintln!("File not found: {file_path}");
            continue;
        }

        match music::import_with_metadata(path, artist.as_deref(), album.as_deref(), Some("Music"))
        {
            Ok(_) => {
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
}

fn handle_playlist_action(conn: &Connection, action: PlaylistAction) {
    match action {
        PlaylistAction::Create { name } => {
            match conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name]) {
                Ok(_) => {
                    let _ = music::create_playlist(&name);
                    println!("{}", serde_json::json!({"ok": true, "playlist": name}));
                }
                Err(e) => fail(&format!("create failed: {e}")),
            }
        }
        PlaylistAction::Add { playlist, track } => playlist_add(conn, &playlist, &track),
        PlaylistAction::Remove { name } => {
            conn.execute("DELETE FROM playlists WHERE name = ?1", params![name])
                .ok();
            let _ = music::delete_playlist(&name);
            println!("{}", serde_json::json!({"ok": true, "removed": name}));
        }
        PlaylistAction::RemoveTrack { playlist, track } => {
            playlist_remove_track(conn, &playlist, &track);
        }
        PlaylistAction::List => playlist_list(conn),
        PlaylistAction::Sync => playlist_sync(conn),
    }
}

fn playlist_add(conn: &Connection, playlist: &str, track: &str) {
    let Some(pl_id) = resolve_playlist_id(conn, playlist) else {
        fail("playlist not found");
    };
    let Some((tid, title, _)) = resolve_track(conn, track) else {
        fail("track not found");
    };

    let pos = next_playlist_position(conn, pl_id);
    conn.execute(
        "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
        params![pl_id, tid, pos],
    )
    .ok();
    let _ = music::add_track_to_playlist(&title, playlist);
    println!(
        "{}",
        serde_json::json!({"ok": true, "track_id": tid, "playlist": playlist})
    );
}

fn playlist_remove_track(conn: &Connection, playlist: &str, track: &str) {
    let Some(pl_id) = resolve_playlist_id(conn, playlist) else {
        fail("playlist not found");
    };
    let Some(tid) = resolve_track_id(conn, track) else {
        fail("track not found");
    };

    conn.execute(
        "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
        params![pl_id, tid],
    )
    .ok();
    println!(
        "{}",
        serde_json::json!({ "ok": true, "removed_track_id": tid, "from_playlist": playlist })
    );
}

fn playlist_list(conn: &Connection) {
    let mut stmt = conn
        .prepare(
            "SELECT p.name, COUNT(pt.track_id) FROM playlists p
             LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
             GROUP BY p.id ORDER BY p.name",
        )
        .expect("query failed");
    let rows: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "name": row.get::<_, String>(0)?,
                "tracks": row.get::<_, i64>(1)?,
            }))
        })
        .expect("query failed")
        .filter_map(Result::ok)
        .collect();
    println!("{}", serde_json::json!({"playlists": rows}));
}

fn playlist_sync(conn: &Connection) {
    let mut stmt = conn
        .prepare(
            "SELECT p.name, t.title FROM playlists p
             JOIN playlist_tracks pt ON pt.playlist_id = p.id
             JOIN tracks t ON t.id = pt.track_id
             ORDER BY p.name, pt.position",
        )
        .expect("query failed");

    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query failed")
        .filter_map(Result::ok)
        .collect();

    let mut synced_playlists = std::collections::HashSet::new();
    let mut tracks_added = 0;

    for (playlist_name, track_title) in &rows {
        if !synced_playlists.contains(playlist_name) {
            let _ = music::create_playlist(playlist_name);
            synced_playlists.insert(playlist_name.clone());
        }
        if music::add_track_to_playlist(track_title, playlist_name).is_ok() {
            tracks_added += 1;
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "playlists_synced": synced_playlists.len(),
            "tracks_added": tracks_added,
        })
    );
}

fn import_tracks(tracks: &[TrackRow]) -> (i64, i64, i64) {
    let mut imported = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for (_id, title, artist, album, file_path) in tracks {
        let path = std::path::Path::new(file_path);

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
            Ok(_) => {
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
