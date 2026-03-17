# Facelift v2: CLI UX Improvements

**Context:** Research on world-class CLI tools (fzf, ripgrep, fd, bat, gh, tldr) identifies patterns that make CLIs delightful: example-first help, shell completions for discoverability, information-dense output, and visual feedback. The cue CLI already has strong foundations (fuzzy search, spinners, styled output, graceful degradation). These 4 improvements close the remaining gaps.

---

## Improvement 1: Richer Search & Now-Playing Output [Done]

**Goal:** Show album name and release year alongside tracks — more information-dense output without being noisy.

**Files modified:**
- `src/commands/mod.rs` — added `release_year()` helper
- `src/commands/search.rs` — updated `search_tracks()`, `search_albums()`, `now()`

### Changes

**`src/commands/mod.rs`** — added helper:
```rust
pub fn release_year(date: Option<&str>) -> Option<&str> {
    date.and_then(|d| d.get(..4))
}
```

**`search_tracks()`** — single-line format with dim album info:
```
  1. Bohemian Rhapsody — Queen (A Night at the Opera, 1975)
```
Album name and release year appended inline, dim-styled when interactive, plain when piped. Year omitted if unavailable; entire parenthetical omitted if album name is empty.

**`search_albums()`** — inline year:
```
  1. A Night at the Opera — Queen (1975)
```
Year appended dim-styled after the artist. Omitted if no release date.

**`now()`** — album name appended inline for tracks (not episodes):
```
Bohemian Rhapsody — Queen (A Night at the Opera) [1:23 / 3:45]
```

### Verification
- `cue search bohemian rhapsody` — inline track results with album + year
- `cue search --album abbey road` — album results with year in parens
- `cue search bohemian | cat` — same info, no ANSI codes
- No empty parens when API returns no release_date
- `cargo clippy` passes

---

## Improvement 2: Visual Progress Bar for `cue now` [Done]

**Goal:** Replace `[1:23 / 3:45]` with a visual progress bar that makes playback state scannable at a glance.

**Files to modify:**
- `src/ui.rs` — add `format_duration()` and `progress_bar()`
- `src/commands/search.rs` — restructure `now()` output

**Depends on:** Improvement 1 (album name extraction in `now()`), but can be implemented independently with minor adjustment.

### Changes

**`src/ui.rs`** — add shared duration formatter (move from `search.rs`):
```rust
pub fn format_duration(total_secs: i64) -> String {
    let total_secs = total_secs.max(0);
    format!("{}:{:02}", total_secs / 60, total_secs % 60)
}
```

**`src/ui.rs`** — add progress bar builder:
```rust
pub fn progress_bar(progress_secs: i64, total_secs: i64) -> String
```
- Get terminal width via `console::Term::stderr().size()` (already a dependency)
- Bar width = terminal cols minus time labels minus 2 spaces, clamped to 10..50
- Filled: `━` (U+2501), empty: `─` (U+2500), empty portion dim-styled
- Ratio = `progress / total`, clamped 0.0..1.0, handle `total == 0`
- Non-interactive fallback: `[1:23 / 3:45]`

**`src/commands/search.rs`** — restructure `now()`:

Interactive (3 lines):
```
Bohemian Rhapsody — Queen
A Night at the Opera
2:15 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━──────────── 5:55
```

Piped (1 line):
```
Bohemian Rhapsody — Queen — A Night at the Opera [2:15 / 5:55]
```

Remove local `format_duration_secs`, use `ui::format_duration` instead.

### Verification
- `cue now` — 3-line output with progress bar in terminal
- `cue now | cat` — single line, no ANSI, bracket time format
- Progress bar fills proportionally (test mid-track)
- Bar width adapts to terminal width
- `cargo clippy` passes

---

## Improvement 3: Example-First Help Text [Done]

**Goal:** Add concrete, copy-pasteable usage examples to `--help` output (tldr pattern). New users should understand the tool from help alone.

**Files to modify:**
- `src/main.rs` — add `after_help` attributes

### Changes

Add `#[command(after_help = "...")]` to `Cli` struct and subcommand variants that take arguments.

**Top-level `Cli`:**
```
Examples:
  cue play starboy
  cue play --album dark side of the moon
  cue now
  cue volume 50
  cue device
```

**`Play`:**
```
Examples:
  cue play starboy
  cue play --album dark side of the moon
  cue play --playlist discover weekly
  cue play -p radiohead
```

**`Search`:**
```
Examples:
  cue search bohemian rhapsody
  cue search --album abbey road
```

**`Device`:**
```
Examples:
  cue device
  cue device macbook
```

**`Volume`:**
```
Examples:
  cue volume 50
  cue volume 0
  cue volume 100
```

**`Queue`:**
```
Examples:
  cue queue stairway to heaven
  cue queue -p led zeppelin
```

**No examples for:** `Pause`, `Resume`, `Next`, `Prev`, `Now`, `Devices` — no meaningful arguments.

### Verification
- `cue --help` — examples at bottom
- `cue play --help` — play-specific examples after flags section
- `cue pause --help` — no examples section
- `cargo clippy` passes

---

## Improvement 4: Shell Completions [Done]

**Goal:** Tab-completable commands and flags via `cue completions <shell>`.

**Files to modify:**
- `Cargo.toml` — add `clap_complete = "4"`
- `src/main.rs` — add `Completions` subcommand, restructure `main()` for early return

**New dependency:** `clap_complete` — part of the clap ecosystem, justified for shell completions.

### Changes

**`Cargo.toml`:**
```toml
clap_complete = "4"
```

**`src/main.rs`** — add subcommand variant:
```rust
/// Generate shell completions
#[command(after_help = "\
Examples:
  cue completions bash >> ~/.bashrc
  cue completions zsh > ~/.zfunc/_cue
  cue completions fish > ~/.config/fish/completions/cue.fish")]
Completions {
    /// Shell to generate completions for
    shell: clap_complete::Shell,
}
```

`clap_complete::Shell` implements `ValueEnum`, so clap auto-accepts `bash`, `zsh`, `fish`, `elvish`, `powershell`.

**`src/main.rs`** — add `use clap::CommandFactory;` and early-return before auth:
```rust
fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Command::Completions { shell } = &cli.command {
        clap_complete::generate(*shell, &mut Cli::command(), "cue", &mut std::io::stdout());
        return Ok(());
    }

    let spotify = client::build_client(auth::load_config()?)?;
    // ... rest unchanged
}
```

Early return is critical — generating completions must not require Spotify credentials.

### Verification
- `cue completions bash` — outputs bash completion script (no auth needed)
- `cue completions zsh` — outputs zsh completion script
- `cue completions fish` — outputs fish completion script
- `cue completions --help` — shows shell options and examples
- Works without `~/.config/cue/config.toml` existing
- `cargo clippy` passes

---

## Summary of All File Changes

| File | Improvements | What |
|------|-------------|------|
| `Cargo.toml` | 4 | Add `clap_complete = "4"` |
| `src/main.rs` | 3, 4 | `after_help` on `Cli` + 6 variants, `Completions` subcommand, early-return, `use clap::CommandFactory` |
| `src/ui.rs` | 2 | Add `format_duration()`, `progress_bar()` |
| `src/commands/mod.rs` | 1 | Add `release_year()` |
| `src/commands/search.rs` | 1, 2 | Richer `now()` with album + progress bar, richer `search_tracks()` and `search_albums()` |

**Untouched:** `play.rs`, `queue.rs`, `devices.rs`, `volume.rs`, `auth.rs`, `client.rs`
