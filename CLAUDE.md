# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`mu` is a minimal local music player and podcast manager CLI for macOS. Downloads audio via yt-dlp, stores metadata in SQLite, **delegates playback to Apple Music** via osascript. Supports podcast subscriptions with auto-download and notifications. All output is JSON — designed to be controlled by Claude Code via Bash.

## Architecture

```
mu add "song"              → yt-dlp → MP3 → imports to Apple Music
mu podcast subscribe "url" → RSS → download → stored locally
mu podcast import "name"   → imports episodes to Apple Music
mu play                    → osascript → Apple Music plays
```

### Modules

- **main.rs** — CLI routing (clap). All command logic lives here.
- **music.rs** — Apple Music integration via osascript (import, play, pause, status, playlists).
- **db.rs** — SQLite with WAL mode, foreign keys, auto-migration on open.
- **downloader.rs** — wraps `yt-dlp` as subprocess. Downloads MP3 only.
- **podcast.rs** — RSS feed parser (rss crate), async episode downloader (yt-dlp + direct HTTP), DB helpers.
- **commands/podcast_commands.rs** — podcast CLI command implementations.
- **notifications.rs** — desktop notifications via notify-rust.

### Key Constraints

- **MP3 only** — Format hardcoded in `downloader.rs`.
- **Apple Music for playback** — All audio plays through Apple Music app. No custom audio daemon.
- **All output is JSON** — success and errors. Exit code 0 = ok, 1 = error. This enables Claude Code to parse responses via Bash.

## Build

```bash
cargo build --release
cp target/release/mu /opt/homebrew/bin/mu
```

## Data

Location: `~/Library/Application Support/mu/`

```
mu.db          SQLite (tracks, playlists, episodes, podcasts, config)
tracks/        downloaded mp3 files
podcasts/      podcast episodes organized by podcast name
```

Schema:
- **Music:** `tracks(id, title, artist, duration_secs, file_path, source_url, added_at)`, `playlists(id, name)`, `playlist_tracks(playlist_id, track_id, position)`
- **Podcasts:** `podcasts(id, title, author, feed_url, last_checked, auto_download, notify_new_episodes, max_episodes)`, `episodes(id, podcast_id, title, pub_date, file_path, playback_status, is_downloaded, guid)`
- **Config:** `config(key, value)` - stores settings like max_podcast_storage_bytes

Foreign keys cascade on delete.

## CLI Reference

```bash
# Download & Import
mu add "song name or URL"              # search YouTube, download mp3, import to Apple Music
mu add "song" --playlist focus          # download and add to playlist

# Playback (via Apple Music)
mu play                                 # play in Apple Music
mu play focus                           # play playlist in Apple Music
mu pause
mu resume
mu next                                 # skip to next track
mu previous                             # go to previous track
mu stop                                 # stop playback

# Query (JSON output)
mu status                               # current track, playing/paused state
mu list                                 # all tracks in library
mu list focus                           # tracks in playlist

# Playlists
mu playlist create <name>
mu playlist add <playlist> <track_id|title>
mu playlist remove-track <playlist> <track_id|title>   # remove track from playlist only
mu playlist remove <name>                               # delete entire playlist
mu playlist list

# Library
mu remove <track_id|title>              # delete track from DB + disk

# Podcasts
mu podcast subscribe "https://feed.url" [--max 10] [--no-auto]
mu podcast list                         # list all subscriptions
mu podcast episodes "DevTalles" [--unplayed-only]
mu podcast update [--podcast "name"]    # check for new episodes
mu podcast import "DevTalles"           # import episodes to Apple Music
mu podcast config "DevTalles" --notify true --auto-download true --max 10
mu podcast unsubscribe "DevTalles" [--delete-files]
mu podcast cleanup [--dry-run] [--force]
mu podcast storage                      # show storage usage
mu podcast set-max-storage 10.0         # set max storage in GB
mu podcast stats                        # global listening stats
mu podcast stats "DevTalles"            # stats for specific podcast
```

Track lookup accepts ID (integer) or title substring match.

## Adding Features

**New CLI command:** add variant to `Commands` enum + match arm in `main()` (both in `main.rs`).

**New Apple Music command:** add function to `music.rs` using osascript.

## External Dependencies

- `yt-dlp` must be in PATH (`brew install yt-dlp`). Plain text queries become `ytsearch1:<query>`, URLs pass through directly.
- **Apple Music** must be installed (comes with macOS).

## Raycast Extension

Located in `mu-raycast/`. Provides UI for:
- Browsing music library
- Browsing podcasts and episodes
- Subscribing to new podcasts
- Viewing podcast stats

Playback is handled by Apple Music directly.
