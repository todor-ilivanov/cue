use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod ui;

#[derive(Parser)]
#[command(name = "cue", about = "A command-line Spotify remote control")]
#[command(after_help = "\
Examples:
  cue play starboy
  cue play --album dark side of the moon
  cue now
  cue volume 50
  cue device")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Play a track, album, or playlist
    #[command(after_help = "\
Examples:
  cue play starboy
  cue play --album dark side of the moon
  cue play --playlist discover weekly
  cue play -p radiohead")]
    Play {
        /// Search query
        query: Vec<String>,
        /// Play an album instead of a track
        #[arg(long)]
        album: bool,
        /// Play a playlist instead of a track
        #[arg(long)]
        playlist: bool,
        /// Force interactive picker even when auto-pick would match
        #[arg(short, long)]
        pick: bool,
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
    #[command(after_help = "\
Examples:
  cue search bohemian rhapsody
  cue search --album abbey road")]
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
    #[command(after_help = "\
Examples:
  cue device
  cue device macbook")]
    Device {
        /// Device name (optional — omit for interactive picker)
        name: Option<Vec<String>>,
    },
    /// Set playback volume (0-100)
    #[command(after_help = "\
Examples:
  cue volume 50
  cue volume 0
  cue volume 100")]
    Volume {
        /// Volume level (0-100)
        level: u8,
    },
    /// Add a track to the queue
    #[command(after_help = "\
Examples:
  cue queue stairway to heaven
  cue queue -p led zeppelin")]
    Queue {
        /// Search query
        query: Vec<String>,
        /// Force interactive picker even when auto-pick would match
        #[arg(short, long)]
        pick: bool,
    },
    /// Generate shell completions
    #[command(after_help = "\
Examples:
  cue completions bash >> ~/.bashrc
  cue completions zsh > ~/.zfunc/_cue
  cue completions fish > ~/.config/fish/completions/cue.fish")]
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Command::Completions { shell } = &cli.command {
        clap_complete::generate(*shell, &mut Cli::command(), "cue", &mut std::io::stdout());
        return Ok(());
    }

    let spotify = client::build_client(auth::load_config()?)?;

    match cli.command {
        Command::Play {
            query,
            album,
            playlist,
            pick,
        } => {
            let query = query.join(" ");
            commands::play::play(&spotify, &query, album, playlist, pick)?;
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
        Command::Queue { query, pick } => {
            let query = query.join(" ");
            commands::queue::queue(&spotify, &query, pick)?;
        }
        Command::Completions { .. } => unreachable!(),
    }

    client::persist_token(&spotify)?;

    Ok(())
}
