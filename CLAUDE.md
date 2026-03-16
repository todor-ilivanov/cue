# cue

A command-line Spotify remote control. It talks to the Spotify Web API to control playback on whatever device is already running. It does not stream audio. It is not a TUI.

## Philosophy

Do one thing and do it perfectly.

Every decision — what to build, what to add, what to leave out — filters through this. If a feature doesn't serve the core purpose of controlling Spotify from the command line, it doesn't belong here. No bloat, no premature abstraction, no feature creep. A focused tool that works flawlessly beats a sprawling one that mostly works.

## Do NOT build

These are explicitly out of scope:

- TUI or interactive terminal UI (no ratatui, no crossterm)
- Direct audio streaming (no librespot)
- Lyrics display
- Playlist management (create, edit, delete)
- Shuffle/repeat toggles
- Fuzzy search or interactive selection
- Anything that requires crates beyond the core set below

## Tech stack

Rust. These crates, and only these:

| Crate | Purpose |
|-------|---------|
| `rspotify` | Spotify Web API + OAuth |
| `clap` (derive) | CLI subcommands |
| `tokio` | Async runtime |
| `serde` / `serde_json` | Token serialization |
| `dirs` | Config directory resolution |
| `anyhow` | Error handling |

Do not add dependencies without justification. If the standard library can do it, use the standard library.

## Project structure

```
src/
├── main.rs              # Entry point, clap CLI definition
├── auth.rs              # OAuth flow, token persistence, config loading
├── client.rs            # Authenticated rspotify client construction
└── commands/
    ├── mod.rs
    ├── play.rs          # play, pause, resume, next, prev
    ├── search.rs        # search, now
    ├── devices.rs       # devices, device (transfer)
    ├── volume.rs        # volume
    └── queue.rs         # queue
```

## Commands

```
cargo build              # Build
cargo run -- <command>   # Run a subcommand
cargo clippy             # Lint (must pass with no warnings)
cargo fmt --check        # Format check
```

## Code style

- No `unwrap()` outside tests. Use `?` and `anyhow`.
- Functions do one thing. Keep them short.
- No unnecessary abstractions — this is a CLI tool, not a framework.
- Use Rust idioms: pattern matching, `?` operator, iterators.
- Minimal comments. Code should be self-explanatory. Comment only when *why* isn't obvious.

## Auth & config

- Config: `~/.config/cue/config.toml` (client_id, client_secret)
- Token: `~/.config/cue/token.json` (0600 permissions)
- OAuth redirect: `http://127.0.0.1:8888/callback`
- Auth flow: Authorization Code (not PKCE — client secret stays local)
- Any command with no saved token triggers OAuth automatically
- Persist updated token after every command

## Error handling

- `anyhow` for propagation, human-readable messages to stderr.
- No active device → list available devices, suggest `cue device <name>`.
- Token refresh fails → delete token file, prompt re-auth.
- No search results → clear message, not a panic.
- Network errors → print underlying message, suggest checking connectivity.
