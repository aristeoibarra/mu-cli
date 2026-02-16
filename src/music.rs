use std::path::Path;
use std::process::Command;

/// Import a track to Apple Music library
pub fn import_to_library(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();
    let script = format!(
        r#"tell application "Music" to add POSIX file "{}" to library playlist 1"#,
        path_str
    );

    run_osascript(&script)
}

/// Play all tracks in library or a specific playlist
pub fn play_playlist(playlist: Option<&str>) -> Result<(), String> {
    let script = match playlist {
        Some(name) => format!(
            r#"tell application "Music"
                play playlist "{}"
            end tell"#,
            name
        ),
        None => r#"tell application "Music"
            play library playlist 1
        end tell"#
            .to_string(),
    };

    run_osascript(&script)
}

/// Play a specific track by name
pub fn play_track(track_name: &str) -> Result<(), String> {
    let script = format!(
        r#"tell application "Music"
            play (first track whose name contains "{}")
        end tell"#,
        track_name
    );

    run_osascript(&script)
}

/// Pause playback
pub fn pause() -> Result<(), String> {
    run_osascript(r#"tell application "Music" to pause"#)
}

/// Resume playback
pub fn resume() -> Result<(), String> {
    run_osascript(r#"tell application "Music" to play"#)
}

/// Stop playback
pub fn stop() -> Result<(), String> {
    run_osascript(r#"tell application "Music" to stop"#)
}

/// Next track
pub fn next_track() -> Result<(), String> {
    run_osascript(r#"tell application "Music" to next track"#)
}

/// Previous track
pub fn previous_track() -> Result<(), String> {
    run_osascript(r#"tell application "Music" to previous track"#)
}

/// Set playback speed (uses Music.app playback rate)
pub fn set_speed(speed: f32) -> Result<(), String> {
    // Music.app doesn't have native speed control, but we can use this
    // For podcasts, users typically use Podcasts.app which has speed control
    // This is a no-op for Music.app
    let _ = speed;
    Ok(())
}

/// Get current playback status
pub fn get_status() -> Result<PlaybackStatus, String> {
    let script = r#"tell application "Music"
        set output to ""
        try
            set t to current track
            set output to (name of t) & "|" & (player state as string) & "|" & (player position) & "|" & (duration of t)
        on error
            set output to "|stopped|0|0"
        end try
        return output
    end tell"#;

    let output = run_osascript_output(script)?;
    let parts: Vec<&str> = output.trim().split('|').collect();

    if parts.len() >= 4 {
        Ok(PlaybackStatus {
            track: if parts[0].is_empty() {
                None
            } else {
                Some(parts[0].to_string())
            },
            state: parts[1].to_string(),
            position_secs: parts[2].parse().unwrap_or(0.0),
            duration_secs: parts[3].parse().unwrap_or(0.0),
        })
    } else {
        Ok(PlaybackStatus {
            track: None,
            state: "stopped".to_string(),
            position_secs: 0.0,
            duration_secs: 0.0,
        })
    }
}

/// Create a playlist in Music.app
pub fn create_playlist(name: &str) -> Result<(), String> {
    let script = format!(
        r#"tell application "Music"
            make new user playlist with properties {{name:"{}"}}
        end tell"#,
        name
    );

    run_osascript(&script)
}

/// Add track to playlist
pub fn add_to_playlist(track_path: &Path, playlist: &str) -> Result<(), String> {
    let path_str = track_path.to_string_lossy();
    let script = format!(
        r#"tell application "Music"
            set theTrack to add POSIX file "{}" to library playlist 1
            duplicate theTrack to playlist "{}"
        end tell"#,
        path_str, playlist
    );

    run_osascript(&script)
}

#[derive(Debug, Clone)]
pub struct PlaybackStatus {
    pub track: Option<String>,
    pub state: String, // "playing", "paused", "stopped"
    pub position_secs: f64,
    pub duration_secs: f64,
}

fn run_osascript(script: &str) -> Result<(), String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("AppleScript error: {}", stderr.trim()))
    }
}

fn run_osascript_output(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("Failed to run osascript: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("AppleScript error: {}", stderr.trim()))
    }
}
