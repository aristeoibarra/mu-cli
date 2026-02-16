use crate::db;
use rodio::{Decoder, OutputStream, Sink};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::UnixListener;

#[derive(Debug, Deserialize)]
struct DaemonCmd {
    cmd: String,
    #[serde(default)]
    tracks: Vec<String>,
    #[serde(default)]
    titles: Vec<String>,
    #[serde(default)]
    episode_ids: Vec<i64>,
    #[serde(default)]
    speed: Option<f32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Status {
    pub playing: bool,
    pub paused: bool,
    pub track: Option<String>,
    pub track_index: usize,
    pub total_tracks: usize,
}

enum PlayerMsg {
    Play(Vec<PathBuf>, Vec<String>, Vec<i64>),  // paths, titles, episode_ids
    Pause,
    Resume,
    Next,
    Previous,
    Stop,
    SetSpeed(f32),
    Status(std::sync::mpsc::Sender<Status>),
}

fn play_track(sink: &Sink, path: &PathBuf) -> bool {
    if let Ok(file) = File::open(path) {
        if let Ok(source) = Decoder::new(BufReader::new(file)) {
            sink.append(source);
            sink.play();
            return true;
        }
    }
    false
}

/// Save episode playback progress
fn save_episode_progress(episode_id: i64, progress: f32) {
    use rusqlite::params;
    if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
        let _ = conn.execute(
            "UPDATE episodes SET playback_progress = ?1, playback_status = 'playing' WHERE id = ?2",
            params![progress, episode_id],
        );
    }
}

