mod commands;
mod db;
mod downloader;
mod error;
mod music;

use crate::commands::favorites::FavAction;
use crate::commands::playlist::PlaylistAction;
use crate::commands::plays::PlaysAction;
use clap::{Parser, Subcommand};
use error::json_error;

#[derive(Parser)]
#[command(
    name = "mu",
    about = "Local music player CLI - Downloads and imports to Apple Music"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a track and import to Apple Music
    Add {
        /// Song name or URL
        query: String,
        /// Add directly to a playlist
        #[arg(short, long)]
        playlist: Option<String>,
    },
    /// Play tracks in Apple Music
    Play {
        /// Playlist name (plays all library if omitted)
        playlist: Option<String>,
        /// Track name to play
        #[arg(short, long)]
        track: Option<String>,
    },
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Skip to next track
    Next,
    /// Go to previous track
    Previous,
    /// Stop playback
    Stop,
    /// Show current status
    Status,
    /// List tracks in local database
    List {
        /// Playlist name to list tracks from
        playlist: Option<String>,
    },
    /// Manage playlists
    Playlist {
        #[command(subcommand)]
        action: PlaylistAction,
    },
    /// Remove a track from library
    Remove {
        /// Track ID or title substring
        track: String,
    },
    /// Migrate existing tracks to Apple Music
    Migrate {
        /// Only show what would be migrated
        #[arg(long)]
        dry_run: bool,
    },
    /// Show Apple Music library info
    Info,
    /// Sync favorites and play counts from Apple Music
    Sync,
    /// Re-import track with updated metadata
    Reimport {
        /// Track ID or title substring (all if omitted)
        track: Option<String>,
    },
    /// Manage favorites
    Fav {
        #[command(subcommand)]
        action: FavAction,
    },
    /// View play counts
    Plays {
        #[command(subcommand)]
        action: PlaysAction,
    },
}

fn main() {
    let cli = Cli::parse();
    let data_dir = match db::data_dir() {
        Ok(d) => d,
        Err(e) => {
            println!("{}", json_error(&e.to_string()));
            std::process::exit(1);
        }
    };
    let db_path = data_dir.join("mu.db");

    let result = match cli.command {
        Commands::Add { query, playlist } => commands::handle_add(&db_path, &query, playlist),
        Commands::Play { playlist, track } => {
            commands::handle_play(playlist.as_deref(), track.as_deref())
        }
        Commands::Pause => commands::handle_pause(),
        Commands::Resume => commands::handle_resume(),
        Commands::Next => commands::handle_next(),
        Commands::Previous => commands::handle_previous(),
        Commands::Stop => commands::handle_stop(),
        Commands::Status => commands::handle_status(&db_path),
        Commands::List { playlist } => commands::handle_list(&db_path, playlist.as_deref()),
        Commands::Playlist { action } => commands::handle_playlist_action(&db_path, action),
        Commands::Remove { track } => commands::handle_remove(&db_path, &track),
        Commands::Migrate { dry_run } => commands::handle_migrate(&db_path, dry_run),
        Commands::Info => commands::handle_info(),
        Commands::Sync => commands::handle_sync(&db_path),
        Commands::Reimport { track } => commands::handle_reimport(&db_path, track.as_deref()),
        Commands::Fav { action } => commands::handle_fav_action(&db_path, action),
        Commands::Plays { action } => commands::handle_plays_action(&db_path, action),
    };

    if let Err(e) = result {
        println!("{}", json_error(&e.to_string()));
        std::process::exit(1);
    }
}
