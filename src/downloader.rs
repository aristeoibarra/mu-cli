use crate::db;
use rusqlite::Connection;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
pub struct AddResult {
    pub added: bool,
    pub id: i64,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub file: String,
    pub artwork: Option<String>,
}

/// Parse artist from video title or uploader
/// Handles formats like "Artist - Song", "Artist - Song (Official Video)", etc.
fn parse_artist_title(raw_title: &str, uploader: &str) -> (String, Option<String>) {
    // Common patterns: "Artist - Title", "Artist - Title (Official)", "Artist | Title"
    let separators = [" - ", " – ", " — ", " | "];

    for sep in separators {
        if let Some(idx) = raw_title.find(sep) {
            let artist = raw_title[..idx].trim().to_string();
            let title = raw_title[idx + sep.len()..].trim().to_string();
            // Clean up title (remove common suffixes)
            let clean_title = clean_title(&title);
            return (clean_title, Some(artist));
        }
    }

    // No separator found - use uploader as artist if it looks like an artist name
    let uploader_clean = uploader
        .replace(" - Topic", "")
        .replace("VEVO", "")
        .replace("Official", "")
        .trim()
        .to_string();

    if !uploader_clean.is_empty()
        && !uploader_clean.to_lowercase().contains("youtube")
        && !uploader_clean.to_lowercase().contains("channel")
    {
        return (clean_title(raw_title), Some(uploader_clean));
    }

    (clean_title(raw_title), None)
}

/// Remove common video suffixes from title
fn clean_title(title: &str) -> String {
    let suffixes = [
        "(Official Video)",
        "(Official Music Video)",
        "(Official Audio)",
        "(Official Lyric Video)",
        "(Lyric Video)",
        "(Audio)",
        "(Music Video)",
        "[Official Video]",
        "[Official Audio]",
        "[Official Music Video]",
        "(Full Album)",
        "[Full Album]",
        "(HD)",
        "(HQ)",
        "(4K)",
        "| Official Video",
        "| Official Audio",
    ];

    let mut result = title.to_string();
    for suffix in suffixes {
        if let Some(idx) = result.to_lowercase().find(&suffix.to_lowercase()) {
            result = result[..idx].trim().to_string();
        }
    }
    result
}

/// Parse album from title if it contains "Full Album" or similar
fn parse_album(title: &str, artist: Option<&str>) -> Option<String> {
    let lower = title.to_lowercase();

    // Check if this is a full album
    if lower.contains("full album") || lower.contains("complete album") {
        // Try to extract album name
        // Format: "Artist - Album Name (Full Album)" or "Album Name - Full Album"
        if let Some(artist_name) = artist {
            // Remove artist prefix if present
            let without_artist = title
                .replace(&format!("{artist_name} - "), "")
                .replace(&format!("{artist_name} – "), "");
            return Some(clean_title(&without_artist));
        }
        return Some(clean_title(title));
    }

    None
}

pub fn download(query: &str, conn: &Connection) -> Result<AddResult, String> {
    let data_dir = db::data_dir();
    let tracks_dir = data_dir.join("tracks");
    let artwork_dir = data_dir.join("artwork");

    // Use yt-dlp to search and download
    let is_url = query.starts_with("http://") || query.starts_with("https://");
    let search_query = if is_url {
        query.to_string()
    } else {
        format!("ytsearch1:{query}")
    };

    // Get metadata including thumbnail URL
    let meta = Command::new("yt-dlp")
        .args([
            "--print",
            "%(title)s\n%(uploader)s\n%(duration)s\n%(id)s\n%(thumbnail)s",
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
    if lines.len() < 5 {
        return Err("could not parse metadata".into());
    }

    let raw_title = lines[0].to_string();
    let uploader = lines[1].to_string();
    let duration: i64 = lines[2].parse().unwrap_or(0);
    let video_id = lines[3].to_string();
    let thumbnail_url = lines[4].to_string();

    // Parse artist and clean title
    let (title, artist) = parse_artist_title(&raw_title, &uploader);
    let album = parse_album(&raw_title, artist.as_deref());

    // Download audio with embedded thumbnail
    let output_template = tracks_dir
        .join("%(title)s.%(ext)s")
        .to_string_lossy()
        .to_string();

    let dl = Command::new("yt-dlp")
        .args([
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "192K",
            "--embed-thumbnail",
            "--add-metadata",
            "--print",
            "after_move:filepath",
            "-o",
            &output_template,
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

    // Download artwork separately for display purposes
    let artwork_path = download_artwork(&video_id, &thumbnail_url, &artwork_dir);

    // Update ID3 tags with our parsed metadata (ffmpeg overwrites yt-dlp's tags)
    update_id3_tags(&file_path, &title, artist.as_deref(), album.as_deref());

    // Insert into database
    conn.execute(
        "INSERT INTO tracks (title, artist, album, duration_secs, file_path, artwork_path, source_url) 
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![title, artist, album, duration, file_path, artwork_path, query],
    )
    .map_err(|e| format!("db insert failed: {e}"))?;

    let id = conn.last_insert_rowid();

    Ok(AddResult {
        added: true,
        id,
        title,
        artist,
        album,
        file: file_path,
        artwork: artwork_path,
    })
}

/// Download artwork/thumbnail to artwork directory
fn download_artwork(video_id: &str, thumbnail_url: &str, artwork_dir: &Path) -> Option<String> {
    if thumbnail_url.is_empty() || thumbnail_url == "NA" {
        return None;
    }

    let artwork_path = artwork_dir.join(format!("{video_id}.jpg"));

    // Download using curl (more reliable than yt-dlp for thumbnails)
    let result = Command::new("curl")
        .args(["-sL", "-o", artwork_path.to_str().unwrap(), thumbnail_url])
        .output();

    match result {
        Ok(output) if output.status.success() && artwork_path.exists() => {
            Some(artwork_path.to_string_lossy().to_string())
        }
        _ => None,
    }
}

/// Update ID3 tags in MP3 file using ffmpeg
fn update_id3_tags(file_path: &str, title: &str, artist: Option<&str>, album: Option<&str>) {
    let tmp_path = format!("{file_path}.tmp.mp3");

    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        file_path.to_string(),
        "-c".to_string(),
        "copy".to_string(),
        "-metadata".to_string(),
        format!("title={}", title),
    ];

    if let Some(a) = artist {
        args.push("-metadata".to_string());
        args.push(format!("artist={a}"));
    }

    if let Some(a) = album {
        args.push("-metadata".to_string());
        args.push(format!("album={a}"));
    }

    args.push(tmp_path.clone());

    let result = Command::new("ffmpeg").args(&args).output();

    if let Ok(output) = result {
        if output.status.success() {
            // Replace original with updated file
            let _ = std::fs::rename(&tmp_path, file_path);
        } else {
            // Cleanup temp file on failure
            let _ = std::fs::remove_file(&tmp_path);
        }
    }
}
