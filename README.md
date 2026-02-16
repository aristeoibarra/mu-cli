# mu

Minimal local music player and podcast manager CLI for macOS.

## Features

### Music
- Download songs from YouTube or search by name
- Organize tracks into playlists
- Playback controls (play, pause, resume, next, previous)
- Shuffle mode
- Background daemon with persistent playback state
- All output in JSON format

### Podcasts (v2)
- Subscribe to RSS feeds with automatic hourly updates
- Auto-download new episodes (configurable per podcast)
- Desktop notifications for new episodes
- Automatic playback progress tracking (saves every 30s)
- Mark episodes as completed automatically
- Intelligent cleanup (keeps unplayed + latest episode)
- Storage management with configurable GB limit
- Playback speed control (0.5x - 3.0x)
- Listening statistics (global and per-podcast)

## Installation

### Prerequisites
- macOS
- Rust toolchain
- `yt-dlp` in PATH: `brew install yt-dlp`

### Build & Install
```bash
cargo build --release
cp target/release/mu /opt/homebrew/bin/mu
```

## Usage

### Music

#### Download
```bash
# Search YouTube and download
mu add "song name"

# Download and add to playlist
mu add "song name" --playlist focus

# Download from direct URL
mu add "https://youtube.com/watch?v=..."
```

#### Playback
```bash
# Play all tracks
mu play

# Play specific playlist
mu play focus

# Play with shuffle
mu play focus --shuffle

# Start from specific track
mu play --from 5
mu play focus --from "song title"

# Controls
mu pause
mu resume
mu next
mu previous
mu speed 1.5          # 0.5x - 3.0x
mu stop               # stops playback and kills daemon
```

#### Playlists
```bash
# Create playlist
mu playlist create <name>

# Add track to playlist
mu playlist add <playlist> <track_id|title>

# Remove track from playlist (keeps track in library)
mu playlist remove-track <playlist> <track_id|title>

# Delete entire playlist
mu playlist remove <name>

# List all playlists
mu playlist list
```

#### Library Management
```bash
# List all tracks (JSON)
mu list

# List tracks in playlist
mu list <playlist>

# Get current playback status
mu status

# Remove track from library and disk
mu remove <track_id|title>
```

### Podcasts

#### Subscribe & Manage
```bash
# Subscribe to podcast
mu podcast subscribe "https://feed.url"
mu podcast subscribe "https://feed.url" --max 10 --no-auto

# List all subscriptions
mu podcast list

# List episodes
mu podcast episodes "Podcast Name"
mu podcast episodes "Podcast Name" --unplayed-only

# Update feeds (auto-runs hourly in daemon)
mu podcast update
mu podcast update --podcast "Podcast Name"

# Configure podcast settings
mu podcast config "Podcast Name" --notify true --auto-download true --max 10

# Unsubscribe
mu podcast unsubscribe "Podcast Name"
mu podcast unsubscribe "Podcast Name" --delete-files
```

#### Playback
```bash
# Play podcast episodes (newest first, unplayed only)
mu play --podcast "Podcast Name"

# Speed control (affects current playback)
mu speed 1.5          # 0.5x - 3.0x

# Progress is automatically tracked every 30s
# Episodes are marked completed when finished
```

#### Storage & Cleanup
```bash
# View storage usage
mu podcast storage

# Set max storage limit (in GB)
mu podcast set-max-storage 10.0

# Cleanup completed episodes (runs automatically on Sundays)
mu podcast cleanup
mu podcast cleanup --dry-run
mu podcast cleanup --force

# Cleanup logic:
# - Removes completed episodes
# - ALWAYS keeps latest episode per podcast
# - ALWAYS keeps unplayed episodes
# - Respects max_episodes setting per podcast
```

#### Statistics
```bash
# Global listening stats
mu podcast stats

# Stats for specific podcast
mu podcast stats "Podcast Name"

# Shows:
# - Total listening time
# - Completed episodes count
# - Completion rate
# - Average listening time
```

## Architecture

```
mu <command>  (CLI, exits immediately)
    ↓ Unix domain socket (~/.../mu/mu.sock)
mu daemon     (background process, auto-spawned by `mu play`)
    ↓ mpsc channel
audio thread  (rodio → CoreAudio)
```

### Modules
- **main.rs** — CLI routing (clap), all command logic
- **daemon.rs** — tokio socket server + audio thread + podcast update loop
- **client.rs** — JSON command sender via Unix socket
- **db.rs** — SQLite with WAL mode, foreign keys, auto-migration
- **downloader.rs** — wraps `yt-dlp` subprocess (MP3 only)
- **podcast.rs** — RSS parser, async episode downloader, DB helpers
- **commands/podcast_commands.rs** — podcast CLI implementations
- **notifications.rs** — desktop notifications via notify-rust

