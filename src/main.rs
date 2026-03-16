use anyhow::Result;
use clap::{Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod ui;

#[derive(Parser)]
#[command(name = "cue", about = "A command-line Spotify remote control")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Play a track, album, or playlist
    Play {
        /// Search query
        query: Vec<String>,
        /// Play an album instead of a track
        #[arg(long)]
        album: bool,
        /// Play a playlist instead of a track
        #[arg(long)]
        playlist: bool,
    },
    /// Pause playback
    Pause,
    /// Resume playback
    Resume,
    /// Skip to the next track
    Next,
    /// Go to the previous track
    Prev,
    /// Show the currently playing track
    Now,
    /// Search for tracks or albums
    Search {
        /// Search query
        query: Vec<String>,
        /// Search for albums instead of tracks
        #[arg(long)]
        album: bool,
    },
    /// List available playback devices
    Devices,
    /// Transfer playback to a device (interactive picker if no name given)
    Device {
        /// Device name (optional — omit for interactive picker)
        name: Option<Vec<String>>,
    },
    /// Set playback volume (0-100)
    Volume {
        /// Volume level (0-100)
        level: u8,
    },
    /// Add a track to the queue
    Queue {
        /// Search query
        query: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let spotify = client::build_client(auth::load_config()?)?;

    match cli.command {
        Command::Play {
            query,
            album,
            playlist,
        } => {
            let query = query.join(" ");
            commands::play::play(&spotify, &query, album, playlist)?;
        }
        Command::Pause => commands::play::pause(&spotify)?,
        Command::Resume => commands::play::resume(&spotify)?,
        Command::Next => commands::play::next(&spotify)?,
        Command::Prev => commands::play::prev(&spotify)?,
        Command::Now => commands::search::now(&spotify)?,
        Command::Search { query, album } => {
            let query = query.join(" ");
            commands::search::search(&spotify, &query, album)?;
        }
        Command::Devices => commands::devices::devices(&spotify)?,
        Command::Device { name } => {
            let name = name.map(|parts| parts.join(" "));
            commands::devices::transfer(&spotify, name.as_deref())?;
        }
        Command::Volume { level } => commands::volume::volume(&spotify, level)?,
        Command::Queue { query } => {
            let query = query.join(" ");
            commands::queue::queue(&spotify, &query)?;
        }
    }

    client::persist_token(&spotify)?;

    Ok(())
}
