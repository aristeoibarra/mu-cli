use crate::db;
use rusqlite::Connection;
use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
pub struct AddResult {
    pub added: bool,
    pub id: i64,
    pub title: String,
    pub file: String,
}

pub fn download(query: &str, conn: &Connection) -> Result<AddResult, String> {
    let tracks_dir = db::data_dir().join("tracks");

    // Use yt-dlp to search and download
    let is_url = query.starts_with("http://") || query.starts_with("https://");
    let search_query = if is_url {
        query.to_string()
    } else {
        format!("ytsearch1:{query}")
    };

    // First get metadata
    let meta = Command::new("yt-dlp")
        .args([
            "--print", "%(title)s\n%(uploader)s\n%(duration)s",
            "--no-download",
            &search_query,
        ])
        .output()
        .map_err(|e| format!("yt-dlp not found: {e}"))?;

    if !meta.status.success() {
        return Err(format!(
            "yt-dlp search failed: {}",
            String::from_utf8_lossy(&meta.stderr)
        ));
    }

    let meta_str = String::from_utf8_lossy(&meta.stdout);
    let lines: Vec<&str> = meta_str.trim().lines().collect();
    if lines.len() < 3 {
        return Err("could not parse metadata".into());
    }

    let title = lines[0].to_string();
    let artist = lines[1].to_string();
    let duration: i64 = lines[2].parse().unwrap_or(0);

    // Download audio
    let output_template = tracks_dir
        .join("%(title)s.%(ext)s")
        .to_string_lossy()
        .to_string();

    let dl = Command::new("yt-dlp")
        .args([
            "-x",
            "--audio-format", "mp3",
            "--audio-quality", "128K",
            "--print", "after_move:filepath",
            "-o", &output_template,
            &search_query,
        ])
        .output()
        .map_err(|e| format!("download failed: {e}"))?;

    if !dl.status.success() {
        return Err(format!(
            "download failed: {}",
            String::from_utf8_lossy(&dl.stderr)
        ));
    }

    let file_path = String::from_utf8_lossy(&dl.stdout).trim().to_string();

    conn.execute(
        "INSERT INTO tracks (title, artist, duration_secs, file_path, source_url) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![title, artist, duration, file_path, query],
    )
    .map_err(|e| format!("db insert failed: {e}"))?;

    let id = conn.last_insert_rowid();

    Ok(AddResult {
        added: true,
        id,
        title,
        file: file_path,
    })
}
