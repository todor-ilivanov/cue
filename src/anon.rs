use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::ui;

// ---------------------------------------------------------------------------
// Anonymous token
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SessionData {
    #[serde(rename = "accessToken")]
    access_token: String,
}

fn fetch_token() -> Result<String> {
    let resp = ureq::get("https://open.spotify.com/search")
        .set(
            "User-Agent",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
        )
        .call()
        .context("failed to fetch Spotify page for anonymous token")?;

    let body = resp
        .into_string()
        .context("failed to read Spotify page body")?;

    let marker = r#"<script id="session" data-testid="session" type="application/json">"#;
    let start = body.find(marker).context(
        "could not find session token in Spotify page — this method may have stopped working",
    )? + marker.len();

    let end = body[start..]
        .find("</script>")
        .context("malformed session script tag")?
        + start;

    let session: SessionData =
        serde_json::from_str(&body[start..end]).context("failed to parse session JSON")?;

    Ok(session.access_token)
}

// ---------------------------------------------------------------------------
// Search types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SearchResponse {
    tracks: Option<TracksPage>,
    albums: Option<AlbumsPage>,
}

#[derive(Deserialize)]
pub struct TracksPage {
    pub items: Vec<TrackItem>,
}

#[derive(Deserialize)]
pub struct TrackItem {
    pub name: String,
    pub id: Option<String>,
    pub uri: String,
    pub artists: Vec<ArtistItem>,
    pub album: AlbumRef,
    pub popularity: u32,
}

#[derive(Deserialize)]
pub struct AlbumsPage {
    pub items: Vec<AlbumItem>,
}

#[derive(Deserialize)]
pub struct AlbumItem {
    pub name: String,
    pub id: Option<String>,
    pub uri: String,
    pub artists: Vec<ArtistItem>,
    pub release_date: Option<String>,
}

#[derive(Deserialize)]
pub struct ArtistItem {
    pub name: String,
}

#[derive(Deserialize)]
pub struct AlbumRef {
    pub name: String,
    pub release_date: Option<String>,
}

