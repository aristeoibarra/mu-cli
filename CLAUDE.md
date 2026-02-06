# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`mu` is a minimal, high-performance local music player CLI for macOS. It downloads audio from YouTube (via yt-dlp), stores tracks in SQLite, and plays them via a background daemon using CoreAudio. All output is JSON for automation/scripting.

**Design principles:**
- Minimal resource usage (daemon: ~18MB RAM, 0% CPU idle)
- No UI/TUI — runs in background, controlled via CLI
- JSON output for easy parsing by scripts and AI tools
- Unix-style daemon + client architecture via Unix domain sockets

## Architecture

### Process Model

```
mu CLI (exits immediately)
    ↓ Unix socket
mu daemon (background process)
    ↓ tokio async + dedicated audio thread
rodio → CoreAudio (hardware audio)
```

The daemon spawns automatically on first `mu play` and persists until `mu stop`.

### Module Structure

- **main.rs** - CLI command routing (clap), executes commands synchronously
- **daemon.rs** - Background process: tokio async server + dedicated audio thread (rodio requires single-thread audio)
- **client.rs** - Sends commands to daemon via Unix socket
- **db.rs** - SQLite schema + migrations, auto-creates data dir
- **downloader.rs** - Wraps yt-dlp subprocess, downloads mp3 (rodio doesn't support opus)

### Key Constraints

1. **Audio must run on dedicated OS thread** - rodio's `OutputStream` and `Sink` are not `Send`. The daemon uses `std::thread::spawn` for audio, communicates via `mpsc::channel`.

2. **MP3 only** - rodio v0.19 supports mp3/wav/flac/ogg-vorbis natively. Opus requires `symphonia-opus` feature which isn't in v0.19. Download format hardcoded to mp3.

3. **Daemon lifecycle** - Started by `mu play`, killed by `mu stop`. PID stored in `~/.local/share/mu/mu.pid`. Socket at `mu.sock`.

## Build & Development

```bash
# Build release binary
cargo build --release

# Install to PATH
cp target/release/mu /opt/homebrew/bin/mu

# Run directly (dev)
cargo run -- add "song name"
cargo run -- play
```

## Data Storage

- **Location**: `~/Library/Application Support/mu/` (macOS)
- **Files**:
  - `mu.db` - SQLite database (tracks, playlists, playlist_tracks)
  - `mu.sock` - Unix domain socket for CLI ↔ daemon IPC
  - `mu.pid` - Daemon process ID
  - `tracks/` - Downloaded mp3 files

### Database Schema

```sql
tracks (id, title, artist, duration_secs, file_path, source_url, added_at)
playlists (id, name)
playlist_tracks (playlist_id, track_id, position)
```

Foreign keys cascade on delete. WAL mode enabled for concurrency.

## External Dependencies

- **yt-dlp** (required) - Must be in PATH. Install: `brew install yt-dlp`
- Downloads from YouTube, SoundCloud, 1000+ sites
- Search syntax: plain text becomes `ytsearch1:query`, URLs passed directly

## Common Commands

```bash
# Download and add to playlist
mu add "lofi beats" --playlist work

# Play with shuffle
mu play work --shuffle

# Control playback
mu pause
mu resume
mu skip
mu stop

# Query state (JSON)
mu status
mu list
mu list work
mu playlist list
```

## Adding New Features

### Audio Format Support

To add opus/aac/other formats:
1. Update `Cargo.toml` rodio features or upgrade rodio version
2. Change `downloader.rs:61` from `"mp3"` to desired format
3. Test with `rodio::Decoder::new()` — it auto-detects but requires format support compiled in

### Daemon Commands

New daemon commands require changes in 3 places:
1. `daemon.rs:DaemonCmd` struct - add fields with `#[serde(default)]`
2. `daemon.rs:handle_command()` - add match arm
3. `daemon.rs:PlayerMsg` enum - add variant if it affects audio thread state

### CLI Commands

New CLI commands:
1. `main.rs:Commands` enum - add variant
2. `main.rs:main()` - add match arm with logic

All commands should output JSON on success/error for consistency.
