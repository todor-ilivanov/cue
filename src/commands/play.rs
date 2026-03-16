use anyhow::{anyhow, bail, Context, Result};
use rspotify::model::SearchResult;
use rspotify::prelude::*;
use rspotify::{AuthCodeSpotify, ClientError};

use crate::ui;

fn playback_error(err: ClientError, action: &str) -> anyhow::Error {
    if let ClientError::Http(ref e) = err {
        let msg = e.to_string();
        if msg.contains("status code 404") {
            return anyhow!(
                "no active device — use `cue devices` to list devices, then `cue device <name>` to select one"
            );
        }
        if msg.contains("status code 403") {
            return anyhow!(
                "cannot {action} — this can happen when playing a single track with no context (try playing an album or playlist instead)"
            );
        }
    }
    anyhow::Error::from(err).context(format!("failed to {action}"))
}

pub fn play(
    spotify: &AuthCodeSpotify,
    query: &str,
    album: bool,
    playlist: bool,
    force_pick: bool,
) -> Result<()> {
    if album {
        play_album(spotify, query, force_pick)
    } else if playlist {
        play_playlist(spotify, query, force_pick)
    } else {
        play_track(spotify, query, force_pick)
    }
}

use super::join_artist_names;

fn play_track(spotify: &AuthCodeSpotify, query: &str, force_pick: bool) -> Result<()> {
    let result = ui::with_spinner("Searching...", || {
        spotify
            .search(
                query,
                rspotify::model::SearchType::Track,
                None,
                None,
                Some(5),
                None,
            )
            .context("failed to search for track")
    })?;

    let tracks = match result {
        SearchResult::Tracks(page) => page,
        _ => bail!("unexpected search result type"),
    };

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
    let track_id = track.id.as_ref().context("track has no ID")?;
    let artists = join_artist_names(&track.artists);

    ui::with_spinner("Starting playback...", || {
        let playable = PlayableId::Track(track_id.clone());
        spotify
            .start_uris_playback([playable], None, None, None)
            .map_err(|e| playback_error(e, "start playback"))
    })?;

    println!("Playing: {}", ui::styled_song(&track.name, &artists));
    Ok(())
}

fn play_album(spotify: &AuthCodeSpotify, query: &str, force_pick: bool) -> Result<()> {
    let result = ui::with_spinner("Searching...", || {
        spotify
            .search(
                query,
                rspotify::model::SearchType::Album,
                None,
                None,
                Some(5),
                None,
            )
            .context("failed to search for album")
    })?;

    let albums = match result {
        SearchResult::Albums(page) => page,
        _ => bail!("unexpected search result type"),
    };

    let (indices, candidates): (Vec<usize>, Vec<ui::PickCandidate>) = albums
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            a.id.as_ref()?;
            Some((
                i,
                ui::PickCandidate {
                    name: a.name.clone(),
                    label: format!("{} — {}", a.name, join_artist_names(&a.artists)),
                    popularity: None,
                },
            ))
        })
        .unzip();

    let pick = ui::pick_result(query, candidates, "Select an album", force_pick)?;
    let idx = indices[pick];

    let album = &albums.items[idx];
    let album_id = album.id.as_ref().context("album has no ID")?;
    let artists = join_artist_names(&album.artists);

    ui::with_spinner("Starting playback...", || {
        let context_id = PlayContextId::Album(album_id.clone());
        spotify
            .start_context_playback(context_id, None, None, None)
            .map_err(|e| playback_error(e, "start album playback"))
    })?;

    println!("Playing album: {}", ui::styled_song(&album.name, &artists));
    Ok(())
}

fn play_playlist(spotify: &AuthCodeSpotify, query: &str, force_pick: bool) -> Result<()> {
    let result = ui::with_spinner("Searching...", || {
        spotify
            .search(
                query,
                rspotify::model::SearchType::Playlist,
                None,
                None,
                Some(5),
                None,
            )
            .context("failed to search for playlist")
    })?;

    let playlists = match result {
        SearchResult::Playlists(page) => page,
        _ => bail!("unexpected search result type"),
    };

    let candidates: Vec<ui::PickCandidate> = playlists
        .items
        .iter()
        .map(|p| {
            let owner = p.owner.display_name.as_deref().unwrap_or("unknown");
            ui::PickCandidate {
                name: p.name.clone(),
                label: format!("{} — by {owner}", p.name),
                popularity: None,
            }
        })
        .collect();

    let idx = ui::pick_result(query, candidates, "Select a playlist", force_pick)?;

    let playlist = &playlists.items[idx];

    ui::with_spinner("Starting playback...", || {
        let context_id = PlayContextId::Playlist(playlist.id.clone());
        spotify
            .start_context_playback(context_id, None, None, None)
            .map_err(|e| playback_error(e, "start playlist playback"))
    })?;

    let owner = playlist.owner.display_name.as_deref().unwrap_or("unknown");
    println!(
        "Playing playlist: {}",
        ui::styled_song(&playlist.name, &format!("by {owner}"))
    );
    Ok(())
}

pub fn pause(spotify: &AuthCodeSpotify) -> Result<()> {
    spotify
        .pause_playback(None)
        .map_err(|e| playback_error(e, "pause playback"))?;
    println!("Paused");
    Ok(())
}

pub fn resume(spotify: &AuthCodeSpotify) -> Result<()> {
    spotify
        .resume_playback(None, None)
        .map_err(|e| playback_error(e, "resume playback"))?;
    println!("Resumed");
    Ok(())
}

pub fn next(spotify: &AuthCodeSpotify) -> Result<()> {
    spotify
        .next_track(None)
        .map_err(|e| playback_error(e, "skip to next track"))?;
    println!("Skipped to next track");
    Ok(())
}

pub fn prev(spotify: &AuthCodeSpotify) -> Result<()> {
    spotify
        .previous_track(None)
        .map_err(|e| playback_error(e, "go to previous track"))?;
    println!("Back to previous track");
    Ok(())
}