/// Mark episode as completed
fn mark_episode_completed(episode_id: i64) {
    use rusqlite::params;
    if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
        let _ = conn.execute(
            "UPDATE episodes SET 
                playback_status = 'completed', 
                playback_progress = 1.0,
                completed_at = CURRENT_TIMESTAMP 
             WHERE id = ?1",
            params![episode_id],
        );
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = db::data_dir();
    let sock_path = data_dir.join("mu.sock");
    let pid_path = data_dir.join("mu.pid");

    let _ = std::fs::remove_file(&sock_path);
    std::fs::write(&pid_path, std::process::id().to_string())?;

    let listener = UnixListener::bind(&sock_path)?;
    let (tx, rx) = mpsc::channel::<PlayerMsg>();

    // Player thread — audio must stay on one OS thread
    std::thread::spawn(move || {
        let (_stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();
        let mut tracks: Vec<PathBuf> = Vec::new();
        let mut titles: Vec<String> = Vec::new();
        let mut episode_ids: Vec<i64> = Vec::new();
        let mut current_index: usize = 0;
        let mut active = false;
        let mut progress_counter = 0u32; // Counter for saving progress (every 30 seconds = 300 iterations)

        loop {
            // Check if current track finished and advance to next
            if active && sink.empty() {
                // Mark current episode as completed if it's a podcast
                if let Some(&episode_id) = episode_ids.get(current_index) {
                    if episode_id > 0 {
                        mark_episode_completed(episode_id);
                    }
                }
                
                let next = current_index + 1;
                if let Some(path) = tracks.get(next) {
                    current_index = next;
                    active = play_track(&sink, path);
                    progress_counter = 0; // Reset counter for new track
                } else {
                    active = false;
                }
            }
            
            // Save progress every 30 seconds (300 iterations × 100ms = 30s)
            if active && !sink.is_paused() {
                progress_counter += 1;
                if progress_counter >= 300 {
                    if let Some(&episode_id) = episode_ids.get(current_index) {
                        if episode_id > 0 {
                            // Calculate approximate progress (we don't have exact position, so we use time elapsed)
                            save_episode_progress(episode_id, 0.5); // Placeholder, will improve later
                        }
                    }
                    progress_counter = 0;
                }
            }

            // Process incoming commands with timeout to keep checking playback state
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(msg) => match msg {
                    PlayerMsg::Play(new_tracks, new_titles, new_episode_ids) => {
                        sink.stop();
                        tracks = new_tracks;
                        titles = new_titles;
                        episode_ids = new_episode_ids;
                        current_index = 0;
                        active = false;
                        progress_counter = 0;
                        if let Some(path) = tracks.get(current_index) {
                            active = play_track(&sink, path);
                        }
                    }
                    PlayerMsg::Pause => sink.pause(),
                    PlayerMsg::Resume => sink.play(),
                    PlayerMsg::Next => {
                        sink.stop();
                        let next = current_index + 1;
                        if let Some(path) = tracks.get(next) {
                            current_index = next;
                            active = play_track(&sink, path);
                        } else {
                            active = false;
                        }
                    }
                    PlayerMsg::Previous => {
                        sink.stop();
                        if current_index > 0 {
                            current_index -= 1;
                            if let Some(path) = tracks.get(current_index) {
                                active = play_track(&sink, path);
                            }
                        } else {
                            // Si ya estamos en el primero, reinicia el track actual
                            if let Some(path) = tracks.get(current_index) {
                                active = play_track(&sink, path);
                            }
                        }
                    }
                    PlayerMsg::Stop => {
                        sink.stop();
                        active = false;
                        tracks.clear();
                        titles.clear();
                        episode_ids.clear();
                        progress_counter = 0;
                    }
                    PlayerMsg::SetSpeed(speed) => {
                        sink.set_speed(speed);
                    }
                    PlayerMsg::Status(reply) => {
                        let _ = reply.send(Status {
                            playing: active && !sink.is_paused(),
                            paused: sink.is_paused(),
                            track: titles.get(current_index).cloned(),
                            track_index: current_index,
                            total_tracks: tracks.len(),
                        });
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Normal timeout, continue loop to check playback state
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Channel closed, exit thread
                    break;
                }
            }
        }
    });

    // Spawn podcast update loop (runs every hour)
    tokio::spawn(async {
        podcast_update_loop().await;
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let tx = tx.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = TokioBufReader::new(reader);
            let mut line = String::new();
            if reader.read_line(&mut line).await.is_ok() {
                let response = handle_command(line.trim(), &tx);
                let _ = writer.write_all(response.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
            }
        });
    }
}

/// Cleanup oldest completed episodes to free up space
async fn cleanup_oldest_completed() {
    use rusqlite::params;
    
    // Find oldest completed episodes (not the latest per podcast) - collect and close connection
    let to_delete: Vec<(i64, String)> = {
        if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
            let query = "
                WITH latest_per_podcast AS (
                    SELECT podcast_id, MAX(pub_date) as latest_date
                    FROM episodes
                    GROUP BY podcast_id
                )
                SELECT e.id, e.file_path
                FROM episodes e
                LEFT JOIN latest_per_podcast lp ON lp.podcast_id = e.podcast_id 
                    AND e.pub_date = lp.latest_date
                WHERE e.playback_status = 'completed'
                  AND e.is_downloaded = 1
                  AND lp.latest_date IS NULL
                ORDER BY e.completed_at ASC
                LIMIT 10
            ";
            
            if let Ok(mut stmt) = conn.prepare(query) {
                stmt.query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    }; // Connection closed here
    
    // Now delete files and update DB with fresh connections
    for (id, file_path) in to_delete {
        if tokio::fs::remove_file(&file_path).await.is_ok() {
            if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
                let _ = conn.execute(
                    "UPDATE episodes SET is_downloaded = 0, file_path = NULL, marked_for_deletion = 1 WHERE id = ?1",
                    params![id],
                );
            }
        }
    }
}

/// Background loop that checks for new podcast episodes every hour
async fn podcast_update_loop() {
    use crate::podcast;
    use rusqlite::params;
    
    loop {
        // Wait 1 hour
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
        
        // Get list of podcasts to update (collect data and close connection)
        let podcasts: Vec<(i64, String, String, bool, bool)> = {
            if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT id, title, feed_url, auto_download, notify_new_episodes FROM podcasts WHERE auto_download = 1"
                ) {
                    stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, bool>(3)?,
                            row.get::<_, bool>(4)?,
                        ))
                    })
                    .ok()
                    .map(|rows| rows.flatten().collect())
                    .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        }; // Connection is dropped here
        
        // Check storage limit before downloading
        let (current_storage, max_storage) = {
            if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
                let current = db::calculate_podcast_storage(&conn).unwrap_or(0);
                let max = db::get_max_storage(&conn).unwrap_or(5368709120); // 5GB default
                (current, max)
            } else {
                (0, 5368709120)
            }
        };
        
        // Now process podcasts with async operations
        for (podcast_id, title, feed_url, auto_download, notify_enabled) in podcasts {
            // Check if we're over storage limit
            if current_storage >= max_storage {
                // Try to cleanup to make space
                cleanup_oldest_completed().await;
                // Skip downloading new episodes if still over limit
                continue;
            }
            
            // Fetch and parse feed
            if let Ok((_feed, episodes)) = podcast::fetch_and_parse(&feed_url).await {
                // Open fresh connection for this podcast
                if let Ok(conn) = db::open(&db::data_dir().join("mu.db")) {
                    for episode in episodes {
                        // Check if episode already exists
                        let exists: bool = conn
                            .query_row(
                                "SELECT 1 FROM episodes WHERE guid = ?1",
                                params![episode.guid],
                                |_| Ok(true),
                            )
                            .unwrap_or(false);
                        
                        if !exists && auto_download {
                            // Download episode (async operation, connection not held)
                            if let Ok((file_path, file_size)) = 
                                podcast::download_episode(&episode, &title).await 
                            {
                                // Insert episode with fresh connection
                                if let Ok(conn2) = db::open(&db::data_dir().join("mu.db")) {
                                    let _ = podcast::insert_episode(
                                        &conn2,
                                        podcast_id,
                                        &episode,
                                        Some(&file_path),
                                        file_size,
                                    );
                                }
                                
                                // Send notification if enabled
                                if notify_enabled {
                                    use crate::notifications;
                                    let _ = notifications::notify_new_episode(&title, &episode.title);
                                }
                            }
                        }
                    }
                    
                    // Update last_checked timestamp
                    let _ = conn.execute(
                        "UPDATE podcasts SET last_checked = CURRENT_TIMESTAMP WHERE id = ?1",
                        params![podcast_id],
                    );
                }
            }
        }
    }
}