### Daemon Features
- Auto-starts on `mu play`, killed by `mu stop`
- PID stored in `mu.pid`, socket in `mu.sock`
- Hourly podcast feed updates (when podcast subscriptions exist)
- Automatic cleanup runs on Sundays at first update check
- Episode progress tracked every 30s during playback
- Episodes marked completed when `sink.empty()`

## Data Storage

Location: `~/Library/Application Support/mu/`

```
mu.db          SQLite database
mu.sock        Unix domain socket (CLI ↔ daemon)
mu.pid         Daemon process ID
tracks/        Downloaded music (mp3)
podcasts/      Podcast episodes organized by podcast name
```

### Database Schema

#### Music Tables
- `tracks` — id, title, artist, duration_secs, file_path, source_url, added_at
- `playlists` — id, name
- `playlist_tracks` — playlist_id, track_id, position

#### Podcast Tables
- `podcasts` — id, title, author, feed_url, last_checked, auto_download, notify_new_episodes, max_episodes, artwork_url
- `episodes` — id, podcast_id, title, pub_date, duration_secs, file_path, playback_status, playback_progress, is_downloaded, completed_at, guid, file_size_bytes

#### Config
- `config` — key-value store (e.g., max_podcast_storage_bytes)

## JSON Output

All commands output JSON on success:
```json
{"status":"success","message":"...","data":{...}}
```

Errors also return JSON with exit code 1:
```json
{"status":"error","message":"..."}
```

This design enables programmatic control (e.g., via Claude Code, Raycast extensions).

## Raycast Extension

A companion Raycast extension is available in `mu-raycast/`:

### Commands
- **Browse Music** — View library and playlists
- **Browse Podcasts** — Manage subscriptions, browse episodes
- **Playback Controls** — Universal controls with speed adjustment
- **Subscribe to Podcast** — Add new RSS feed
- **Podcast Stats** — View listening statistics

### Features
- Visual episode status (🆕 New, ▶️ Playing, ✅ Done)
- Progress bars for in-progress episodes
- Speed control dropdown (0.5x - 3.0x)
- Auto-refresh status display
- Quick actions for all podcast operations

See `mu-raycast/README.md` for setup instructions.

## Technical Notes

### Audio Constraints
- **MP3 only** — rodio v0.19 doesn't support opus
- **No Send for audio** — rodio's `OutputStream`/`Sink` require dedicated thread
- **Format hardcoded** — in `downloader.rs` for yt-dlp

### Daemon Lifecycle
- Auto-spawns on first `mu play` command
- Socket communication for all state changes
- Survives terminal close (background process)
- Killed explicitly via `mu stop`

### Episode Download Strategy
1. Try `yt-dlp` first (supports YouTube podcast episodes)
2. Fallback to direct HTTP download for MP3/M4A
3. Track file sizes for storage management
4. Organize by podcast name in `podcasts/` directory

### Progress Tracking
- Audio thread counts iterations (300 × 100ms = 30s)
- Calculates progress: `current_track_index / total_tracks`
- Saves to DB via mpsc channel to main async runtime
- Marks completed when `sink.empty()` returns true

### Cleanup Algorithm
- Uses SQL CTE to find completed episodes
- Excludes latest episode per podcast (always kept)
- Excludes unplayed episodes (status != 'completed')
- Respects per-podcast `max_episodes` setting
- Only runs on Sundays unless `--force` used

## Development

### Adding Features

**New CLI command:**
1. Add variant to `Commands` enum in `main.rs`
2. Add match arm in `main()` function

**New daemon command:**
1. Add field to `DaemonCmd` struct (use `#[serde(default)]`)
2. Add match arm in `handle_command()`
3. Add `PlayerMsg` variant if it affects audio state

**New audio format:**
1. Upgrade rodio or add symphonia features in `Cargo.toml`
2. Change format string in `downloader.rs`

### Testing

```bash
# Build and test
cargo build --release

# Test music
mu add "test song"
mu play
mu status

# Test podcasts
mu podcast subscribe "https://example.com/feed.xml"
mu podcast update
mu play --podcast "Podcast Name"
mu podcast stats

# Test daemon
ps aux | grep mu       # check daemon running
cat ~/Library/Application\ Support/mu/mu.pid
```

## License

MIT
