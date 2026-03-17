# cue

A command-line Spotify remote control. It talks to the Spotify Web API to control playback on whatever device is already running. It does not stream audio.

## Install

```bash
git clone https://github.com/todor-ilivanov/cue.git
cd cue
cargo build --release
```

Optionally, copy the binary to your PATH:

```bash
cp target/release/cue ~/.local/bin/
```

## Setup

**1. Create a Spotify app:**

- Go to https://developer.spotify.com/dashboard
- Create a new app
- Set the redirect URI to `http://127.0.0.1:8888/callback`
- Note your Client ID and Client Secret

**2. Create the config file:**

The config directory depends on your OS:

| OS    | Path                                      |
|-------|-------------------------------------------|
| Linux | `~/.config/cue/`                          |
| macOS | `~/Library/Application Support/cue/`      |

Create it and add your credentials to `config.toml`:

```toml
[spotify]
client_id = "your_client_id"
client_secret = "your_client_secret"
```

**3. Authenticate:**

Run any command (e.g. `cue devices`). Your browser will open automatically for Spotify OAuth. After authorizing, the token is saved automatically.

## Usage

Spotify must be open on at least one device (phone, desktop app, web player). `cue` is a remote control — it doesn't play audio itself.

No quotes needed around multi-word queries. When multiple results match, an interactive picker lets you choose.

```
cue play <query>            Play a track (fuzzy search, interactive pick)
cue play --album <query>    Play an album
cue play --playlist <query> Play a playlist
cue pause                   Pause playback
cue resume                  Resume playback
cue next                    Skip to next track
cue prev                    Go to previous track
cue now                     Show what's currently playing
cue search <query>          Search for tracks
cue search --album <query>  Search for albums
cue devices                 List available devices
cue device                  Interactive device picker
cue device <name>           Transfer playback to a device by name
cue volume <0-100>          Set volume
cue queue <query>           Add a track to the queue
```

### Example

```bash
cue devices
cue device MacBook
cue play bohemian rhapsody
cue now
cue volume 50
cue next
cue queue another one bites the dust
cue search --album abbey road
```