fn handle_command(line: &str, tx: &mpsc::Sender<PlayerMsg>) -> String {
    let cmd: DaemonCmd = match serde_json::from_str(line) {
        Ok(c) => c,
        Err(e) => return format!(r#"{{"error":"invalid command: {e}"}}"#),
    };

    match cmd.cmd.as_str() {
        "play" => {
            let paths: Vec<PathBuf> = cmd.tracks.into_iter().map(PathBuf::from).collect();
            let titles = cmd.titles;
            let episode_ids = cmd.episode_ids;
            if paths.is_empty() {
                return r#"{"error":"no tracks provided"}"#.to_string();
            }
            let count = paths.len();
            let _ = tx.send(PlayerMsg::Play(paths, titles, episode_ids));
            format!(r#"{{"ok":true,"action":"playing","tracks":{count}}}"#)
        }
        "pause" => {
            let _ = tx.send(PlayerMsg::Pause);
            r#"{"ok":true,"action":"paused"}"#.to_string()
        }
        "resume" => {
            let _ = tx.send(PlayerMsg::Resume);
            r#"{"ok":true,"action":"resumed"}"#.to_string()
        }
        "next" => {
            let _ = tx.send(PlayerMsg::Next);
            r#"{"ok":true,"action":"next"}"#.to_string()
        }
        "previous" => {
            let _ = tx.send(PlayerMsg::Previous);
            r#"{"ok":true,"action":"previous"}"#.to_string()
        }
        "stop" => {
            let _ = tx.send(PlayerMsg::Stop);
            r#"{"ok":true,"action":"stopped"}"#.to_string()
        }
        "status" => {
            let (otx, orx) = std::sync::mpsc::channel();
            let _ = tx.send(PlayerMsg::Status(otx));
            match orx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(status) => serde_json::to_string(&status).unwrap_or_default(),
                Err(_) => r#"{"error":"player not responding"}"#.to_string(),
            }
        }
        "set_speed" => {
            let speed = cmd.speed.unwrap_or(1.0).clamp(0.5, 3.0);
            let _ = tx.send(PlayerMsg::SetSpeed(speed));
            format!(r#"{{"ok":true,"speed":{}}}"#, speed)
        }
        _ => format!(r#"{{"error":"unknown command: {}"}}"#, cmd.cmd),
    }
}
