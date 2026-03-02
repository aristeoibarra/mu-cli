# mu

Minimal local music player CLI for macOS. Downloads audio via yt-dlp (with embedded artwork), stores metadata in SQLite, delegates playback to Apple Music via osascript. All output is JSON.

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

### Download
```bash
mu add "song name"
mu add "song name" --playlist focus
mu add "https://youtube.com/watch?v=..."
```

### Playback
```bash
mu play                        # play all library
mu play focus                  # play playlist
mu play --track "song title"   # play specific track
mu pause
mu resume
mu next
mu previous
mu stop
```

### Library
```bash
mu list                        # all tracks (JSON)
mu list focus                  # tracks in playlist
mu status                      # current playback state
mu remove <track_id|title>     # delete from DB + disk
mu info                        # Apple Music library stats
```

### Playlists
```bash
mu playlist create <name>
mu playlist add <playlist> <track_id|title>
mu playlist remove-track <playlist> <track_id|title>
mu playlist remove <name>
mu playlist list
mu playlist sync               # sync local playlists to Apple Music
```

### Migration
```bash
mu migrate                     # import all tracks to Apple Music
mu migrate --dry-run
mu reimport                    # re-import with updated metadata
mu reimport "song title"
```

## Architecture

```
mu add "song"    → yt-dlp → M4A/AAC 256kbps (with artwork) → imports to Apple Music
mu play          → osascript → Apple Music plays
mu playlist sync → syncs local playlists to Apple Music
```

### Modules
- **main.rs** — CLI routing (clap), command handlers
- **music.rs** — Apple Music integration via osascript
- **db.rs** — SQLite with WAL mode, foreign keys, auto-migration
- **downloader.rs** — wraps `yt-dlp` subprocess (M4A/AAC with embedded thumbnails)

## Data

Location: `~/Library/Application Support/mu/`

```
mu.db          SQLite (tracks, playlists)
tracks/        downloaded m4a files
artwork/       thumbnail images (jpg)
```

## JSON Output

All commands output JSON. Exit code 0 = success, 1 = error.

```json
{"ok": true, "message": "..."}
{"error": "..."}
```

## License

MIT
