use chrono::{DateTime, Utc};
use rss::Channel;
use rusqlite::{Connection, params};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::db;

#[derive(Debug, Clone, Serialize)]
pub struct PodcastFeed {
    pub title: String,
    pub author: String,
    pub description: String,
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeData {
    pub title: String,
    pub description: String,
    pub pub_date: DateTime<Utc>,
    pub duration_secs: Option<i64>,
    pub source_url: String,
    pub guid: String,
}

/// Fetch and parse RSS feed from URL
pub async fn fetch_and_parse(feed_url: &str) -> Result<(PodcastFeed, Vec<EpisodeData>), String> {
    // Fetch feed with timeout
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("mu-podcast-player/0.1")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(feed_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch feed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let content = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse RSS/Atom feed
    let channel = Channel::read_from(&content[..])
        .map_err(|e| format!("Failed to parse feed: {}", e))?;

    let podcast = PodcastFeed {
        title: channel.title().to_string(),
        author: channel
            .itunes_ext()
            .and_then(|ext| ext.author())
            .unwrap_or("Unknown")
            .to_string(),
        description: channel.description().to_string(),
        artwork_url: channel
            .itunes_ext()
            .and_then(|ext| ext.image())
            .or_else(|| channel.image().map(|img| img.url()))
            .map(|s| s.to_string()),
    };

    let episodes: Vec<EpisodeData> = channel
        .items()
        .iter()
        .filter_map(|item| {
            // Must have enclosure (audio file)
            let enclosure = item.enclosure()?;
            
            // Must have GUID
            let guid = item.guid()?.value().to_string();
            
            // Parse publication date
            let pub_date = item
                .pub_date()
                .and_then(|date_str| parse_rfc2822_date(date_str))
                .or_else(|| {
                    // Fallback to Dublin Core date
                    item.dublin_core_ext()
                        .and_then(|dc| dc.dates().first())
                        .and_then(|date_str| parse_iso8601_date(date_str))
                })?;

            // Parse duration from iTunes extension
            let duration_secs = item
                .itunes_ext()
                .and_then(|ext| ext.duration())
                .and_then(|dur_str| parse_duration(dur_str));

            Some(EpisodeData {
                title: item.title().unwrap_or("Untitled Episode").to_string(),
                description: item.description().unwrap_or("").to_string(),
                pub_date,
                duration_secs,
                source_url: enclosure.url().to_string(),
                guid,
            })
        })
        .collect();

    if episodes.is_empty() {
        return Err("No episodes found in feed".to_string());
    }

    Ok((podcast, episodes))
}

/// Parse RFC 2822 date (common in RSS feeds)
fn parse_rfc2822_date(date_str: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc2822(date_str)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parse ISO 8601 date (Dublin Core extension)
fn parse_iso8601_date(date_str: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(date_str)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parse iTunes duration format (HH:MM:SS or MM:SS or seconds)
fn parse_duration(duration_str: &str) -> Option<i64> {
    let parts: Vec<&str> = duration_str.split(':').collect();
    
    match parts.len() {
        // Just seconds (e.g., "3661")
        1 => duration_str.parse::<i64>().ok(),
        
        // MM:SS
        2 => {
            let minutes = parts[0].parse::<i64>().ok()?;
            let seconds = parts[1].parse::<i64>().ok()?;
            Some(minutes * 60 + seconds)
        }
        
        // HH:MM:SS
        3 => {
            let hours = parts[0].parse::<i64>().ok()?;
            let minutes = parts[1].parse::<i64>().ok()?;
            let seconds = parts[2].parse::<i64>().ok()?;
            Some(hours * 3600 + minutes * 60 + seconds)
        }
        
        _ => None,
    }
}

/// Sanitize podcast/episode title for filesystem
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .chars()
        .take(100) // Limit filename length
        .collect()
}

/// Download episode audio file
pub async fn download_episode(
    episode: &EpisodeData,
    podcast_title: &str,
) -> Result<(PathBuf, u64), String> {
    let podcasts_dir = db::data_dir().join("podcasts");
    let podcast_dir = podcasts_dir.join(sanitize_filename(podcast_title));
    
    tokio::fs::create_dir_all(&podcast_dir)
        .await
        .map_err(|e| format!("Failed to create podcast directory: {}", e))?;

    // Try yt-dlp first (supports many sources including YouTube)
    if let Ok(result) = download_with_ytdlp(&episode.source_url, &podcast_dir, &episode.title).await {
        return Ok(result);
    }

    // Fallback: direct download (for MP3/M4A URLs)
    download_direct(&episode.source_url, &podcast_dir, &episode.title).await
}

/// Download using yt-dlp (async)
async fn download_with_ytdlp(
    url: &str,
    output_dir: &Path,
    title: &str,
) -> Result<(PathBuf, u64), String> {
    let safe_title = sanitize_filename(title);
    let output_template = output_dir.join(format!("{}.%(ext)s", safe_title));
    
    let output = tokio::process::Command::new("yt-dlp")
        .args([
            "-x",
            "--audio-format", "mp3",
            "--audio-quality", "128K",
            "--print", "after_move:filepath",
            "-o", &output_template.to_string_lossy(),
            url,
        ])
        .output()
        .await
        .map_err(|e| format!("yt-dlp execution failed: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "yt-dlp failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let file_path = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    let path = PathBuf::from(&file_path);
    
    let file_size = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("Failed to get file size: {}", e))?
        .len();

    Ok((path, file_size))
}

/// Direct download for MP3/M4A URLs (async)
async fn download_direct(
    url: &str,
    output_dir: &Path,
    title: &str,
) -> Result<(PathBuf, u64), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300)) // 5 minutes
        .user_agent("mu-podcast-player/0.1")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    // Determine file extension from content-type or URL
    let ext = response
        .headers()
        .get("content-type")
        .and_then(|ct| ct.to_str().ok())
        .and_then(|ct| {
            if ct.contains("mpeg") || ct.contains("mp3") {
                Some("mp3")
            } else if ct.contains("m4a") || ct.contains("mp4") {
                Some("m4a")
            } else {
                None
            }
        })
        .or_else(|| {
            if url.ends_with(".mp3") {
                Some("mp3")
            } else if url.ends_with(".m4a") {
                Some("m4a")
            } else {
                Some("mp3") // default
            }
        })
        .unwrap();

    let safe_title = sanitize_filename(title);
    let filename = format!("{}.{}", safe_title, ext);
    let filepath = output_dir.join(&filename);

    let content = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    let file_size = content.len() as u64;

    tokio::fs::write(&filepath, &content)
        .await
        .map_err(|e| format!("Failed to write file: {}", e))?;

    Ok((filepath, file_size))
}

/// Download podcast artwork and save locally
pub async fn download_artwork(
    artwork_url: &str,
    podcast_title: &str,
) -> Result<PathBuf, String> {
    let artwork_dir = db::data_dir().join("artwork");
    
    tokio::fs::create_dir_all(&artwork_dir)
        .await
        .map_err(|e| format!("Failed to create artwork directory: {}", e))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("mu-podcast-player/0.1")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    let response = client
        .get(artwork_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download artwork: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    // Determine file extension from content-type or URL
    let ext = response
        .headers()
        .get("content-type")
        .and_then(|ct| ct.to_str().ok())
        .and_then(|ct| {
            if ct.contains("jpeg") || ct.contains("jpg") {
                Some("jpg")
            } else if ct.contains("png") {
                Some("png")
            } else if ct.contains("webp") {
                Some("webp")
            } else {
                None
            }
        })
        .or_else(|| {
            if artwork_url.ends_with(".jpg") || artwork_url.ends_with(".jpeg") {
                Some("jpg")
            } else if artwork_url.ends_with(".png") {
                Some("png")
            } else if artwork_url.ends_with(".webp") {
                Some("webp")
            } else {
                Some("jpg") // default
            }
        })
        .unwrap();

    let safe_title = sanitize_filename(podcast_title);
    let filename = format!("{}.{}", safe_title, ext);
    let filepath = artwork_dir.join(&filename);

    let content = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read artwork: {}", e))?;

    tokio::fs::write(&filepath, &content)
        .await
        .map_err(|e| format!("Failed to write artwork: {}", e))?;

    Ok(filepath)
}

/// Insert or update podcast in database
pub fn upsert_podcast(
    conn: &Connection,
    feed: &PodcastFeed,
    feed_url: &str,
    max_episodes: i64,
    artwork_path: Option<&str>,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO podcasts (title, author, feed_url, description, artwork_url, artwork_path, max_episodes, last_checked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
         ON CONFLICT(feed_url) DO UPDATE SET
            title = ?1,
            author = ?2,
            description = ?4,
            artwork_url = ?5,
            artwork_path = ?6,
            last_checked = CURRENT_TIMESTAMP",
        params![
            feed.title,
            feed.author,
            feed_url,
            feed.description,
            feed.artwork_url,
            artwork_path,
            max_episodes,
        ],
    )
    .map_err(|e| format!("Failed to insert podcast: {}", e))?;

    let podcast_id = conn.last_insert_rowid();
    Ok(podcast_id)
}

/// Insert episode into database (skip if guid exists)
pub fn insert_episode(
    conn: &Connection,
    podcast_id: i64,
    episode: &EpisodeData,
    file_path: Option<&Path>,
    file_size: u64,
) -> Result<i64, String> {
    // Check if episode already exists by guid
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM episodes WHERE guid = ?1",
            params![episode.guid],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if exists {
        return Err("Episode already exists".to_string());
    }

    conn.execute(
        "INSERT INTO episodes 
            (podcast_id, title, description, pub_date, duration_secs, source_url, guid, file_path, file_size_bytes, is_downloaded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            podcast_id,
            episode.title,
            episode.description,
            episode.pub_date.to_rfc3339(),
            episode.duration_secs,
            episode.source_url,
            episode.guid,
            file_path.map(|p| p.to_string_lossy().to_string()),
            file_size as i64,
            file_path.is_some(),
        ],
    )
    .map_err(|e| format!("Failed to insert episode: {}", e))?;

    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("3661"), Some(3661));
        assert_eq!(parse_duration("61:01"), Some(3661));
        assert_eq!(parse_duration("1:01:01"), Some(3661));
        assert_eq!(parse_duration("05:30"), Some(330));
        assert_eq!(parse_duration("invalid"), None);
    }

    #[test]
    fn test_parse_rfc2822_date() {
        let date = parse_rfc2822_date("Wed, 02 Oct 2024 10:00:00 GMT");
        assert!(date.is_some());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("Episode: Test?"), "Episode_ Test_");
        assert_eq!(sanitize_filename("Normal Title"), "Normal Title");
        assert_eq!(sanitize_filename("Path/With\\Slashes"), "Path_With_Slashes");
    }
}