pub fn join_artist_names(artists: &[ArtistItem]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Anonymous search
// ---------------------------------------------------------------------------

pub fn search_tracks(query: &str, limit: u32) -> Result<TracksPage> {
    let token = ui::with_spinner("Connecting...", fetch_token)?;

    let resp = ui::with_spinner("Searching...", || {
        ureq::get("https://api.spotify.com/v1/search")
            .set("Authorization", &format!("Bearer {token}"))
            .query("q", query)
            .query("type", "track")
            .query("limit", &limit.to_string())
            .call()
            .context("failed to search tracks")
    })?;

    let body = resp
        .into_string()
        .context("failed to read search response")?;
    let result: SearchResponse =
        serde_json::from_str(&body).context("failed to parse search response")?;

    result.tracks.context("no tracks in search response")
}

pub fn search_albums(query: &str, limit: u32) -> Result<AlbumsPage> {
    let token = ui::with_spinner("Connecting...", fetch_token)?;

    let resp = ui::with_spinner("Searching...", || {
        ureq::get("https://api.spotify.com/v1/search")
            .set("Authorization", &format!("Bearer {token}"))
            .query("q", query)
            .query("type", "album")
            .query("limit", &limit.to_string())
            .call()
            .context("failed to search albums")
    })?;

    let body = resp
        .into_string()
        .context("failed to read search response")?;
    let result: SearchResponse =
        serde_json::from_str(&body).context("failed to parse search response")?;

    result.albums.context("no albums in search response")
}

// ---------------------------------------------------------------------------
// AppleScript playback (macOS only)
// ---------------------------------------------------------------------------

fn osascript(script: &str) -> Result<String> {
    let output = std::process::Command::new("osascript")
        .args(["-e", script])
        .output()
        .context("failed to run osascript — are you on macOS?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Spotify AppleScript error: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn play_uri(uri: &str) -> Result<()> {
    osascript(&format!(
        "tell application \"Spotify\" to play track \"{uri}\""
    ))?;
    Ok(())
}

pub fn pause() -> Result<()> {
    osascript("tell application \"Spotify\" to pause")?;
    println!("Paused");
    Ok(())
}

pub fn resume() -> Result<()> {
    let state = osascript("tell application \"Spotify\" to player state as string")?;
    if state == "playing" {
        println!("Already playing");
        return Ok(());
    }
    osascript("tell application \"Spotify\" to play")?;
    println!("Resumed");
    Ok(())
}

pub fn next() -> Result<()> {
    osascript("tell application \"Spotify\" to next track")?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Ok(info) = now_playing_info() {
        println!(
            "Now playing: {}",
            ui::styled_song(&info.title, &info.artist)
        );
    } else {
        println!("Skipped to next track");
    }
    Ok(())
}

pub fn prev() -> Result<()> {
    osascript("tell application \"Spotify\" to previous track")?;
    std::thread::sleep(std::time::Duration::from_millis(300));
    if let Ok(info) = now_playing_info() {
        println!(
            "Now playing: {}",
            ui::styled_song(&info.title, &info.artist)
        );
    } else {
        println!("Back to previous track");
    }
    Ok(())
}

struct NowPlaying {
    title: String,
    artist: String,
    album: String,
    position_secs: i64,
    duration_secs: i64,
    state: String,
}

fn now_playing_info() -> Result<NowPlaying> {
    let script = r#"tell application "Spotify"
    set trackName to name of current track
    set trackArtist to artist of current track
    set trackAlbum to album of current track
    set trackDuration to duration of current track
    set trackPosition to player position
    set trackState to player state as string
    return trackName & "||" & trackArtist & "||" & trackAlbum & "||" & (trackDuration as string) & "||" & (trackPosition as string) & "||" & trackState
end tell"#;

    let output = osascript(script)?;
    let parts: Vec<&str> = output.splitn(6, "||").collect();
    if parts.len() < 6 {
        bail!("unexpected AppleScript output");
    }

    let duration_ms: f64 = parts[3].parse().unwrap_or(0.0);
    let position: f64 = parts[4].parse().unwrap_or(0.0);

    Ok(NowPlaying {
        title: parts[0].to_string(),
        artist: parts[1].to_string(),
        album: parts[2].to_string(),
        position_secs: position as i64,
        duration_secs: (duration_ms / 1000.0) as i64,
        state: parts[5].to_string(),
    })
}

pub fn now() -> Result<()> {
    let info = now_playing_info()?;

    if info.state == "stopped" {
        println!("Not playing");
        return Ok(());
    }

    if ui::is_interactive() {
        println!("{}", ui::styled_song(&info.title, &info.artist));
        if !info.album.is_empty() {
            println!("{}", console::style(&info.album).dim());
        }
        println!(
            "{}",
            ui::progress_bar(info.position_secs, info.duration_secs)
        );
    } else {
        let album_suffix = if info.album.is_empty() {
            String::new()
        } else {
            format!(" — {}", info.album)
        };
        println!(
            "{}{album_suffix} {}",
            ui::styled_song(&info.title, &info.artist),
            ui::progress_bar(info.position_secs, info.duration_secs)
        );
    }

    Ok(())
}

pub fn get_volume() -> Result<u32> {
    let output = osascript("tell application \"Spotify\" to sound volume")?;
    output.parse().context("failed to parse Spotify volume")
}

pub fn set_volume(level: u8) -> Result<()> {
    osascript(&format!(
        "tell application \"Spotify\" to set sound volume to {level}"
    ))?;
    println!("Volume: {level}%");
    Ok(())
}

// ---------------------------------------------------------------------------
// High-level commands for anonymous mode
// ---------------------------------------------------------------------------

pub fn play(query: &str, album: bool, force_pick: bool) -> Result<()> {
    if album {
        play_album_anon(query, force_pick)
    } else {
        play_track_anon(query, force_pick)
    }
}

fn play_track_anon(query: &str, force_pick: bool) -> Result<()> {
    let tracks = search_tracks(query, 10)?;

    let (indices, candidates): (Vec<usize>, Vec<ui::PickCandidate>) = tracks
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            t.id.as_ref()?;
            Some((
                i,
                ui::PickCandidate {
                    name: t.name.clone(),
                    label: format!("{} — {}", t.name, join_artist_names(&t.artists)),
                    popularity: Some(t.popularity),
                },
            ))
        })
        .unzip();

    let pick = ui::pick_result(query, candidates, "Select a track", force_pick)?;
    let idx = indices[pick];

    let track = &tracks.items[idx];
    let artists = join_artist_names(&track.artists);

    ui::with_spinner("Starting playback...", || play_uri(&track.uri))?;

    println!("Playing: {}", ui::styled_song(&track.name, &artists));
    Ok(())
}

