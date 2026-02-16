mod commands;
mod db;
mod downloader;
mod music;
mod notifications;
mod podcast;

use clap::{Parser, Subcommand};
use rusqlite::params;

#[derive(Parser)]
#[command(name = "mu", about = "Local music player CLI - Downloads and imports to Apple Music")]
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
    /// Manage podcasts
    Podcast {
        #[command(subcommand)]
        action: PodcastCommand,
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
    /// List all playlists
    List,
}

#[derive(Subcommand)]
enum PodcastCommand {
    /// Subscribe to a podcast feed
    Subscribe {
        /// RSS feed URL
        feed_url: String,
        /// Maximum episodes to keep locally
        #[arg(long, default_value = "5")]
        max: i64,
        /// Don't auto-download episodes
        #[arg(long)]
        no_auto: bool,
    },
    /// List subscribed podcasts
    List,
    /// List episodes of a podcast
    Episodes {
        /// Podcast name or ID
        podcast: String,
        /// Show only unplayed episodes
        #[arg(long)]
        unplayed_only: bool,
    },
    /// Update podcast feeds (check for new episodes)
    Update {
        /// Specific podcast to update (all if omitted)
        podcast: Option<String>,
    },
    /// Configure podcast settings
    Config {
        /// Podcast name
        podcast: String,
        /// Enable/disable auto-download
        #[arg(long)]
        auto_download: Option<bool>,
        /// Enable/disable notifications
        #[arg(long)]
        notify: Option<bool>,
        /// Set max episodes to keep
        #[arg(long)]
        max: Option<i64>,
    },
    /// Unsubscribe from a podcast
    Unsubscribe {
        /// Podcast name
        podcast: String,
        /// Also delete downloaded files
        #[arg(long)]
        delete_files: bool,
    },
    /// Cleanup old completed episodes
    Cleanup {
        /// Show what would be deleted without deleting
        #[arg(long)]
        dry_run: bool,
        /// Force cleanup even if not Sunday
        #[arg(long)]
        force: bool,
    },
    /// Show podcast storage usage and limits
    Storage,
    /// Set max podcast storage limit
    SetMaxStorage {
        /// Maximum storage in GB
        size_gb: f64,
    },
    /// Show listening statistics
    Stats {
        /// Specific podcast (all podcasts if omitted)
        podcast: Option<String>,
    },
    /// Import all episodes to Apple Music library
    Import {
        /// Podcast name
        podcast: String,
    },
}

fn json_error(msg: &str) -> String {
    serde_json::json!({"error": msg}).to_string()
}

