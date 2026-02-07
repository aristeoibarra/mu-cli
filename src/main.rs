mod client;
mod daemon;
mod db;
mod downloader;

use clap::{Parser, Subcommand};
use rusqlite::params;

#[derive(Parser)]
#[command(name = "mu", about = "Local music player CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a track
    Add {
        /// Song name or URL
        query: String,
        /// Add directly to a playlist
        #[arg(short, long)]
        playlist: Option<String>,
    },
    /// Start playing tracks in background
    Play {
        /// Playlist name (plays all tracks if omitted)
        playlist: Option<String>,
        /// Shuffle tracks
        #[arg(short, long)]
        shuffle: bool,
    },
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Skip to next track
    Next,
    /// Go to previous track
    Previous,
    /// Stop playback and kill daemon
    Stop,
    /// Show current status
    Status,
    /// List tracks or playlists
    List {
        /// Playlist name to list tracks from
        playlist: Option<String>,
    },
    /// Manage playlists
    Playlist {
        #[command(subcommand)]
        action: PlaylistAction,
    },
    /// Remove a track from library and disk
    Remove {
        /// Track ID or title substring
        track: String,
    },
    /// Run as daemon (internal)
    Daemon,
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
}

fn json_error(msg: &str) -> String {
    serde_json::json!({"error": msg}).to_string()
}

