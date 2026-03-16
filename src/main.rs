use anyhow::Result;
use clap::{Parser, Subcommand};

mod auth;
mod client;
mod commands;

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
        query: String,
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
        query: String,
        /// Search for albums instead of tracks
        #[arg(long)]
        album: bool,
    },
    /// List available playback devices
    Devices,
    /// Transfer playback to a device
    Device {
        /// Device name or ID
        name: String,
    },
    /// Set playback volume (0-100)
    Volume {
        /// Volume level (0-100)
        level: u8,
    },
    /// Add a track to the queue
    Queue {
        /// Search query
        query: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let spotify = client::build_client(auth::load_config()?)?;

    match cli.command {
        Command::Play { .. } => println!("not yet implemented"),
        Command::Pause => println!("not yet implemented"),
        Command::Resume => println!("not yet implemented"),
        Command::Next => println!("not yet implemented"),
        Command::Prev => println!("not yet implemented"),
        Command::Now => commands::search::now(&spotify)?,
        Command::Search { .. } => println!("not yet implemented"),
        Command::Devices => println!("not yet implemented"),
        Command::Device { .. } => println!("not yet implemented"),
        Command::Volume { .. } => println!("not yet implemented"),
        Command::Queue { .. } => println!("not yet implemented"),
    }

    client::persist_token(&spotify)?;

    Ok(())
}
