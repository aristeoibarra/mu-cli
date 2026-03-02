use crate::db;
use crate::error::{MuError, Result};
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
    let separators = [" - ", " – ", " — ", " | "];

    for sep in separators {
        if let Some(idx) = raw_title.find(sep) {
            let artist = raw_title[..idx].trim().to_string();
            let title = raw_title[idx + sep.len()..].trim().to_string();
            let clean_title = clean_title(&title);
            return (clean_title, Some(artist));
        }
    }

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

    if lower.contains("full album") || lower.contains("complete album") {
        if let Some(artist_name) = artist {
            let without_artist = title
                .replace(&format!("{artist_name} - "), "")
                .replace(&format!("{artist_name} – "), "");
            return Some(clean_title(&without_artist));
        }
        return Some(clean_title(title));
    }

    None
}

fn run_yt_dlp(args: &[&str]) -> Result<std::process::Output> {
    Command::new("yt-dlp")
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                MuError::ExternalTool("yt-dlp not found. Install with: brew install yt-dlp".into())
            } else {
                MuError::ExternalTool(format!("failed to run yt-dlp: {e}"))
            }
        })
}

#[allow(clippy::too_many_lines)]
pub fn download(query: &str, conn: &Connection) -> Result<AddResult> {
    let data_dir = db::data_dir();
    let tracks_dir = data_dir.join("tracks");
    let artwork_dir = data_dir.join("artwork");

    let is_url = query.starts_with("http://") || query.starts_with("https://");
    let search_query = if is_url {
        query.to_string()
    } else {
        format!("ytsearch1:{query}")
    };

    // Get metadata including thumbnail URL
    let meta = run_yt_dlp(&[
        "--print",
        "%(title)s\n%(uploader)s\n%(duration)s\n%(id)s\n%(thumbnail)s",
        "--no-download",
        &search_query,
    ])?;

    if !meta.status.success() {
        return Err(MuError::Download(format!(
            "yt-dlp search failed: {}",
            String::from_utf8_lossy(&meta.stderr)
        )));
    }

    let meta_str = String::from_utf8_lossy(&meta.stdout);
    let lines: Vec<&str> = meta_str.trim().lines().collect();
    if lines.len() < 5 {
        return Err(MuError::Download("could not parse metadata".into()));
    }

    let raw_title = lines[0].to_string();
    let uploader = lines[1].to_string();
    let duration: i64 = lines[2].parse().unwrap_or(0);
    let video_id = lines[3].to_string();
    let thumbnail_url = lines[4].to_string();

    // Check for duplicate: use original query for URLs, constructed URL for searches
    let source_url = if is_url {
        query.to_string()
    } else {
        format!("https://www.youtube.com/watch?v={video_id}")
    };
    if let Some((existing_id, existing_title)) = db::find_track_by_url(conn, &source_url) {
        return Err(MuError::DuplicateTrack {
            id: existing_id,
            title: existing_title,
        });
    }
    // Also check the original query in case it differs from constructed URL
    if !is_url {
        if let Some((existing_id, existing_title)) = db::find_track_by_url(conn, query) {
            return Err(MuError::DuplicateTrack {
                id: existing_id,
                title: existing_title,
            });
        }
    }

    // Parse artist and clean title
    let (title, artist) = parse_artist_title(&raw_title, &uploader);
    let album = parse_album(&raw_title, artist.as_deref());

    // Download audio with embedded thumbnail
    let output_template = tracks_dir
        .join("%(title)s.%(ext)s")
        .to_string_lossy()
        .to_string();

    let dl = run_yt_dlp(&[
        "-x",
        "--audio-format",
        "m4a",
        "--audio-quality",
        "256K",
        "--embed-thumbnail",
        "--add-metadata",
        "--print",
        "after_move:filepath",
        "-o",
        &output_template,
        &search_query,
    ])?;

    if !dl.status.success() {
        return Err(MuError::Download(format!(
            "download failed: {}",
            String::from_utf8_lossy(&dl.stderr)
        )));
    }

    let file_path = String::from_utf8_lossy(&dl.stdout).trim().to_string();

    // Fix permissions so Apple Music can read the file
    set_readable_permissions(&file_path);

    // Try iTunes artwork first, fall back to YouTube thumbnail
    let artwork_path =
        fetch_itunes_artwork(artist.as_deref(), &title, &artwork_dir, &video_id)
            .or_else(|| download_artwork(&video_id, &thumbnail_url, &artwork_dir));

    // Update metadata and embed artwork if available
    update_metadata(
        &file_path,
        &title,
        artist.as_deref(),
        album.as_deref(),
        artwork_path.as_deref(),
    );

    // Insert into database
    conn.execute(
        "INSERT INTO tracks (title, artist, album, duration_secs, file_path, artwork_path, source_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![title, artist, album, duration, file_path, artwork_path, source_url],
    )?;

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

/// Set file permissions to 644 so Apple Music can read imported files
fn set_readable_permissions(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
}

/// Fetch high-quality artwork from iTunes Search API
fn fetch_itunes_artwork(
    artist: Option<&str>,
    title: &str,
    artwork_dir: &Path,
    video_id: &str,
) -> Option<String> {
    let search_term = match artist {
        Some(a) => format!("{a} {title}"),
        None => title.to_string(),
    };

    let url = format!(
        "https://itunes.apple.com/search?term={}&media=music&limit=1",
        urlencod(&search_term)
    );

    let result = Command::new("curl")
        .args(["-sL", &url])
        .output()
        .ok()?;

    if !result.status.success() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&result.stdout).ok()?;
    let artwork_url_100 = json
        .get("results")?
        .as_array()?
        .first()?
        .get("artworkUrl100")?
        .as_str()?;

    let artwork_url = artwork_url_100.replace("100x100bb", "1200x1200bb");
    let artwork_path = artwork_dir.join(format!("{video_id}.jpg"));

    let dl = Command::new("curl")
        .args(["-sL", "-o", artwork_path.to_str()?, &artwork_url])
        .output()
        .ok()?;

    if dl.status.success() && artwork_path.exists() {
        Some(artwork_path.to_string_lossy().to_string())
    } else {
        None
    }
}