fn main() {
    let cli = Cli::parse();
    let data_dir = db::data_dir();
    let db_path = data_dir.join("mu.db");

    match cli.command {
        Commands::Add { query, playlist } => {
            let conn = db::open(&db_path).expect("db open failed");
            match downloader::download(&query, &conn) {
                Ok(result) => {
                    if let Some(pl_name) = playlist {
                        let pl_id: Option<i64> = conn
                            .query_row(
                                "SELECT id FROM playlists WHERE name = ?1",
                                params![pl_name],
                                |row| row.get(0),
                            )
                            .ok();
                        if let Some(pl_id) = pl_id {
                            let pos: i64 = conn
                                .query_row(
                                    "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
                                    params![pl_id],
                                    |row| row.get(0),
                                )
                                .unwrap_or(1);
                            conn.execute(
                                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                                params![pl_id, result.id, pos],
                            )
                            .ok();
                        }
                    }
                    println!("{}", serde_json::to_string(&result).unwrap());
                }
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Play { playlist, shuffle } => {
            let conn = db::open(&db_path).expect("db open failed");

            let (paths, titles): (Vec<String>, Vec<String>) = if let Some(ref pl_name) = playlist {
                let mut stmt = conn
                    .prepare(
                        "SELECT t.file_path, t.title FROM tracks t
                         JOIN playlist_tracks pt ON pt.track_id = t.id
                         JOIN playlists p ON p.id = pt.playlist_id
                         WHERE p.name = ?1
                         ORDER BY pt.position",
                    )
                    .expect("query failed");
                let rows = stmt
                    .query_map(params![pl_name], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .expect("query failed");
                rows.filter_map(|r| r.ok()).unzip()
            } else {
                let mut stmt = conn
                    .prepare("SELECT file_path, title FROM tracks ORDER BY id")
                    .expect("query failed");
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .expect("query failed");
                rows.filter_map(|r| r.ok()).unzip()
            };

            if paths.is_empty() {
                println!("{}", json_error("no tracks found"));
                std::process::exit(1);
            }

            let (paths, titles) = if shuffle {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut indices: Vec<usize> = (0..paths.len()).collect();
                let mut hasher = DefaultHasher::new();
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    .hash(&mut hasher);
                let seed = hasher.finish();
                let mut rng = seed;
                for i in (1..indices.len()).rev() {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let j = (rng as usize) % (i + 1);
                    indices.swap(i, j);
                }
                let paths: Vec<String> = indices.iter().map(|&i| paths[i].clone()).collect();
                let titles: Vec<String> = indices.iter().map(|&i| titles[i].clone()).collect();
                (paths, titles)
            } else {
                (paths, titles)
            };

            // Start daemon if not running
            if !client::daemon_running() {
                let exe = std::env::current_exe().expect("cannot find self");
                std::process::Command::new(exe)
                    .arg("daemon")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("failed to start daemon");
                for _ in 0..20 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    if client::daemon_running() {
                        break;
                    }
                }
            }

            // Send play command with tracks
            let sock_path = data_dir.join("mu.sock");
            let msg = serde_json::json!({
                "cmd": "play",
                "tracks": paths,
                "titles": titles,
            });
            match std::os::unix::net::UnixStream::connect(&sock_path) {
                Ok(mut stream) => {
                    use std::io::Write;
                    let _ = write!(stream, "{}\n", msg);
                    stream.flush().ok();
                    let mut reader = std::io::BufReader::new(stream);
                    let mut response = String::new();
                    use std::io::BufRead;
                    let _ = reader.read_line(&mut response);
                    println!("{}", response.trim());
                }
                Err(_) => {
                    println!("{}", json_error("could not connect to daemon"));
                    std::process::exit(1);
                }
            }
        }

        Commands::Pause => match client::send_command("pause") {
            Ok(r) => println!("{r}"),
            Err(e) => {
                println!("{}", json_error(&e));
                std::process::exit(1);
            }
        },

        Commands::Resume => match client::send_command("resume") {
            Ok(r) => println!("{r}"),
            Err(e) => {
                println!("{}", json_error(&e));
                std::process::exit(1);
            }
        },

        Commands::Next => match client::send_command("next") {
            Ok(r) => println!("{r}"),
            Err(e) => {
                println!("{}", json_error(&e));
                std::process::exit(1);
            }
        },

        Commands::Previous => match client::send_command("previous") {
            Ok(r) => println!("{r}"),
            Err(e) => {
                println!("{}", json_error(&e));
                std::process::exit(1);
            }
        },

        Commands::Stop => {
            let pid_path = data_dir.join("mu.pid");
            let mut killed = false;

            if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
                if let Ok(pid) = pid_str.trim().parse::<i32>() {
                    unsafe {
                        if libc::kill(pid, libc::SIGTERM) == 0 {
                            killed = true;
                        }
                    }
                }
            }

            if killed {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            let _ = std::fs::remove_file(data_dir.join("mu.sock"));
            let _ = std::fs::remove_file(pid_path);
            println!(r#"{{"ok":true,"action":"stopped"}}"#);
        }

        Commands::Status => match client::send_command("status") {
            Ok(r) => println!("{r}"),
            Err(e) => {
                println!("{}", json_error(&e));
                std::process::exit(1);
            }
        },

        Commands::List { playlist } => {
            let conn = db::open(&db_path).expect("db open failed");
            if let Some(ref pl_name) = playlist {
                let mut stmt = conn
                    .prepare(
                        "SELECT t.id, t.title, t.artist, t.duration_secs FROM tracks t
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
                            "duration": row.get::<_, Option<i64>>(3)?,
                        }))
                    })
                    .expect("query failed")
                    .filter_map(|r| r.ok())
                    .collect();
                println!(
                    "{}",
                    serde_json::json!({"playlist": pl_name, "tracks": rows})
                );
            } else {
                let mut stmt = conn
                    .prepare("SELECT id, title, artist, duration_secs FROM tracks ORDER BY id")
                    .expect("query failed");
                let rows: Vec<serde_json::Value> = stmt
                    .query_map([], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, i64>(0)?,
                            "title": row.get::<_, String>(1)?,
                            "artist": row.get::<_, Option<String>>(2)?,
                            "duration": row.get::<_, Option<i64>>(3)?,
                        }))
                    })
                    .expect("query failed")
                    .filter_map(|r| r.ok())
                    .collect();
                println!("{}", serde_json::json!({"tracks": rows}));
            }
        }

        Commands::Playlist { action } => {
            let conn = db::open(&db_path).expect("db open failed");
            match action {
                PlaylistAction::Create { name } => {
                    match conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name]) {
                        Ok(_) => println!(
                            "{}",
                            serde_json::json!({"ok": true, "playlist": name})
                        ),
                        Err(e) => {
                            println!("{}", json_error(&format!("create failed: {e}")));
                            std::process::exit(1);
                        }
                    }
                }
                PlaylistAction::Add { playlist, track } => {
                    let pl_id: Result<i64, _> = conn.query_row(
                        "SELECT id FROM playlists WHERE name = ?1",
                        params![playlist],
                        |row| row.get(0),
                    );
                    let pl_id = match pl_id {
                        Ok(id) => id,
                        Err(_) => {
                            println!("{}", json_error("playlist not found"));
                            std::process::exit(1);
                        }
                    };
                    let track_id: Option<i64> = track
                        .parse::<i64>()
                        .ok()
                        .and_then(|id| {
                            conn.query_row(
                                "SELECT id FROM tracks WHERE id = ?1",
                                params![id],
                                |row| row.get(0),
                            )
                            .ok()
                        })
                        .or_else(|| {
                            conn.query_row(
                                "SELECT id FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                                params![track],
                                |row| row.get(0),
                            )
                            .ok()
                        });

                    match track_id {
                        Some(tid) => {
                            let pos: i64 = conn
                                .query_row(
                                    "SELECT COALESCE(MAX(position), 0) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
                                    params![pl_id],
                                    |row| row.get(0),
                                )
                                .unwrap_or(1);
                            conn.execute(
                                "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                                params![pl_id, tid, pos],
                            )
                            .ok();
                            println!(
                                "{}",
                                serde_json::json!({"ok": true, "track_id": tid, "playlist": playlist})
                            );
                        }
                        None => {
                            println!("{}", json_error("track not found"));
                            std::process::exit(1);
                        }
                    }
                }
                PlaylistAction::Remove { name } => {
                    conn.execute("DELETE FROM playlists WHERE name = ?1", params![name])
                        .ok();
                    println!("{}", serde_json::json!({"ok": true, "removed": name}));
                }
                PlaylistAction::RemoveTrack { playlist, track } => {
                    let pl_id: Result<i64, _> = conn.query_row(
                        "SELECT id FROM playlists WHERE name = ?1",
                        params![playlist],
                        |row| row.get(0),
                    );
                    let pl_id = match pl_id {
                        Ok(id) => id,
                        Err(_) => {
                            println!("{}", json_error("playlist not found"));
                            std::process::exit(1);
                        }
                    };
                    let track_id: Option<i64> = track
                        .parse::<i64>()
                        .ok()
                        .or_else(|| {
                            conn.query_row(
                                "SELECT t.id FROM tracks t
                                 JOIN playlist_tracks pt ON pt.track_id = t.id
                                 WHERE pt.playlist_id = ?1 AND t.title LIKE '%' || ?2 || '%' LIMIT 1",
                                params![pl_id, track],
                                |row| row.get(0),
                            )
                            .ok()
                        });
                    match track_id {
                        Some(tid) => {
                            conn.execute(
                                "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
                                params![pl_id, tid],
                            )
                            .ok();
                            println!(
                                "{}",
                                serde_json::json!({"ok": true, "removed_track": tid, "playlist": playlist})
                            );
                        }
                        None => {
                            println!("{}", json_error("track not found in playlist"));
                            std::process::exit(1);
                        }
                    }
                }
                PlaylistAction::List => {
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
                        .filter_map(|r| r.ok())
                        .collect();
                    println!("{}", serde_json::json!({"playlists": rows}));
                }
            }
        }

        Commands::Remove { track } => {
            let conn = db::open(&db_path).expect("db open failed");
            let row: Option<(i64, String)> = track
                .parse::<i64>()
                .ok()
                .and_then(|id| {
                    conn.query_row(
                        "SELECT id, file_path FROM tracks WHERE id = ?1",
                        params![id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok()
                })
                .or_else(|| {
                    conn.query_row(
                        "SELECT id, file_path FROM tracks WHERE title LIKE '%' || ?1 || '%' LIMIT 1",
                        params![track],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .ok()
                });
            match row {
                Some((tid, file_path)) => {
                    conn.execute("DELETE FROM playlist_tracks WHERE track_id = ?1", params![tid]).ok();
                    conn.execute("DELETE FROM tracks WHERE id = ?1", params![tid]).ok();
                    let _ = std::fs::remove_file(&file_path);
                    println!(
                        "{}",
                        serde_json::json!({"ok": true, "removed_id": tid, "file_deleted": file_path})
                    );
                }
                None => {
                    println!("{}", json_error("track not found"));
                    std::process::exit(1);
                }
            }
        }

        Commands::Daemon => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime failed");
            rt.block_on(async {
                if let Err(e) = daemon::run().await {
                    eprintln!("daemon error: {e}");
                    std::process::exit(1);
                }
            });
        }
    }
}
