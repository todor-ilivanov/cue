# cue

A command-line Spotify remote control. It talks to the Spotify Web API to control playback on whatever device is already running. It does not stream audio. It is not a TUI.

## Philosophy

Do one thing and do it perfectly.

Every decision — what to build, what to add, what to leave out — filters through this. If a feature doesn't serve the core purpose of controlling Spotify from the command line, it doesn't belong here. No bloat, no premature abstraction, no feature creep. A focused tool that works flawlessly beats a sprawling one that mostly works.

## Do NOT build

These are explicitly out of scope:

- Full TUI framework (no ratatui, no crossterm) — lightweight live views using `console` are OK
- Direct audio streaming (no librespot)
- Lyrics display
- Playlist management (create, edit, delete)
- Shuffle/repeat toggles

## Tech stack

Rust. These crates, and only these:

| Crate | Purpose |
|-------|---------|
| `rspotify` | Spotify Web API + OAuth |
| `clap` (derive) | CLI subcommands |
| `serde` / `serde_json` | Token serialization |
| `dirs` | Config directory resolution |
| `anyhow` | Error handling |
| `dialoguer` | Arrow-key selection menus |
| `indicatif` | Spinners during API calls |
| `console` | Colors, terminal detection |
| `fuzzy-matcher` | Fuzzy ranking of search results |

Do not add dependencies without justification. If the standard library can do it, use the standard library.

## Project structure

```
src/
├── main.rs              # Entry point, clap CLI definition
├── auth.rs              # OAuth flow, token persistence, config loading, device memory
├── client.rs            # Authenticated rspotify client construction
├── ui.rs                # Terminal interaction: spinners, styled output, selection, browser open
└── commands/
    ├── mod.rs
    ├── play.rs          # play, pause, resume, next, prev (fuzzy search + interactive select)
    ├── player.rs        # player (live now-playing view with keyboard controls)
    ├── search.rs        # search, now (styled output)
    ├── devices.rs       # devices, device (smart picker with memory)
    ├── volume.rs        # volume
    └── queue.rs         # queue (fuzzy search + interactive select)
```

## Commands

```
cargo build              # Build
cargo run -- <command>   # Run a subcommand
cargo clippy             # Lint (must pass with no warnings)
cargo fmt --check        # Format check
```

## UX principles

- Minimal typing: no quotes needed for multi-word queries, fuzzy matching, auto-selection when unambiguous
- Interactive when helpful: arrow-key pickers for search results and device selection
- Graceful degradation: no interactivity or color when piped (non-TTY)
- No emoji. Subtle color only: bold titles, dim metadata
- Spinners on stderr, output on stdout

## Code style

- No `unwrap()` outside tests. Use `?` and `anyhow`.
- Functions do one thing. Keep them short.
- No unnecessary abstractions — this is a CLI tool, not a framework.
- Use Rust idioms: pattern matching, `?` operator, iterators.
- Minimal comments. Code should be self-explanatory. Comment only when *why* isn't obvious.

## Auth & config

- Config dir: `dirs::config_dir()/cue/` (`~/.config/cue/` on Linux, `~/Library/Application Support/cue/` on macOS)
- Config: `config.toml` (client_id, client_secret)
- Token: `token.json` (0600 permissions)
- Last device: `last_device` (0600 permissions, plain text device ID)
- OAuth redirect: `http://127.0.0.1:8888/callback`
- Auth flow: Authorization Code (not PKCE — client secret stays local)
- Any command with no saved token triggers OAuth automatically
- Auth auto-opens browser when possible, falls back to printing URL
- Persist updated token after every command

## Error handling

- `anyhow` for propagation, human-readable messages to stderr.
- No active device → list available devices, suggest `cue device <name>`.
- Token refresh fails → delete token file, prompt re-auth.
- No search results → clear message, not a panic.
- Network errors → print underlying message, suggest checking connectivity.
- Device remembered but gone → fall back to interactive picker or active device.