/// Minimal URL encoding for iTunes search queries
fn urlencod(s: &str) -> String {
    s.replace(' ', "+")
        .replace('&', "%26")
        .replace('?', "%3F")
        .replace('#', "%23")
}

/// Download artwork/thumbnail to artwork directory (`YouTube` fallback)
fn download_artwork(video_id: &str, thumbnail_url: &str, artwork_dir: &Path) -> Option<String> {
    if thumbnail_url.is_empty() || thumbnail_url == "NA" {
        return None;
    }

    let artwork_path = artwork_dir.join(format!("{video_id}.jpg"));

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

/// Update metadata (and optionally embed artwork) in audio file using ffmpeg
fn update_metadata(
    file_path: &str,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    artwork_path: Option<&str>,
) {
    let tmp_path = format!("{file_path}.tmp.m4a");

    let mut args = vec!["-y".to_string(), "-i".to_string(), file_path.to_string()];

    if let Some(art) = artwork_path {
        args.extend(["-i".to_string(), art.to_string()]);
        args.extend([
            "-map".to_string(),
            "0:a".to_string(),
            "-map".to_string(),
            "1".to_string(),
            "-c".to_string(),
            "copy".to_string(),
            "-disposition:v:0".to_string(),
            "attached_pic".to_string(),
        ]);
    } else {
        args.extend(["-c".to_string(), "copy".to_string()]);
    }

    args.extend(["-metadata".to_string(), format!("title={title}")]);

    if let Some(a) = artist {
        args.extend(["-metadata".to_string(), format!("artist={a}")]);
    }

    if let Some(a) = album {
        args.extend(["-metadata".to_string(), format!("album={a}")]);
    }

    args.push(tmp_path.clone());

    let result = Command::new("ffmpeg").args(&args).output();

    if let Ok(output) = result {
        if output.status.success() {
            let _ = std::fs::rename(&tmp_path, file_path);
        } else {
            let _ = std::fs::remove_file(&tmp_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_artist_title ---

    #[test]
    fn parse_artist_title_with_dash() {
        let (title, artist) = parse_artist_title("Radiohead - Creep (Official Video)", "RadioheadVEVO");
        assert_eq!(title, "Creep");
        assert_eq!(artist, Some("Radiohead".to_string()));
    }

    #[test]
    fn parse_artist_title_with_en_dash() {
        let (title, artist) = parse_artist_title("Daft Punk \u{2013} Something About Us", "Daft Punk");
        assert_eq!(title, "Something About Us");
        assert_eq!(artist, Some("Daft Punk".to_string()));
    }

    #[test]
    fn parse_artist_title_with_em_dash() {
        let (title, artist) = parse_artist_title("Artist \u{2014} Song Name", "SomeUploader");
        assert_eq!(title, "Song Name");
        assert_eq!(artist, Some("Artist".to_string()));
    }

    #[test]
    fn parse_artist_title_with_pipe() {
        let (title, artist) = parse_artist_title("Artist | Song Name", "SomeUploader");
        assert_eq!(title, "Song Name");
        assert_eq!(artist, Some("Artist".to_string()));
    }

    #[test]
    fn parse_artist_title_no_separator_uses_uploader() {
        let (title, artist) = parse_artist_title("Bohemian Rhapsody", "Queen - Topic");
        assert_eq!(title, "Bohemian Rhapsody");
        assert_eq!(artist, Some("Queen".to_string()));
    }

    #[test]
    fn parse_artist_title_filters_vevo_uploader() {
        let (title, artist) = parse_artist_title("Some Song", "ArtistVEVO");
        assert_eq!(title, "Some Song");
        assert_eq!(artist, Some("Artist".to_string()));
    }

    #[test]
    fn parse_artist_title_youtube_uploader_returns_none() {
        let (title, artist) = parse_artist_title("Some Song", "YouTube Music Channel");
        assert_eq!(title, "Some Song");
        assert_eq!(artist, None);
    }

    #[test]
    fn parse_artist_title_empty_uploader_returns_none() {
        let (title, artist) = parse_artist_title("Some Song", "");
        assert_eq!(title, "Some Song");
        assert_eq!(artist, None);
    }

    // --- clean_title ---

    #[test]
    fn clean_title_removes_official_video() {
        assert_eq!(clean_title("Song (Official Video)"), "Song");
    }

    #[test]
    fn clean_title_removes_official_audio_brackets() {
        assert_eq!(clean_title("Song [Official Audio]"), "Song");
    }

    #[test]
    fn clean_title_removes_official_music_video() {
        assert_eq!(clean_title("Song (Official Music Video)"), "Song");
    }

    #[test]
    fn clean_title_removes_lyric_video() {
        assert_eq!(clean_title("Song (Lyric Video)"), "Song");
    }

    #[test]
    fn clean_title_removes_hd() {
        assert_eq!(clean_title("Song (HD)"), "Song");
    }

    #[test]
    fn clean_title_case_insensitive() {
        assert_eq!(clean_title("Song (official video)"), "Song");
        assert_eq!(clean_title("Song (OFFICIAL VIDEO)"), "Song");
    }

    #[test]
    fn clean_title_preserves_normal_parens() {
        assert_eq!(clean_title("Song (feat. Artist)"), "Song (feat. Artist)");
    }

    #[test]
    fn clean_title_preserves_plain_title() {
        assert_eq!(clean_title("Just a Song"), "Just a Song");
    }

    // --- parse_album ---

    #[test]
    fn parse_album_full_album_with_artist() {
        let album = parse_album("Artist - Album Name (Full Album)", Some("Artist"));
        assert_eq!(album, Some("Album Name".to_string()));
    }

    #[test]
    fn parse_album_full_album_without_artist() {
        let album = parse_album("Album Name (Full Album)", None);
        assert_eq!(album, Some("Album Name".to_string()));
    }

    #[test]
    fn parse_album_complete_album() {
        let album = parse_album("Some Complete Album Collection", Some("Artist"));
        assert_eq!(album, Some("Some Complete Album Collection".to_string()));
    }

    #[test]
    fn parse_album_not_album() {
        let album = parse_album("Artist - Regular Song", Some("Artist"));
        assert_eq!(album, None);
    }

    #[test]
    fn parse_album_no_match_returns_none() {
        let album = parse_album("Just a Normal Title", None);
        assert_eq!(album, None);
    }
}
