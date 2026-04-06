use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

mod auth;
mod client;
mod commands;
mod lyrics;
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
    /// Live player with progress bar and keyboard controls
    Player {
        /// Start without lyrics panel
        #[arg(long)]
        slim: bool,
    },
    /// Search for tracks, albums, or artists
    #[command(after_help = "\
Examples:
  cue search bohemian rhapsody
  cue search --album abbey road
  cue search --artist radiohead")]
    Search {
        /// Search query
        query: Vec<String>,
        /// Search for albums instead of tracks
        #[arg(long)]
        album: bool,
        /// Search for artists instead of tracks
        #[arg(long)]
        artist: bool,
    },
    /// List available playback devices
    Devices,
    /// Show active device, or transfer to a named device
    #[command(after_help = "\
Examples:
  cue device
  cue device macbook")]
    Device {
        /// Device name (optional — omit to show active device)
        name: Option<Vec<String>>,
    },
    /// Get or set playback volume (0-100)
    #[command(after_help = "\
Examples:
  cue volume
  cue volume 50
  cue volume +10
  cue volume -10")]
    Volume {
        /// Volume level: 0-100, +N, or -N (omit to show current)
        #[arg(allow_hyphen_values = true)]
        level: Option<String>,
    },
    /// Start a radio based on the currently playing track
    Radio,
    /// Show the queue or add a track to it
    #[command(after_help = "\
Examples:
  cue queue
  cue queue stairway to heaven
  cue queue -p led zeppelin")]
    Queue {
        /// Search query (omit to show current queue)
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

    let needs_device = !matches!(
        cli.command,
        Command::Devices
            | Command::Device { .. }
            | Command::Search { .. }
            | Command::Completions { .. }
    );
    if needs_device {
        commands::devices::ensure_device(&spotify)?;
    }

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
        Command::Player { slim } => commands::player::player(&spotify, slim)?,
        Command::Search {
            query,
            album,
            artist,
        } => {
            let query = query.join(" ");
            commands::search::search(&spotify, &query, album, artist)?;
        }
        Command::Devices => commands::devices::devices(&spotify)?,
        Command::Device { name } => {
            let name = name.map(|parts| parts.join(" "));
            commands::devices::transfer(&spotify, name.as_deref())?;
        }
        Command::Volume { level } => commands::volume::volume(&spotify, level.as_deref())?,
        Command::Radio => commands::radio::radio(&spotify)?,
        Command::Queue { query, pick } => {
            let query = query.join(" ");
            if query.is_empty() {
                commands::queue::queue_show(&spotify)?;
            } else {
                commands::queue::queue_add(&spotify, &query, pick)?;
            }
        }
        Command::Completions { .. } => {} // handled by early return above
    }

    client::persist_token(&spotify)?;

    Ok(())
}
