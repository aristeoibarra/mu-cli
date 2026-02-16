use crate::{db, podcast};
use rusqlite::params;
use serde_json::json;

/// Subscribe to a podcast feed
pub async fn subscribe(
    feed_url: String,
    max_episodes: Option<i64>,
    auto_download: bool,
) -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    // Fetch and parse feed
    let (feed, mut episodes) = podcast::fetch_and_parse(&feed_url).await?;
    
    // Insert podcast
    let podcast_id = podcast::upsert_podcast(&conn, &feed, &feed_url, max_episodes.unwrap_or(5))?;
    
    // Sort episodes by date (oldest first for initial subscription)
    episodes.sort_by(|a, b| a.pub_date.cmp(&b.pub_date));
    
    // Take only the latest N episodes
    let max = max_episodes.unwrap_or(5) as usize;
    let episodes_to_download: Vec<_> = episodes.iter().rev().take(max).collect();
    
    let mut downloaded_count = 0;
    let mut failed_count = 0;
    
    if auto_download {
        for episode in episodes_to_download.iter() {
            // Download episode
            match podcast::download_episode(episode, &feed.title).await {
                Ok((file_path, file_size)) => {
                    // Insert into DB
                    if podcast::insert_episode(&conn, podcast_id, episode, Some(&file_path), file_size).is_ok() {
                        downloaded_count += 1;
                    }
                }
                Err(_) => {
                    // Insert without file (failed download)
                    podcast::insert_episode(&conn, podcast_id, episode, None, 0).ok();
                    failed_count += 1;
                }
            }
        }
    } else {
        // Just insert episode metadata without downloading
        for episode in episodes_to_download.iter() {
            podcast::insert_episode(&conn, podcast_id, episode, None, 0).ok();
        }
    }
    
    Ok(json!({
        "ok": true,
        "podcast_id": podcast_id,
        "title": feed.title,
        "author": feed.author,
        "episodes_found": episodes.len(),
        "downloaded": downloaded_count,
        "failed": failed_count,
    }).to_string())
}

/// List all subscribed podcasts
pub fn list() -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    let mut stmt = conn
        .prepare(
            "SELECT 
                p.id,
                p.title,
                p.author,
                p.feed_url,
                p.last_checked,
                p.auto_download,
                p.notify_new_episodes,
                p.max_episodes,
                COUNT(CASE WHEN e.playback_status = 'new' AND e.is_downloaded = 1 THEN 1 END) as unplayed,
                COUNT(e.id) as total_episodes
             FROM podcasts p
             LEFT JOIN episodes e ON e.podcast_id = p.id
             GROUP BY p.id
             ORDER BY p.title",
        )
        .map_err(|e| format!("Query error: {}", e))?;
    
    let podcasts: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "title": row.get::<_, String>(1)?,
                "author": row.get::<_, Option<String>>(2)?,
                "feed_url": row.get::<_, String>(3)?,
                "last_checked": row.get::<_, Option<String>>(4)?,
                "auto_download": row.get::<_, bool>(5)?,
                "notify": row.get::<_, bool>(6)?,
                "max_episodes": row.get::<_, i64>(7)?,
                "unplayed": row.get::<_, i64>(8)?,
                "total_episodes": row.get::<_, i64>(9)?,
            }))
        })
        .map_err(|e| format!("Query error: {}", e))?
        .filter_map(|r| r.ok())
        .collect();
    
    Ok(json!({ "podcasts": podcasts }).to_string())
}