fn play_album_anon(query: &str, force_pick: bool) -> Result<()> {
    let albums = search_albums(query, 10)?;

    let (indices, candidates): (Vec<usize>, Vec<ui::PickCandidate>) = albums
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            a.id.as_ref()?;
            let pop = 10u32.saturating_sub(i as u32) * 10;
            Some((
                i,
                ui::PickCandidate {
                    name: a.name.clone(),
                    label: format!("{} — {}", a.name, join_artist_names(&a.artists)),
                    popularity: Some(pop),
                },
            ))
        })
        .unzip();

    let pick = ui::pick_result(query, candidates, "Select an album", force_pick)?;
    let idx = indices[pick];

    let album = &albums.items[idx];
    let artists = join_artist_names(&album.artists);

    ui::with_spinner("Starting playback...", || play_uri(&album.uri))?;

    println!("Playing album: {}", ui::styled_song(&album.name, &artists));
    Ok(())
}

pub fn search(query: &str, album: bool) -> Result<()> {
    if album {
        search_albums_display(query)
    } else {
        search_tracks_display(query)
    }
}

fn search_tracks_display(query: &str) -> Result<()> {
    let tracks = search_tracks(query, 10)?;

    if tracks.items.is_empty() {
        bail!("no results for \"{query}\"");
    }

    let candidates: Vec<ui::PickCandidate> = tracks
        .items
        .iter()
        .map(|t| ui::PickCandidate {
            name: t.name.clone(),
            label: format!("{} — {}", t.name, join_artist_names(&t.artists)),
            popularity: Some(t.popularity),
        })
        .collect();

    let ranked = ui::rank_candidates(query, &candidates, 5);

    for (display_idx, &(orig_idx, _)) in ranked.iter().enumerate() {
        let track = &tracks.items[orig_idx];
        let artists = join_artist_names(&track.artists);

        let album_info = {
            let name = &track.album.name;
            let year = track.album.release_date.as_deref().and_then(|d| d.get(..4));
            match (name.is_empty(), year) {
                (true, _) => String::new(),
                (false, Some(y)) => format!(" ({name}, {y})"),
                (false, None) => format!(" ({name})"),
            }
        };

        if ui::is_interactive() {
            println!(
                "  {}. {}{}",
                display_idx + 1,
                ui::styled_song(&track.name, &artists),
                console::style(&album_info).dim()
            );
        } else {
            println!(
                "  {}. {} — {}{album_info}",
                display_idx + 1,
                track.name,
                artists
            );
        }
    }

    Ok(())
}

fn search_albums_display(query: &str) -> Result<()> {
    let albums = search_albums(query, 10)?;

    if albums.items.is_empty() {
        bail!("no results for \"{query}\"");
    }

    let candidates: Vec<ui::PickCandidate> = albums
        .items
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let pop = 10u32.saturating_sub(i as u32) * 10;
            ui::PickCandidate {
                name: a.name.clone(),
                label: format!("{} — {}", a.name, join_artist_names(&a.artists)),
                popularity: Some(pop),
            }
        })
        .collect();

    let ranked = ui::rank_candidates(query, &candidates, 5);

    for (display_idx, &(orig_idx, _)) in ranked.iter().enumerate() {
        let album = &albums.items[orig_idx];
        let artists = join_artist_names(&album.artists);

        let year_suffix = match album.release_date.as_deref().and_then(|d| d.get(..4)) {
            Some(y) => format!(" ({y})"),
            None => String::new(),
        };

        if ui::is_interactive() {
            println!(
                "  {}. {}{}",
                display_idx + 1,
                ui::styled_song(&album.name, &artists),
                console::style(&year_suffix).dim()
            );
        } else {
            println!(
                "  {}. {} — {}{year_suffix}",
                display_idx + 1,
                album.name,
                artists
            );
        }
    }

    Ok(())
}

pub fn volume(level: Option<&str>) -> Result<()> {
    match level {
        Some(s) => {
            let input = s.trim();
            let target = if input.starts_with('+') || input.starts_with('-') {
                let current = get_volume()?;
                let delta: i32 = input
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid volume adjustment: {input}"))?;
                (current as i32 + delta).clamp(0, 100) as u8
            } else {
                let level: u32 = input
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid volume level: {input}"))?;
                if level > 100 {
                    bail!("volume must be 0-100, got {level}");
                }
                level as u8
            };
            set_volume(target)
        }
        None => {
            let vol = get_volume()?;
            println!("Volume: {vol}%");
            Ok(())
        }
    }
}
