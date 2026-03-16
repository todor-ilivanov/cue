use anyhow::{anyhow, bail, Context, Result};
use rspotify::model::SearchResult;
use rspotify::prelude::*;
use rspotify::{AuthCodeSpotify, ClientError};

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

pub fn play(spotify: &AuthCodeSpotify, query: &str, album: bool, playlist: bool) -> Result<()> {
    if album {
        play_album(spotify, query)
    } else if playlist {
        play_playlist(spotify, query)
    } else {
        play_track(spotify, query)
    }
}

fn play_track(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
    let result = spotify
        .search(
            query,
            rspotify::model::SearchType::Track,
            None,
            None,
            Some(1),
            None,
        )
        .context("failed to search for track")?;

    let tracks = match result {
        SearchResult::Tracks(page) => page,
        _ => bail!("unexpected search result type"),
    };

    let track = match tracks.items.first() {
        Some(t) => t,
        None => bail!("no results for \"{query}\""),
    };

    let track_id = match &track.id {
        Some(id) => id,
        None => bail!("track has no ID"),
    };

    let artists = track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let playable = PlayableId::Track(track_id.clone());
    spotify
        .start_uris_playback([playable], None, None, None)
        .map_err(|e| playback_error(e, "start playback"))?;

    println!("Playing: {artists} — {}", track.name);
    Ok(())
}

fn play_album(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
    let result = spotify
        .search(
            query,
            rspotify::model::SearchType::Album,
            None,
            None,
            Some(1),
            None,
        )
        .context("failed to search for album")?;

    let albums = match result {
        SearchResult::Albums(page) => page,
        _ => bail!("unexpected search result type"),
    };

    let album = match albums.items.first() {
        Some(a) => a,
        None => bail!("no results for \"{query}\""),
    };

    let album_id = match &album.id {
        Some(id) => id,
        None => bail!("album has no ID"),
    };

    let artists = album
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let context_id = PlayContextId::Album(album_id.clone());
    spotify
        .start_context_playback(context_id, None, None, None)
        .map_err(|e| playback_error(e, "start album playback"))?;

    println!("Playing album: {} — {artists}", album.name);
    Ok(())
}

fn play_playlist(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
    let result = spotify
        .search(
            query,
            rspotify::model::SearchType::Playlist,
            None,
            None,
            Some(1),
            None,
        )
        .context("failed to search for playlist")?;

    let playlists = match result {
        SearchResult::Playlists(page) => page,
        _ => bail!("unexpected search result type"),
    };

    let playlist = match playlists.items.first() {
        Some(p) => p,
        None => bail!("no results for \"{query}\""),
    };

    let context_id = PlayContextId::Playlist(playlist.id.clone());
    spotify
        .start_context_playback(context_id, None, None, None)
        .map_err(|e| playback_error(e, "start playlist playback"))?;

    println!("Playing playlist: {}", playlist.name);
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
