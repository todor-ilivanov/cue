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

fn join_artist_names_json(value: &serde_json::Value) -> String {
    value["artists"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["name"].as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Credit information for a track (assembled from track + album endpoints).
pub struct TrackCredits {
    pub performers: Vec<String>,
    pub album: String,
    pub album_artists: Vec<String>,
    pub release_date: Option<String>,
    pub label: Option<String>,
    pub copyrights: Vec<String>,
    pub isrc: Option<String>,
}

/// Fetch credit information for a track by combining track and album details.
pub fn fetch_track_credits(spotify: &AuthCodeSpotify, track_id: &str) -> Result<TrackCredits> {
    let access_token = get_access_token(spotify)?;

    // Fetch full track to get album ID, ISRC, and artist list
    let resp = ureq::get(&format!("https://api.spotify.com/v1/tracks/{track_id}"))
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .context("failed to fetch track details")?;

    let body = resp
        .into_string()
        .context("failed to read track response")?;
    let track_json: serde_json::Value =
        serde_json::from_str(&body).context("failed to parse track response")?;

    let performers: Vec<String> = track_json["artists"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let album_name = track_json["album"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let album_artists: Vec<String> = track_json["album"]["artists"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let release_date = track_json["album"]["release_date"]
        .as_str()
        .map(String::from);

    let isrc = track_json["external_ids"]["isrc"]
        .as_str()
        .map(String::from);

    let album_id = track_json["album"]["id"].as_str().unwrap_or_default();

    // Fetch full album for label and copyrights
    let (label, copyrights) = if !album_id.is_empty() {
        let resp = ureq::get(&format!("https://api.spotify.com/v1/albums/{album_id}"))
            .set("Authorization", &format!("Bearer {access_token}"))
            .call();

        match resp {
            Ok(resp) => {
                let body = resp.into_string().unwrap_or_default();
                let album_json: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();

                let label = album_json["label"].as_str().map(String::from);
                let copyrights = album_json["copyrights"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| c["text"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                (label, copyrights)
            }
            Err(_) => (None, Vec::new()),
        }
    } else {
        (None, Vec::new())
    };

    Ok(TrackCredits {
        performers,
        album: album_name,
        album_artists,
        release_date,
        label,
        copyrights,
        isrc,
    })
}

/// A track within an album or playlist context.
pub struct ContextTrack {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: String,
}

/// Fetch all tracks from an album.
pub fn fetch_album_tracks(spotify: &AuthCodeSpotify, album_id: &str) -> Result<Vec<ContextTrack>> {
    let access_token = get_access_token(spotify)?;
    let mut tracks = Vec::new();
    let mut offset = 0u32;
    let limit = 50u32;

    loop {
        let resp = ureq::get(&format!(
            "https://api.spotify.com/v1/albums/{album_id}/tracks"
        ))
        .set("Authorization", &format!("Bearer {access_token}"))
        .query("limit", &limit.to_string())
        .query("offset", &offset.to_string())
        .call()
        .context("failed to fetch album tracks")?;

        let body = resp
            .into_string()
            .context("failed to read album tracks response")?;
        let json: serde_json::Value =
            serde_json::from_str(&body).context("failed to parse album tracks response")?;

        let items = json["items"]
            .as_array()
            .context("album tracks response missing items array")?;

        if items.is_empty() {
            break;
        }

        for t in items {
            let id = match t["id"].as_str() {
                Some(id) => id.to_string(),
                None => continue,
            };
            let uri = t["uri"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("spotify:track:{id}"));
            let name = t["name"].as_str().unwrap_or_default().to_string();
            let artists = join_artist_names_json(t);
            tracks.push(ContextTrack {
                id,
                uri,
                name,
                artists,
            });
        }

        let total = json["total"].as_u64().unwrap_or(0) as u32;
        offset += limit;
        if offset >= total {
            break;
        }
    }

    Ok(tracks)
}

/// Fetch tracks from a playlist (up to 500).
pub fn fetch_playlist_tracks(
    spotify: &AuthCodeSpotify,
    playlist_id: &str,
) -> Result<Vec<ContextTrack>> {
    let access_token = get_access_token(spotify)?;
    let mut tracks = Vec::new();
    let mut offset = 0u32;
    let limit = 100u32;

    loop {
        let resp = ureq::get(&format!(
            "https://api.spotify.com/v1/playlists/{playlist_id}/tracks"
        ))
        .set("Authorization", &format!("Bearer {access_token}"))
        .query("limit", &limit.to_string())
        .query("offset", &offset.to_string())
        .query("fields", "items(track(id,uri,name,artists(name))),total")
        .call()
        .context("failed to fetch playlist tracks")?;

        let body = resp
            .into_string()
            .context("failed to read playlist tracks response")?;
        let json: serde_json::Value =
            serde_json::from_str(&body).context("failed to parse playlist tracks response")?;

        let items = json["items"]
            .as_array()
            .context("playlist tracks response missing items array")?;

        if items.is_empty() {
            break;
        }

        for item in items {
            let t = &item["track"];
            if t.is_null() {
                continue;
            }
            let id = match t["id"].as_str() {
                Some(id) => id.to_string(),
                None => continue,
            };
            let uri = t["uri"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("spotify:track:{id}"));
            let name = t["name"].as_str().unwrap_or_default().to_string();
            let artists = join_artist_names_json(t);
            tracks.push(ContextTrack {
                id,
                uri,
                name,
                artists,
            });
        }

        let total = json["total"].as_u64().unwrap_or(0) as u32;
        offset += limit;
        if offset >= total || tracks.len() >= 500 {
            break;
        }
    }

    Ok(tracks)
}

pub struct RadioTrack {
    pub id: String,
}

fn fetch_related_artists(access_token: &str, artist_id: &str) -> Result<Vec<String>> {
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

/// A top track with display metadata.
pub struct ArtistTopTrack {
    pub id: String,
    pub name: String,
    pub artists: String,
}

fn fetch_artist_top_tracks_raw(access_token: &str, artist_id: &str) -> Result<Vec<ArtistTopTrack>> {
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
            let name = t["name"].as_str()?.to_string();
            let artists = join_artist_names_json(t);
            Some(ArtistTopTrack { id, name, artists })
        })
        .collect())
}

/// Fetch an artist's top tracks with full metadata (name + artists).
pub fn fetch_artist_top_tracks_full(
    spotify: &AuthCodeSpotify,
    artist_id: &str,
) -> Result<Vec<ArtistTopTrack>> {
    let access_token = get_access_token(spotify)?;
    fetch_artist_top_tracks_raw(&access_token, artist_id)
}

fn fetch_artist_top_tracks(access_token: &str, artist_id: &str) -> Result<Vec<RadioTrack>> {
    Ok(fetch_artist_top_tracks_raw(access_token, artist_id)?
        .into_iter()
        .map(|t| RadioTrack { id: t.id })
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
    let access_token = get_access_token(spotify)?;
    let related = fetch_related_artists(&access_token, artist_id)?;

    // Seed artist + up to 7 related artists = up to 80 candidate tracks
    let mut artist_ids: Vec<String> = vec![artist_id.to_string()];
    artist_ids.extend(related.into_iter().take(7));

    let mut buckets: Vec<Vec<RadioTrack>> = Vec::new();
    for aid in &artist_ids {
        if let Ok(tracks) = fetch_artist_top_tracks(&access_token, aid) {
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
                if !seen.contains(&track.id) {
                    seen.insert(track.id.clone());
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
