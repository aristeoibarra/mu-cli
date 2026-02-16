# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`mu` is a minimal local music player and podcast manager CLI for macOS. Downloads audio via yt-dlp, stores metadata in SQLite, plays via background daemon using CoreAudio. Supports podcast subscriptions with auto-download and notifications. All output is JSON — designed to be controlled by Claude Code via Bash.

## Architecture

```
mu <command>  (CLI, exits immediately)
    ↓ Unix domain socket (~/.../mu/mu.sock)
mu daemon     (background process, auto-spawned by `mu play`)
    ↓ mpsc channel
audio thread  (rodio → CoreAudio)
```

### Modules

- **main.rs** — CLI routing (clap). All command logic lives here.
- **daemon.rs** — tokio async socket server + dedicated audio thread + podcast update loop (every hour). Audio thread required because rodio's `OutputStream`/`Sink` are not `Send`.
- **client.rs** — sends JSON commands to daemon via Unix socket, reads response.
- **db.rs** — SQLite with WAL mode, foreign keys, auto-migration on open.
- **downloader.rs** — wraps `yt-dlp` as subprocess. Downloads MP3 only (rodio v0.19 lacks opus support).
- **podcast.rs** — RSS feed parser (rss crate), async episode downloader (yt-dlp + direct HTTP), DB helpers.
- **commands/podcast_commands.rs** — podcast CLI command implementations.
- **notifications.rs** — desktop notifications via notify-rust.

### Key Constraints

- **MP3 only** — rodio v0.19 doesn't support opus. Format hardcoded in `downloader.rs`.
- **Daemon lifecycle** — auto-starts on `mu play`, killed by `mu stop`. PID in `mu.pid`, socket in `mu.sock`.
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
mu.sock        Unix socket (CLI ↔ daemon IPC)
mu.pid         daemon PID
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
# Download
mu add "song name or URL"              # search YouTube, download mp3
mu add "song" --playlist focus          # download and add to playlist

# Playback (daemon auto-starts)
mu play                                 # play all tracks
mu play focus                           # play playlist
mu play focus --shuffle                 # shuffle order
mu play --podcast "DevTalles"           # play podcast episodes (newest first, unplayed only)
mu play --from <track_id|title>         # start from specific track
mu play focus --from "song name"        # start playlist from specific track
mu pause
mu resume
mu next                                 # skip to next track
mu previous                             # go to previous track (or restart current if at index 0)
mu speed 1.5                            # set playback speed (0.5x - 3.0x)
mu stop                                 # stop and kill daemon

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

**New daemon command:** 3 places — `DaemonCmd` struct (add `#[serde(default)]` fields), `handle_command()` match arm, `PlayerMsg` enum if it touches audio state. All in `daemon.rs`.

**New audio format:** upgrade rodio or add symphonia features in `Cargo.toml`, change format string in `downloader.rs`.

## External Dependency

`yt-dlp` must be in PATH (`brew install yt-dlp`). Plain text queries become `ytsearch1:<query>`, URLs pass through directly.