/// List episodes of a specific podcast
pub fn list_episodes(podcast_name: String, unplayed_only: bool) -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    // Find podcast ID
    let podcast_id = db::find_podcast_id(&conn, &podcast_name)
        .map_err(|_| format!("Podcast '{}' not found", podcast_name))?;
    
    let query = if unplayed_only {
        "SELECT id, title, pub_date, duration_secs, playback_status, is_downloaded, file_size_bytes
         FROM episodes
         WHERE podcast_id = ?1 AND playback_status = 'new' AND is_downloaded = 1
         ORDER BY pub_date DESC"
    } else {
        "SELECT id, title, pub_date, duration_secs, playback_status, is_downloaded, file_size_bytes
         FROM episodes
         WHERE podcast_id = ?1
         ORDER BY pub_date DESC"
    };
    
    let mut stmt = conn
        .prepare(query)
        .map_err(|e| format!("Query error: {}", e))?;
    
    let episodes: Vec<serde_json::Value> = stmt
        .query_map([podcast_id], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "title": row.get::<_, String>(1)?,
                "pub_date": row.get::<_, String>(2)?,
                "duration_secs": row.get::<_, Option<i64>>(3)?,
                "status": row.get::<_, String>(4)?,
                "downloaded": row.get::<_, bool>(5)?,
                "size_mb": row.get::<_, i64>(6)? as f64 / 1024.0 / 1024.0,
            }))
        })
        .map_err(|e| format!("Query error: {}", e))?
        .filter_map(|r| r.ok())
        .collect();
    
    Ok(json!({
        "podcast": podcast_name,
        "episodes": episodes
    }).to_string())
}

/// Update podcast feeds (check for new episodes)
pub async fn update(podcast_name: Option<String>) -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    let query = if let Some(ref name) = podcast_name {
        format!("SELECT id, title, feed_url, auto_download FROM podcasts WHERE title = '{}' OR feed_url = '{}'", name, name)
    } else {
        "SELECT id, title, feed_url, auto_download FROM podcasts".to_string()
    };
    
    let mut stmt = conn
        .prepare(&query)
        .map_err(|e| format!("Query error: {}", e))?;
    
    let podcasts: Vec<(i64, String, String, bool)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })
        .map_err(|e| format!("Query error: {}", e))?
        .filter_map(|r| r.ok())
        .collect();
    
    let mut results = Vec::new();
    
    for (podcast_id, title, feed_url, auto_download) in podcasts {
        match podcast::fetch_and_parse(&feed_url).await {
            Ok((_feed, episodes)) => {
                let mut new_count = 0;
                let mut downloaded_count = 0;
                
                for episode in episodes {
                    // Check if episode already exists
                    let exists: bool = conn
                        .query_row(
                            "SELECT 1 FROM episodes WHERE guid = ?1",
                            params![episode.guid],
                            |_| Ok(true),
                        )
                        .unwrap_or(false);
                    
                    if !exists {
                        new_count += 1;
                        
                        if auto_download {
                            match podcast::download_episode(&episode, &title).await {
                                Ok((file_path, file_size)) => {
                                    podcast::insert_episode(&conn, podcast_id, &episode, Some(&file_path), file_size).ok();
                                    downloaded_count += 1;
                                }
                                Err(_) => {
                                    podcast::insert_episode(&conn, podcast_id, &episode, None, 0).ok();
                                }
                            }
                        } else {
                            podcast::insert_episode(&conn, podcast_id, &episode, None, 0).ok();
                        }
                    }
                }
                
                // Update last_checked
                conn.execute(
                    "UPDATE podcasts SET last_checked = CURRENT_TIMESTAMP WHERE id = ?1",
                    params![podcast_id],
                ).ok();
                
                results.push(json!({
                    "podcast": title,
                    "new_episodes": new_count,
                    "downloaded": downloaded_count,
                }));
            }
            Err(e) => {
                results.push(json!({
                    "podcast": title,
                    "error": e,
                }));
            }
        }
    }
    
    Ok(json!({
        "ok": true,
        "results": results
    }).to_string())
}

/// Configure podcast settings
pub fn config(
    podcast_name: String,
    auto_download: Option<bool>,
    notify: Option<bool>,
    max_episodes: Option<i64>,
) -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    let podcast_id = db::find_podcast_id(&conn, &podcast_name)
        .map_err(|_| format!("Podcast '{}' not found", podcast_name))?;
    
    if let Some(ad) = auto_download {
        conn.execute(
            "UPDATE podcasts SET auto_download = ?1 WHERE id = ?2",
            params![ad, podcast_id],
        ).map_err(|e| format!("Update error: {}", e))?;
    }
    
    if let Some(n) = notify {
        conn.execute(
            "UPDATE podcasts SET notify_new_episodes = ?1 WHERE id = ?2",
            params![n, podcast_id],
        ).map_err(|e| format!("Update error: {}", e))?;
    }
    
    if let Some(max) = max_episodes {
        conn.execute(
            "UPDATE podcasts SET max_episodes = ?1 WHERE id = ?2",
            params![max, podcast_id],
        ).map_err(|e| format!("Update error: {}", e))?;
    }
    
    Ok(json!({
        "ok": true,
        "podcast": podcast_name,
        "updated": true
    }).to_string())
}

