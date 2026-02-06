use crate::db;
use rodio::{Decoder, OutputStream, Sink};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

#[derive(Debug, Deserialize)]
struct DaemonCmd {
    cmd: String,
    #[serde(default)]
    tracks: Vec<String>,
    #[serde(default)]
    titles: Vec<String>,
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
    Play(Vec<PathBuf>, Vec<String>),
    Pause,
    Resume,
    Skip,
    Stop,
    Status(tokio::sync::oneshot::Sender<Status>),
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

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = db::data_dir();
    let sock_path = data_dir.join("mu.sock");
    let pid_path = data_dir.join("mu.pid");

    let _ = std::fs::remove_file(&sock_path);
    std::fs::write(&pid_path, std::process::id().to_string())?;

    let listener = UnixListener::bind(&sock_path)?;
    let (tx, mut rx) = mpsc::channel::<PlayerMsg>(32);

    // Player thread — audio must stay on one OS thread
    std::thread::spawn(move || {
        let (_stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();
        let mut tracks: Vec<PathBuf> = Vec::new();
        let mut titles: Vec<String> = Vec::new();
        let mut current_index: usize = 0;
        let mut active = false;

        while let Some(msg) = rx.blocking_recv() {
            match msg {
                PlayerMsg::Play(new_tracks, new_titles) => {
                    sink.stop();
                    tracks = new_tracks;
                    titles = new_titles;
                    current_index = 0;
                    active = false;
                    if let Some(path) = tracks.get(current_index) {
                        active = play_track(&sink, path);
                    }
                }
                PlayerMsg::Pause => sink.pause(),
                PlayerMsg::Resume => sink.play(),
                PlayerMsg::Skip => {
                    sink.stop();
                    current_index += 1;
                    if let Some(path) = tracks.get(current_index) {
                        active = play_track(&sink, path);
                    } else {
                        active = false;
                    }
                }
                PlayerMsg::Stop => {
                    sink.stop();
                    active = false;
                    tracks.clear();
                    titles.clear();
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
            }
        }
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let tx = tx.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = TokioBufReader::new(reader);
            let mut line = String::new();
            if reader.read_line(&mut line).await.is_ok() {
                let response = handle_command(line.trim(), &tx).await;
                let _ = writer.write_all(response.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
            }
        });
    }
}

async fn handle_command(line: &str, tx: &mpsc::Sender<PlayerMsg>) -> String {
    let cmd: DaemonCmd = match serde_json::from_str(line) {
        Ok(c) => c,
        Err(e) => return format!(r#"{{"error":"invalid command: {e}"}}"#),
    };

    match cmd.cmd.as_str() {
        "play" => {
            let paths: Vec<PathBuf> = cmd.tracks.into_iter().map(PathBuf::from).collect();
            let titles = cmd.titles;
            if paths.is_empty() {
                return r#"{"error":"no tracks provided"}"#.to_string();
            }
            let count = paths.len();
            let _ = tx.send(PlayerMsg::Play(paths, titles)).await;
            format!(r#"{{"ok":true,"action":"playing","tracks":{count}}}"#)
        }
        "pause" => {
            let _ = tx.send(PlayerMsg::Pause).await;
            r#"{"ok":true,"action":"paused"}"#.to_string()
        }
        "resume" => {
            let _ = tx.send(PlayerMsg::Resume).await;
            r#"{"ok":true,"action":"resumed"}"#.to_string()
        }
        "skip" => {
            let _ = tx.send(PlayerMsg::Skip).await;
            r#"{"ok":true,"action":"skipped"}"#.to_string()
        }
        "stop" => {
            let _ = tx.send(PlayerMsg::Stop).await;
            r#"{"ok":true,"action":"stopped"}"#.to_string()
        }
        "status" => {
            let (otx, orx) = tokio::sync::oneshot::channel();
            let _ = tx.send(PlayerMsg::Status(otx)).await;
            match orx.await {
                Ok(status) => serde_json::to_string(&status).unwrap_or_default(),
                Err(_) => r#"{"error":"player not responding"}"#.to_string(),
            }
        }
        _ => format!(r#"{{"error":"unknown command: {}"}}"#, cmd.cmd),
    }
}
