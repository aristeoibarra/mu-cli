use crate::error::{MuError, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

const FIELD_SEP: char = '\x1E'; // ASCII Record Separator
const RECORD_SEP: char = '\x1D'; // ASCII Group Separator

/// Result of importing a track to Apple Music
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub track_name: Option<String>,
    pub persistent_id: Option<String>,
}

/// Import track and set metadata (artist, album). Genre is always "Music".
pub fn import_with_metadata(
    path: &Path,
    artist: Option<&str>,
    album: Option<&str>,
) -> Result<ImportResult> {
    let path_str = escape_applescript(&path.to_string_lossy());

    let mut set_props = vec!["set genre of theTrack to \"Music\"".to_string()];
    if let Some(a) = artist {
        set_props.push(format!(
            "set artist of theTrack to \"{}\"",
            escape_applescript(a)
        ));
    }
    if let Some(a) = album {
        set_props.push(format!(
            "set album of theTrack to \"{}\"",
            escape_applescript(a)
        ));
    }

    let set_commands = set_props.join("\n            ");
    let fs = FIELD_SEP;

    let script = format!(
        r#"tell application "Music"
            set theTrack to add POSIX file "{path_str}"
            {set_commands}
            set trackName to name of theTrack
            set trackId to persistent ID of theTrack
            return trackName & "{fs}" & trackId
        end tell"#
    );

    let output = run_osascript_output(&script)?;
    let parts: Vec<&str> = output.trim().split(FIELD_SEP).collect();

    Ok(ImportResult {
        track_name: parts.first().map(ToString::to_string),
        persistent_id: parts.get(1).map(ToString::to_string),
    })
}