fn json_ok(msg: &str) -> String {
    serde_json::json!({"ok": true, "message": msg}).to_string()
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
                    // Import to Apple Music library
                    let file_path = std::path::Path::new(&result.file);
                    if let Err(e) = music::import_to_library(file_path) {
                        eprintln!("Warning: Failed to import to Apple Music: {}", e);
                    }
                    
                    if let Some(pl_name) = playlist {
                        // Add to local playlist in DB
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
                        
                        // Also create/update playlist in Apple Music
                        let _ = music::add_to_playlist(file_path, &pl_name);
                    }
                    println!("{}", serde_json::to_string(&result).unwrap());
                }
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Play { playlist, track } => {
            let result = if let Some(track_name) = track {
                music::play_track(&track_name)
            } else {
                music::play_playlist(playlist.as_deref())
            };
            
            match result {
                Ok(_) => println!("{}", json_ok("Playing in Apple Music")),
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Pause => {
            match music::pause() {
                Ok(_) => println!("{}", json_ok("Paused")),
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Resume => {
            match music::resume() {
                Ok(_) => println!("{}", json_ok("Resumed")),
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Next => {
            match music::next_track() {
                Ok(_) => println!("{}", json_ok("Next track")),
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Previous => {
            match music::previous_track() {
                Ok(_) => println!("{}", json_ok("Previous track")),
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Stop => {
            match music::stop() {
                Ok(_) => println!("{}", json_ok("Stopped")),
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

        Commands::Status => {
            match music::get_status() {
                Ok(status) => {
                    println!("{}", serde_json::json!({
                        "track": status.track,
                        "state": status.state,
                        "position_secs": status.position_secs,
                        "duration_secs": status.duration_secs,
                    }));
                }
                Err(e) => {
                    println!("{}", json_error(&e));
                    std::process::exit(1);
                }
            }
        }

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
                    // Create in local DB
                    match conn.execute("INSERT INTO playlists (name) VALUES (?1)", params![name]) {
                        Ok(_) => {
                            // Also create in Apple Music
                            let _ = music::create_playlist(&name);
                            println!("{}", serde_json::json!({"ok": true, "playlist": name}));
                        }
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
                    
                    // Find track
                    let track_row: Option<(i64, String)> = track
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

                    match track_row {
                        Some((tid, file_path)) => {
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
                            
                            // Also add to Apple Music playlist
                            let _ = music::add_to_playlist(std::path::Path::new(&file_path), &playlist);
                            
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

        Commands::Podcast { action } => {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime failed");
            rt.block_on(async {
                let result = match action {
                    PodcastCommand::Subscribe { feed_url, max, no_auto } => {
                        commands::podcast_commands::subscribe(feed_url, Some(max), !no_auto).await
                    }
                    PodcastCommand::List => {
                        commands::podcast_commands::list()
                    }
                    PodcastCommand::Episodes { podcast, unplayed_only } => {
                        commands::podcast_commands::list_episodes(podcast, unplayed_only)
                    }
                    PodcastCommand::Update { podcast } => {
                        commands::podcast_commands::update(podcast).await
                    }
                    PodcastCommand::Config { podcast, auto_download, notify, max } => {
                        commands::podcast_commands::config(podcast, auto_download, notify, max)
                    }
                    PodcastCommand::Unsubscribe { podcast, delete_files } => {
                        commands::podcast_commands::unsubscribe(podcast, delete_files).await
                    }
                    PodcastCommand::Cleanup { dry_run, force } => {
                        commands::podcast_commands::cleanup(dry_run, force).await
                    }
                    PodcastCommand::Storage => {
                        let conn = db::open(&db_path).expect("db open failed");
                        let current = db::calculate_podcast_storage(&conn).unwrap_or(0);
                        let max = db::get_max_storage(&conn).unwrap_or(5368709120);
                        let current_gb = current as f64 / 1024.0 / 1024.0 / 1024.0;
                        let max_gb = max as f64 / 1024.0 / 1024.0 / 1024.0;
                        let percent = if max > 0 { current as f64 / max as f64 * 100.0 } else { 0.0 };
                        
                        Ok(serde_json::json!({
                            "current_gb": format!("{:.2}", current_gb),
                            "max_gb": format!("{:.2}", max_gb),
                            "used_percent": format!("{:.1}", percent),
                            "current_bytes": current,
                            "max_bytes": max,
                        }).to_string())
                    }
                    PodcastCommand::SetMaxStorage { size_gb } => {
                        let conn = db::open(&db_path).expect("db open failed");
                        let bytes = (size_gb * 1024.0 * 1024.0 * 1024.0) as i64;
                        match db::set_max_storage(&conn, bytes) {
                            Ok(_) => Ok(serde_json::json!({
                                "ok": true,
                                "max_storage_gb": size_gb,
                                "max_storage_bytes": bytes,
                            }).to_string()),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                    PodcastCommand::Stats { podcast } => {
                        commands::podcast_commands::stats(podcast)
                    }
                    PodcastCommand::Import { podcast } => {
                        // Import all downloaded episodes to Apple Music
                        let conn = db::open(&db_path).expect("db open failed");
                        let podcast_id = match db::find_podcast_id(&conn, &podcast) {
                            Ok(id) => id,
                            Err(_) => return,
                        };
                        
                        let mut stmt = match conn.prepare("SELECT file_path, title FROM episodes WHERE podcast_id = ?1 AND is_downloaded = 1") {
                            Ok(s) => s,
                            Err(e) => {
                                println!("{}", json_error(&e.to_string()));
                                return;
                            }
                        };
                        
                        let episodes: Vec<(String, String)> = stmt
                            .query_map([podcast_id], |row| Ok((row.get(0)?, row.get(1)?)))
                            .unwrap_or_else(|_| panic!("query failed"))
                            .filter_map(|r| r.ok())
                            .collect();
                        
                        let mut imported = 0;
                        let mut failed = 0;
                        
                        for (file_path, _title) in &episodes {
                            let path = std::path::Path::new(file_path);
                            if music::import_to_library(path).is_ok() {
                                imported += 1;
                            } else {
                                failed += 1;
                            }
                        }
                        
                        Ok(serde_json::json!({
                            "ok": true,
                            "podcast": podcast,
                            "imported": imported,
                            "failed": failed,
                            "total": episodes.len(),
                        }).to_string())
                    }
                };
                
                match result {
                    Ok(output) => println!("{}", output),
                    Err(e) => {
                        println!("{}", json_error(&e));
                        std::process::exit(1);
                    }
                }
            });
        }
    }
}
