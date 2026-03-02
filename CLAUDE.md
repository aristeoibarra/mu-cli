# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`mu` is a minimal local music player CLI for macOS. Downloads audio via yt-dlp (with embedded artwork), stores metadata in SQLite, **delegates playback to Apple Music** via osascript. All output is JSON — designed to be controlled by Claude Code via Bash.

## Architecture

```
mu add "song"    → yt-dlp → MP3 (with artwork) → imports to Apple Music
mu play          → osascript → Apple Music plays
mu playlist sync → syncs local playlists to Apple Music
```

### Modules

- **main.rs** — CLI routing (clap). All command logic lives here.
- **music.rs** — Apple Music integration via osascript (import, play, pause, status, playlists).
- **db.rs** — SQLite with WAL mode, foreign keys, auto-migration on open.
- **downloader.rs** — wraps `yt-dlp` as subprocess. Downloads MP3 with embedded thumbnails. Parses artist/album from video metadata.

### Key Constraints

- **MP3 only** — Format hardcoded in `downloader.rs`.
- **Apple Music for playback** — All audio plays through Apple Music app. No custom audio daemon.
- **All output is JSON** — success and errors. Exit code 0 = ok, 1 = error. This enables Claude Code to parse responses via Bash.
- **Artwork embedded** — yt-dlp embeds thumbnails in MP3 files via `--embed-thumbnail`.

## Build

```bash
cargo build --release
cp target/release/mu /opt/homebrew/bin/mu
```

## Data

Location: `~/Library/Application Support/mu/`

```
mu.db          SQLite (tracks, playlists)
tracks/        downloaded mp3 files
artwork/       thumbnail images (jpg)
```

Schema:
- **tracks(id, title, artist, album, duration_secs, file_path, artwork_path, source_url, added_at)**
- **playlists(id, name)**
- **playlist_tracks(playlist_id, track_id, position)**

Foreign keys cascade on delete.

## CLI Reference

```bash
# Download & Import
mu add "song name or URL"              # search YouTube, download mp3 with artwork, import to Apple Music
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

## Adding Features

**New CLI command:** add variant to `Commands` enum + match arm in `main()` (both in `main.rs`).

**New Apple Music command:** add function to `music.rs` using osascript.

## External Dependencies

- `yt-dlp` must be in PATH (`brew install yt-dlp`). Plain text queries become `ytsearch1:<query>`, URLs pass through directly. Uses `--embed-thumbnail` for artwork.
- **Apple Music** must be installed (comes with macOS).
- `curl` for downloading artwork separately.

## Metadata Parsing

The downloader automatically parses artist and album from video titles:
- "Artist - Song Title" → artist: "Artist", title: "Song Title"
- "Artist - Album (Full Album)" → artist: "Artist", album: "Album", title: "Album"
- Cleans up common suffixes: "(Official Video)", "[Official Audio]", etc.
- Falls back to YouTube uploader as artist if no separator found.
