# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`mu` is a minimal local music player CLI for macOS. Downloads audio via yt-dlp (with embedded artwork), stores metadata in SQLite, **delegates playback to Apple Music** via osascript. All output is JSON — designed to be controlled by Claude Code via Bash.

## Architecture

```
mu add "song"    → yt-dlp → M4A/AAC 256kbps → iTunes artwork (fallback: YouTube thumbnail) → imports to Apple Music
mu play          → osascript → Apple Music plays
mu playlist sync → syncs local playlists to Apple Music
```

### Modules

- **main.rs** — CLI definition (clap `Commands` enum) + dispatch to command handlers.
- **commands/** — Command handlers, split by domain:
  - `add.rs` — download + import + optional playlist add.
  - `playback.rs` — play, pause, resume, next, previous, stop.
  - `library.rs` — list, remove, status, info, migrate, reimport.
  - `playlist.rs` — CRUD + sync. Defines `PlaylistAction` subcommand enum.
- **music.rs** — Apple Music integration via osascript (import, play, pause, status, playlists, delete tracks, sync).
- **db.rs** — SQLite with WAL mode, foreign keys, auto-migration on open. Track/playlist resolution helpers. Stores Apple Music persistent IDs for reliable sync.
- **downloader.rs** — wraps `yt-dlp` as subprocess. Downloads M4A/AAC with embedded thumbnails. Fetches high-quality artwork from iTunes Search API (falls back to YouTube thumbnail). Parses artist/album from video metadata. Updates metadata + embeds artwork via `ffmpeg`.
- **error.rs** — `MuError` enum (thiserror), `Result<T>` alias, `json_error()`/`json_ok()` helpers.

### Key Constraints

- **M4A/AAC only** — Format hardcoded in `downloader.rs` (256 kbps, iTunes Store quality).
- **Apple Music for playback** — All audio plays through Apple Music app. No custom audio daemon.
- **All output is JSON** — success and errors. Exit code 0 = ok, 1 = error. This enables Claude Code to parse responses via Bash.
- **Artwork embedded** — iTunes Search API provides high-quality album art (1200x1200). Falls back to YouTube thumbnails via `--embed-thumbnail`.

## Build & Test

```bash
cargo build --release
cp target/release/mu /opt/homebrew/bin/mu

# Tests (unit tests in downloader.rs for metadata parsing)
cargo test
cargo test parse_artist       # run single test by name

# Linting (strict config in Cargo.toml: clippy::all = deny, pedantic = warn)
cargo clippy
```

## Data

Location: `~/Library/Application Support/mu/`

```
mu.db          SQLite (tracks, playlists)
tracks/        downloaded m4a files
artwork/       thumbnail images (jpg)
```

Schema:
- **tracks(id, title, artist, album, duration_secs, file_path, artwork_path, source_url, added_at, apple_music_id)**
- **playlists(id, name)**
- **playlist_tracks(playlist_id, track_id, position)**

Foreign keys cascade on delete. `apple_music_id` stores the Apple Music persistent ID for reliable sync (populated on add/reimport/migrate).

## CLI Reference

```bash
# Download & Import
mu add "song name or URL"              # search YouTube, download m4a with artwork, import to Apple Music
mu add "song" --playlist focus         # download and add to playlist

# Playback (via Apple Music)
mu play                                # play in Apple Music
mu play focus                          # play playlist in Apple Music
mu play --track "song name"            # play specific track
mu pause
mu resume
mu next                                # skip to next track
mu previous                            # go to previous track
mu stop                                # stop playback

# Query (JSON output)
mu status                              # current track, artist, album, playing/paused state
mu list                                # all tracks in library (with artwork paths)
mu list focus                          # tracks in playlist
mu info                                # Apple Music library stats

# Playlists
mu playlist create <name>
mu playlist add <playlist> <track_id|title>
mu playlist remove-track <playlist> <track_id|title>
mu playlist remove <name>
mu playlist list
mu playlist sync                       # sync local playlists to Apple Music

# Library
mu remove <track_id|title>             # delete track from DB + disk

# Migration & Maintenance
mu migrate [--dry-run]                 # import all tracks to Apple Music
mu reimport [track]                    # re-import track(s) with updated metadata
```

Track lookup accepts ID (integer) or title substring match.

## Apple Music Sync

SQLite is the source of truth. Apple Music is kept in sync via persistent IDs:

- **`mu add`** — imports to Apple Music, stores persistent ID in DB
- **`mu remove`** — deletes from Apple Music (by persistent ID, fallback to file path), then from DB + disk
- **`mu playlist add`** — adds to Apple Music playlist (by persistent ID, fallback to name)
- **`mu playlist remove-track`** — removes from Apple Music playlist, then from DB
- **`mu playlist remove`** — deletes playlist from Apple Music + DB
- **`mu playlist sync`** — bidirectional: adds missing tracks, removes extras from Apple Music playlists
- **`mu reimport`** / **`mu migrate`** — backfills persistent IDs for existing tracks

## Adding Features

**New CLI command:** add variant to `Commands` enum in `main.rs`, create handler in the appropriate `commands/*.rs` file, re-export from `commands/mod.rs`, add match arm in `main()`.

**New Apple Music command:** add function to `music.rs` using osascript.

## External Dependencies

- `yt-dlp` must be in PATH (`brew install yt-dlp`). Plain text queries become `ytsearch1:<query>`, URLs pass through directly. Uses `--embed-thumbnail` for artwork.
- **Apple Music** must be installed (comes with macOS).
- `curl` for downloading artwork (iTunes Search API + YouTube thumbnails).
- `ffmpeg` for updating metadata and embedding artwork in M4A files (optional — fails silently if missing).
- **iTunes Search API** (free, no auth) for high-quality album artwork (1200x1200). Falls back to YouTube thumbnails if no match found.

## Metadata Parsing

The downloader automatically parses artist and album from video titles:
- "Artist - Song Title" → artist: "Artist", title: "Song Title"
- "Artist - Album (Full Album)" → artist: "Artist", album: "Album", title: "Album"
- Cleans up common suffixes: "(Official Video)", "[Official Audio]", etc.
- Falls back to YouTube uploader as artist if no separator found.
