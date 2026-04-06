use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;

use anyhow::{anyhow, bail, Context, Result};
use rspotify::{
    model::{Page, SimplifiedPlaylist},
    prelude::{BaseClient, OAuthClient},
    scopes, AuthCodeSpotify, Credentials, OAuth,
};

use crate::auth::{self, Config};

const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";

fn lock_token(
    spotify: &AuthCodeSpotify,
) -> Result<std::sync::MutexGuard<'_, Option<rspotify::Token>>> {
    spotify
        .token
        .lock()
        .map_err(|_| anyhow!("token lock failed"))
}

pub fn build_client(config: Config) -> Result<AuthCodeSpotify> {
    let creds = Credentials::new(&config.client_id, &config.client_secret);
    let oauth = OAuth {
        redirect_uri: REDIRECT_URI.to_owned(),
        scopes: scopes!(
            "user-read-playback-state",
            "user-modify-playback-state",
            "user-read-currently-playing",
            "user-read-recently-played"
        ),
        ..Default::default()
    };
    let spotify = AuthCodeSpotify::new(creds, oauth);

    if let Some(token) = auth::load_token()? {
        let expired = token.is_expired();
        *lock_token(&spotify)? = Some(token);
        if expired {
            match spotify.refresh_token() {
                Ok(()) => {
                    if let Some(t) = lock_token(&spotify)?.as_ref() {
                        auth::save_token(t)?;
                    }
                }
                Err(_) => {
                    auth::delete_token()?;
                    bail!("token refresh failed — re-run the command to re-authenticate");
                }
            }
        }
        return Ok(spotify);
    }

    // No saved token — run the full OAuth flow.
    let auth_url = spotify
        .get_authorize_url(false)
        .context("could not build authorization URL")?;

    let opened = crate::ui::open_browser(&auth_url).unwrap_or(false);
    if opened {
        eprintln!("Opened browser for authentication.");
    } else {
        eprintln!("Open this URL in your browser to authenticate:\n\n{auth_url}\n");
    }

    let code = crate::ui::with_spinner("Waiting for authentication...", || {
        wait_for_callback(&spotify)
    })?;
    spotify
        .request_token(&code)
        .context("failed to exchange authorization code for token")?;

    if let Some(t) = lock_token(&spotify)?.as_ref() {
        auth::save_token(t)?;
    }

    Ok(spotify)
}

pub fn persist_token(spotify: &AuthCodeSpotify) -> Result<()> {
    if let Some(t) = lock_token(spotify)?.as_ref() {
        auth::save_token(t)?;
    }
    Ok(())
}

fn get_access_token(spotify: &AuthCodeSpotify) -> Result<String> {
    let guard = spotify
        .token
        .lock()
        .map_err(|_| anyhow!("token lock failed"))?;
    Ok(guard
        .as_ref()
        .context("no token available")?
        .access_token
        .clone())
}

/// Search for playlists, filtering out null items that Spotify sometimes
/// returns for unavailable playlists (which cause rspotify parse errors).
pub fn search_playlists(
    spotify: &AuthCodeSpotify,
    query: &str,
    limit: u32,
) -> Result<Page<SimplifiedPlaylist>> {
    let access_token = get_access_token(spotify)?;

    let resp = ureq::get("https://api.spotify.com/v1/search")
        .set("Authorization", &format!("Bearer {access_token}"))
        .query("q", query)
        .query("type", "playlist")
        .query("limit", &limit.to_string())
        .call()
        .context("failed to search for playlist")?;

    let body = resp
        .into_string()
        .context("failed to read search response")?;
    let mut json: serde_json::Value =
        serde_json::from_str(&body).context("failed to parse search response")?;

    if let Some(items) = json.pointer_mut("/playlists/items") {
        if let Some(arr) = items.as_array_mut() {
            arr.retain(|item| !item.is_null());
        }
    }

    serde_json::from_value(json["playlists"].take()).context("failed to parse playlist results")
}

pub struct RadioTrack {
    pub id: String,
}

fn fetch_related_artists(spotify: &AuthCodeSpotify, artist_id: &str) -> Result<Vec<String>> {
    let access_token = get_access_token(spotify)?;

    let resp = ureq::get(&format!(
        "https://api.spotify.com/v1/artists/{artist_id}/related-artists"
    ))
    .set("Authorization", &format!("Bearer {access_token}"))
    .call()
    .context("failed to fetch related artists")?;

    let body = resp
        .into_string()
        .context("failed to read related artists response")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("failed to parse related artists response")?;

    let artists = json["artists"]
        .as_array()
        .context("related artists response missing artists array")?;

    Ok(artists
        .iter()
        .filter_map(|a| a["id"].as_str().map(String::from))
        .collect())
}

fn fetch_artist_top_tracks(spotify: &AuthCodeSpotify, artist_id: &str) -> Result<Vec<RadioTrack>> {
    let access_token = get_access_token(spotify)?;

    let resp = ureq::get(&format!(
        "https://api.spotify.com/v1/artists/{artist_id}/top-tracks"
    ))
    .set("Authorization", &format!("Bearer {access_token}"))
    .query("market", "US")
    .call()
    .context("failed to fetch artist top tracks")?;

    let body = resp
        .into_string()
        .context("failed to read top tracks response")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("failed to parse top tracks response")?;

    let tracks = json["tracks"]
        .as_array()
        .context("top tracks response missing tracks array")?;

    Ok(tracks
        .iter()
        .filter_map(|t| {
            let id = t["id"].as_str()?.to_string();
            Some(RadioTrack { id })
        })
        .collect())
}

/// Build a radio-style track list from related artists' top tracks.
/// Returns track IDs interleaved across artists for variety.
pub fn fetch_radio_tracks(
    spotify: &AuthCodeSpotify,
    artist_id: &str,
    exclude_track_id: &str,
    limit: usize,
) -> Result<Vec<RadioTrack>> {
    let related = fetch_related_artists(spotify, artist_id)?;

    // Seed artist + up to 7 related artists = up to 80 candidate tracks
    let mut artist_ids: Vec<String> = vec![artist_id.to_string()];
    artist_ids.extend(related.into_iter().take(7));

    let mut buckets: Vec<Vec<RadioTrack>> = Vec::new();
    for aid in &artist_ids {
        if let Ok(tracks) = fetch_artist_top_tracks(spotify, aid) {
            if !tracks.is_empty() {
                buckets.push(tracks);
            }
        }
    }

    // Round-robin interleave for artist diversity
    let mut result: Vec<RadioTrack> = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(exclude_track_id.to_string());

    let max_len = buckets.iter().map(|b| b.len()).max().unwrap_or(0);
    for i in 0..max_len {
        for bucket in &buckets {
            if let Some(track) = bucket.get(i) {
                if seen.insert(track.id.clone()) {
                    result.push(RadioTrack {
                        id: track.id.clone(),
                    });
                }
                if result.len() >= limit {
                    return Ok(result);
                }
            }
        }
    }

    Ok(result)
}

fn wait_for_callback(spotify: &AuthCodeSpotify) -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:8888")
        .context("could not listen on 127.0.0.1:8888 for OAuth callback")?;

    let (mut stream, _) = listener
        .accept()
        .context("failed to accept OAuth callback")?;

    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .context("failed to set read timeout on callback stream")?;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .context("failed to read callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h1>Authenticated!</h1><p>You can close this tab.</p></body></html>";
    stream.write_all(response).ok();

    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("malformed HTTP request in OAuth callback")?;

    let url = format!("http://127.0.0.1:8888{path}");
    spotify
        .parse_response_code(&url)
        .context("could not parse authorization code from callback URL")
}