/// Check if a track is already in Apple Music library by file path
pub fn is_track_in_library(path: &Path) -> bool {
    let path_str = escape_applescript(&path.to_string_lossy());
    let script = format!(
        r#"tell application "Music"
            try
                set matchingTracks to (every track of library playlist 1 whose location is (POSIX file "{path_str}"))
                return (count of matchingTracks) > 0
            on error
                return false
            end try
        end tell"#
    );

    run_osascript_output(&script)
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Play all tracks in library or a specific playlist
pub fn play_playlist(playlist: Option<&str>) -> Result<()> {
    let script = match playlist {
        Some(name) => format!(
            r#"tell application "Music"
                try
                    play playlist "{}"
                on error
                    -- Playlist doesn't exist, try playing from library
                    play library playlist 1
                end try
            end tell"#,
            escape_applescript(name)
        ),
        None => r#"tell application "Music"
            play library playlist 1
        end tell"#
            .to_string(),
    };

    run_osascript(&script)
}

/// Play a specific track by name
pub fn play_track(track_name: &str) -> Result<()> {
    let script = format!(
        r#"tell application "Music"
            play (first track of library playlist 1 whose name contains "{}")
        end tell"#,
        escape_applescript(track_name)
    );

    run_osascript(&script)
}

/// Pause playback
pub fn pause() -> Result<()> {
    run_osascript(r#"tell application "Music" to pause"#)
}

/// Resume playback
pub fn resume() -> Result<()> {
    run_osascript(r#"tell application "Music" to play"#)
}

/// Stop playback
pub fn stop() -> Result<()> {
    run_osascript(r#"tell application "Music" to stop"#)
}

/// Next track
pub fn next_track() -> Result<()> {
    run_osascript(r#"tell application "Music" to next track"#)
}

/// Previous track
pub fn previous_track() -> Result<()> {
    run_osascript(r#"tell application "Music" to previous track"#)
}

/// Get current playback status
pub fn get_status() -> Result<PlaybackStatus> {
    let fs = FIELD_SEP;
    let script = format!(
        r#"tell application "Music"
        set output to ""
        try
            set t to current track
            set trackName to name of t
            set trackArtist to artist of t
            set trackAlbum to album of t
            set state to player state as string
            set pos to player position
            set dur to duration of t
            set output to trackName & "{fs}" & trackArtist & "{fs}" & trackAlbum & "{fs}" & state & "{fs}" & pos & "{fs}" & dur
        on error
            set output to "{fs}{fs}stopped{fs}0{fs}0"
        end try
        return output
    end tell"#
    );

    let output = run_osascript_output(&script)?;
    let parts: Vec<&str> = output.trim().split(FIELD_SEP).collect();

    if parts.len() >= 6 {
        Ok(PlaybackStatus {
            track: if parts[0].is_empty() {
                None
            } else {
                Some(parts[0].to_string())
            },
            artist: if parts[1].is_empty() {
                None
            } else {
                Some(parts[1].to_string())
            },
            album: if parts[2].is_empty() {
                None
            } else {
                Some(parts[2].to_string())
            },
            state: parts[3].to_string(),
            position_secs: parts[4].parse().unwrap_or(0.0),
            duration_secs: parts[5].parse().unwrap_or(0.0),
        })
    } else {
        Ok(PlaybackStatus {
            track: None,
            artist: None,
            album: None,
            state: "stopped".to_string(),
            position_secs: 0.0,
            duration_secs: 0.0,
        })
    }
}

/// Create a playlist in Music.app
pub fn create_playlist(name: &str) -> Result<()> {
    let script = format!(
        r#"tell application "Music"
            try
                get playlist "{}"
            on error
                make new user playlist with properties {{name:"{}"}}
            end try
        end tell"#,
        escape_applescript(name),
        escape_applescript(name)
    );

    run_osascript(&script)
}

/// Add existing track (by name) to playlist
pub fn add_track_to_playlist(track_name: &str, playlist: &str) -> Result<()> {
    create_playlist(playlist)?;

    let script = format!(
        r#"tell application "Music"
            set thePlaylist to playlist "{}"
            set theTrack to first track of library playlist 1 whose name contains "{}"
            duplicate theTrack to thePlaylist
        end tell"#,
        escape_applescript(playlist),
        escape_applescript(track_name)
    );

    run_osascript(&script)
}

/// Add track to playlist using persistent ID if available, fallback to name matching
pub fn add_track_to_playlist_smart(
    persistent_id: Option<&str>,
    track_name: &str,
    playlist: &str,
) -> Result<()> {
    if let Some(pid) = persistent_id {
        add_track_to_playlist_by_id(pid, playlist)
    } else {
        add_track_to_playlist(track_name, playlist)
    }
}

/// Delete a track from Apple Music library by persistent ID (cascades to all playlists)
pub fn delete_track(persistent_id: &str) -> Result<()> {
    let escaped_id = escape_applescript(persistent_id);
    let script = format!(
        r#"tell application "Music"
            try
                delete (first track of library playlist 1 whose persistent ID is "{escaped_id}")
            end try
        end tell"#
    );

    run_osascript(&script)
}

/// Delete a track from Apple Music library by file path (fallback when no persistent ID)
pub fn delete_track_by_path(path: &str) -> Result<()> {
    let escaped_path = escape_applescript(path);
    let script = format!(
        r#"tell application "Music"
            try
                set matchingTracks to (every track of library playlist 1 whose location is (POSIX file "{escaped_path}"))
                repeat with t in matchingTracks
                    delete t
                end repeat
            end try
        end tell"#
    );

    run_osascript(&script)
}

/// Remove a track from a specific playlist (not from library) by persistent ID
pub fn remove_track_from_playlist(persistent_id: &str, playlist: &str) -> Result<()> {
    let escaped_id = escape_applescript(persistent_id);
    let script = format!(
        r#"tell application "Music"
            try
                set thePlaylist to user playlist "{}"
                delete (first track of thePlaylist whose persistent ID is "{escaped_id}")
            end try
        end tell"#,
        escape_applescript(playlist)
    );

    run_osascript(&script)
}

/// Add track to playlist by persistent ID (precise, no substring matching)
pub fn add_track_to_playlist_by_id(persistent_id: &str, playlist: &str) -> Result<()> {
    create_playlist(playlist)?;

    let escaped_id = escape_applescript(persistent_id);
    let script = format!(
        r#"tell application "Music"
            set thePlaylist to playlist "{}"
            set theTrack to first track of library playlist 1 whose persistent ID is "{escaped_id}"
            duplicate theTrack to thePlaylist
        end tell"#,
        escape_applescript(playlist)
    );

    run_osascript(&script)
}

/// Get persistent IDs of all tracks in an Apple Music playlist
pub fn get_playlist_track_ids(playlist: &str) -> Result<Vec<String>> {
    let rs = RECORD_SEP;
    let script = format!(
        r#"tell application "Music"
            try
                set thePlaylist to user playlist "{}"
                set output to ""
                repeat with t in tracks of thePlaylist
                    set output to output & (persistent ID of t) & "{rs}"
                end repeat
                return output
            on error
                return ""
            end try
        end tell"#,
        escape_applescript(playlist)
    );

    let output = run_osascript_output(&script)?;
    let ids: Vec<String> = output
        .trim()
        .split(RECORD_SEP)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();

    Ok(ids)
}

/// Delete a playlist from Apple Music
pub fn delete_playlist(name: &str) -> Result<()> {
    let script = format!(
        r#"tell application "Music"
            try
                delete playlist "{}"
            end try
        end tell"#,
        escape_applescript(name)
    );

    run_osascript(&script)
}

/// Get list of all user playlists in Apple Music
pub fn list_playlists() -> Result<Vec<PlaylistInfo>> {
    let fs = FIELD_SEP;
    let rs = RECORD_SEP;
    let script = format!(
        r#"tell application "Music"
        set output to ""
        repeat with p in user playlists
            set pName to name of p
            set pCount to count of tracks of p
            set output to output & pName & "{fs}" & pCount & "{rs}"
        end repeat
        return output
    end tell"#
    );

    let output = run_osascript_output(&script)?;
    let playlists: Vec<PlaylistInfo> = output
        .trim()
        .split(RECORD_SEP)
        .filter(|s| !s.is_empty())
        .filter_map(|s| {
            let parts: Vec<&str> = s.split(FIELD_SEP).collect();
            if parts.len() >= 2 {
                Some(PlaylistInfo {
                    name: parts[0].to_string(),
                    track_count: parts[1].parse().unwrap_or(0),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(playlists)
}

/// Get track count and total duration in Apple Music library (batch query)
pub fn get_library_stats() -> Result<LibraryStats> {
    let fs = FIELD_SEP;
    let script = format!(
        r#"tell application "Music"
        set trackCount to count of tracks of library playlist 1
        set durations to duration of every track of library playlist 1
        set totalTime to 0
        repeat with d in durations
            set totalTime to totalTime + d
        end repeat
        return (trackCount as string) & "{fs}" & (totalTime as string)
    end tell"#
    );

    let output = run_osascript_output(&script)?;
    let parts: Vec<&str> = output.trim().split(FIELD_SEP).collect();

    Ok(LibraryStats {
        track_count: parts.first().and_then(|s| s.parse().ok()).unwrap_or(0),
        total_duration_secs: parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackStatus {
    pub track: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub state: String,
    pub position_secs: f64,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaylistInfo {
    pub name: String,
    pub track_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryStats {
    pub track_count: i64,
    pub total_duration_secs: f64,
}

/// Set the favorited property on a track by persistent ID
pub fn set_track_loved(persistent_id: &str, loved: bool) -> Result<()> {
    let loved_str = if loved { "true" } else { "false" };
    let escaped_id = escape_applescript(persistent_id);
    let script = format!(
        r#"tell application "Music"
            set theTrack to first track of library playlist 1 whose persistent ID is "{escaped_id}"
            set favorited of theTrack to {loved_str}
        end tell"#
    );

    run_osascript(&script)
}

/// Get persistent IDs of all favorited tracks in Apple Music library
pub fn get_loved_track_ids() -> Result<Vec<String>> {
    let rs = RECORD_SEP;
    let script = format!(
        r#"tell application "Music"
        set output to ""
        repeat with t in (every track of library playlist 1 whose favorited is true)
            set output to output & (persistent ID of t) & "{rs}"
        end repeat
        return output
    end tell"#
    );

    let output = run_osascript_output(&script)?;
    let ids: Vec<String> = output
        .trim()
        .split(RECORD_SEP)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect();

    Ok(ids)
}

/// Get play counts for all tracks in Apple Music library
pub fn get_play_counts() -> Result<Vec<(String, i64)>> {
    let fs = FIELD_SEP;
    let rs = RECORD_SEP;
    let script = format!(
        r#"tell application "Music"
        set output to ""
        repeat with t in (every track of library playlist 1)
            set pid to persistent ID of t
            set pc to played count of t
            set output to output & pid & "{fs}" & pc & "{rs}"
        end repeat
        return output
    end tell"#
    );

    let output = run_osascript_output(&script)?;
    let results: Vec<(String, i64)> = output
        .trim()
        .split(RECORD_SEP)
        .filter(|s| !s.is_empty())
        .filter_map(|s| {
            let parts: Vec<&str> = s.split(FIELD_SEP).collect();
            if parts.len() >= 2 {
                Some((parts[0].to_string(), parts[1].parse().unwrap_or(0)))
            } else {
                None
            }
        })
        .collect();

    Ok(results)
}

/// Escape special characters for `AppleScript` strings
pub fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn run_osascript(script: &str) -> Result<()> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(MuError::AppleScript(stderr.trim().to_string()))
    }
}

fn run_osascript_output(script: &str) -> Result<String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(MuError::AppleScript(stderr.trim().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_applescript_quotes() {
        assert_eq!(escape_applescript(r#"say "hello""#), r#"say \"hello\""#);
    }

    #[test]
    fn escape_applescript_backslashes() {
        assert_eq!(escape_applescript(r"path\to\file"), r"path\\to\\file");
    }

    #[test]
    fn escape_applescript_combined() {
        assert_eq!(
            escape_applescript(r#"a\"b"#),
            r#"a\\\"b"#
        );
    }

    #[test]
    fn escape_applescript_clean_input() {
        assert_eq!(escape_applescript("hello world"), "hello world");
    }
}