/// Unsubscribe from a podcast
pub async fn unsubscribe(podcast_name: String, delete_files: bool) -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    let podcast_id = db::find_podcast_id(&conn, &podcast_name)
        .map_err(|_| format!("Podcast '{}' not found", podcast_name))?;
    
    if delete_files {
        // Get all file paths
        let mut stmt = conn
            .prepare("SELECT file_path FROM episodes WHERE podcast_id = ?1 AND is_downloaded = 1")
            .map_err(|e| format!("Query error: {}", e))?;
        
        let files: Vec<String> = stmt
            .query_map([podcast_id], |row| row.get(0))
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        
        // Delete files
        for file_path in files {
            tokio::fs::remove_file(&file_path).await.ok();
        }
        
        // Try to remove podcast directory
        let podcast_dir = db::data_dir().join("podcasts").join(&podcast_name);
        tokio::fs::remove_dir_all(&podcast_dir).await.ok();
    }
    
    // Delete from database (cascade will handle episodes)
    conn.execute("DELETE FROM podcasts WHERE id = ?1", params![podcast_id])
        .map_err(|e| format!("Delete error: {}", e))?;
    
    Ok(json!({
        "ok": true,
        "podcast": podcast_name,
        "files_deleted": delete_files
    }).to_string())
}

/// Cleanup completed episodes
pub async fn cleanup(dry_run: bool, force: bool) -> Result<String, String> {
    use chrono::{Datelike, Utc, Weekday};
    
    let now = Utc::now();
    
    // Only run on Sundays unless forced
    if !force && now.weekday() != Weekday::Sun {
        return Ok(json!({
            "ok": true,
            "message": "Cleanup only runs on Sundays. Use --force to run now.",
            "today": now.weekday().to_string()
        }).to_string());
    }
    
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    // Find episodes to delete (completed, downloaded, but not the latest per podcast)
    let query = "
        WITH latest_per_podcast AS (
            SELECT podcast_id, MAX(pub_date) as latest_date
            FROM episodes
            GROUP BY podcast_id
        )
        SELECT e.id, e.file_path, e.title, p.title as podcast_title, e.file_size_bytes
        FROM episodes e
        JOIN podcasts p ON p.id = e.podcast_id
        LEFT JOIN latest_per_podcast lp ON lp.podcast_id = e.podcast_id 
            AND e.pub_date = lp.latest_date
        WHERE e.playback_status = 'completed'
          AND e.is_downloaded = 1
          AND lp.latest_date IS NULL
    ";
    
    let mut stmt = conn
        .prepare(query)
        .map_err(|e| format!("Query error: {}", e))?;
    
    let to_delete: Vec<(i64, String, String, String, i64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|e| format!("Query error: {}", e))?
        .filter_map(|r| r.ok())
        .collect();
    
    let mut deleted = Vec::new();
    let mut total_freed: u64 = 0;
    
    if !dry_run {
        for (id, file_path, title, podcast_title, file_size) in to_delete {
            if tokio::fs::remove_file(&file_path).await.is_ok() {
                conn.execute(
                    "UPDATE episodes SET is_downloaded = 0, file_path = NULL, marked_for_deletion = 1 WHERE id = ?1",
                    params![id],
                ).ok();
                
                total_freed += file_size as u64;
                deleted.push(json!({
                    "podcast": podcast_title,
                    "episode": title,
                    "size_mb": file_size as f64 / 1024.0 / 1024.0,
                }));
            }
        }
    } else {
        for (_, _, title, podcast_title, file_size) in to_delete {
            total_freed += file_size as u64;
            deleted.push(json!({
                "podcast": podcast_title,
                "episode": title,
                "size_mb": file_size as f64 / 1024.0 / 1024.0,
            }));
        }
    }
    
    Ok(json!({
        "ok": true,
        "dry_run": dry_run,
        "deleted": deleted,
        "total_freed_mb": total_freed as f64 / 1024.0 / 1024.0,
    }).to_string())
}

/// Get listening statistics
pub fn stats(podcast_name: Option<String>) -> Result<String, String> {
    let conn = db::open(&db::data_dir().join("mu.db")).map_err(|e| format!("DB error: {}", e))?;
    
    if let Some(ref name) = podcast_name {
        // Stats for specific podcast
        let podcast_id = db::find_podcast_id(&conn, name)
            .map_err(|_| format!("Podcast '{}' not found", name))?;
        
        let mut stmt = conn
            .prepare(
                "SELECT 
                    COUNT(CASE WHEN playback_status = 'completed' THEN 1 END) as completed,
                    COUNT(CASE WHEN playback_status = 'new' THEN 1 END) as unplayed,
                    COUNT(CASE WHEN playback_status = 'playing' THEN 1 END) as in_progress,
                    SUM(CASE WHEN playback_status = 'completed' THEN duration_secs ELSE 0 END) as total_time,
                    COUNT(*) as total_episodes
                 FROM episodes
                 WHERE podcast_id = ?1"
            )
            .map_err(|e| format!("Query error: {}", e))?;
        
        let stats = stmt
            .query_row([podcast_id], |row| {
                Ok(json!({
                    "podcast": name,
                    "completed_episodes": row.get::<_, i64>(0)?,
                    "unplayed_episodes": row.get::<_, i64>(1)?,
                    "in_progress_episodes": row.get::<_, i64>(2)?,
                    "total_listening_time_seconds": row.get::<_, i64>(3)?,
                    "total_listening_time_hours": row.get::<_, i64>(3)? as f64 / 3600.0,
                    "total_episodes": row.get::<_, i64>(4)?,
                }))
            })
            .map_err(|e| format!("Query error: {}", e))?;
        
        Ok(stats.to_string())
    } else {
        // Global stats across all podcasts
        let mut stmt = conn
            .prepare(
                "SELECT 
                    p.title,
                    COUNT(CASE WHEN e.playback_status = 'completed' THEN 1 END) as completed,
                    SUM(CASE WHEN e.playback_status = 'completed' THEN e.duration_secs ELSE 0 END) as total_time
                 FROM podcasts p
                 LEFT JOIN episodes e ON e.podcast_id = p.id
                 GROUP BY p.id
                 HAVING completed > 0
                 ORDER BY total_time DESC"
            )
            .map_err(|e| format!("Query error: {}", e))?;
        
        let podcasts: Vec<serde_json::Value> = stmt
            .query_map([], |row| {
                Ok(json!({
                    "podcast": row.get::<_, String>(0)?,
                    "completed_episodes": row.get::<_, i64>(1)?,
                    "listening_time_seconds": row.get::<_, i64>(2)?,
                    "listening_time_hours": row.get::<_, i64>(2)? as f64 / 3600.0,
                }))
            })
            .map_err(|e| format!("Query error: {}", e))?
            .filter_map(|r| r.ok())
            .collect();
        
        // Overall totals
        let totals: (i64, i64, i64) = conn
            .query_row(
                "SELECT 
                    COUNT(CASE WHEN playback_status = 'completed' THEN 1 END),
                    COUNT(CASE WHEN playback_status = 'new' THEN 1 END),
                    SUM(CASE WHEN playback_status = 'completed' THEN duration_secs ELSE 0 END)
                 FROM episodes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            )
            .unwrap_or((0, 0, 0));
        
        Ok(json!({
            "total_completed_episodes": totals.0,
            "total_unplayed_episodes": totals.1,
            "total_listening_time_seconds": totals.2,
            "total_listening_time_hours": totals.2 as f64 / 3600.0,
            "total_listening_time_days": totals.2 as f64 / 3600.0 / 24.0,
            "podcasts": podcasts,
        }).to_string())
    }
}
