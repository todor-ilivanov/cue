use anyhow::{bail, Context, Result};
use rspotify::model::{PlayableItem, SearchResult};
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use crate::ui;

fn format_duration_secs(total_secs: i64) -> String {
    let total_secs = total_secs.max(0);
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("{}:{:02}", minutes, seconds)
}

pub fn now(spotify: &AuthCodeSpotify) -> Result<()> {
    let context = ui::with_spinner("Fetching...", || {
        spotify
            .current_playing(None, None::<&[_]>)
            .context("failed to get currently playing track")
    })?;

    let (progress, item) = match context.and_then(|c| c.item.map(|i| (c.progress, i))) {
        Some(pair) => pair,
        None => {
            println!("Not playing");
            return Ok(());
        }
    };

    let progress_secs = progress.map(|d| d.num_seconds()).unwrap_or(0);

    let (artist, title, duration_secs) = match &item {
        PlayableItem::Track(track) => {
            let artists = track
                .artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            (artists, track.name.as_str(), track.duration.num_seconds())
        }
        PlayableItem::Episode(episode) => (
            episode.show.name.clone(),
            episode.name.as_str(),
            episode.duration.num_seconds(),
        ),
    };

    println!(
        "{} [{} / {}]",
        ui::styled_song(title, &artist),
        format_duration_secs(progress_secs),
        format_duration_secs(duration_secs)
    );

    Ok(())
}

pub fn search(spotify: &AuthCodeSpotify, query: &str, album: bool) -> Result<()> {
    if album {
        search_albums(spotify, query)
    } else {
        search_tracks(spotify, query)
    }
}

fn search_tracks(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
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
            .context("failed to search for tracks")
    })?;

    let tracks = match result {
        SearchResult::Tracks(page) => page,
        _ => bail!("unexpected search result type"),
    };

    if tracks.items.is_empty() {
        bail!("no results for \"{query}\"");
    }

    for (i, track) in tracks.items.iter().enumerate() {
        let artists = track
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {}. {}", i + 1, ui::styled_song(&track.name, &artists));
    }

    Ok(())
}

fn search_albums(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
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
            .context("failed to search for albums")
    })?;

    let albums = match result {
        SearchResult::Albums(page) => page,
        _ => bail!("unexpected search result type"),
    };

    if albums.items.is_empty() {
        bail!("no results for \"{query}\"");
    }

    for (i, album) in albums.items.iter().enumerate() {
        let artists = album
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  {}. {}", i + 1, ui::styled_song(&album.name, &artists));
    }

    Ok(())
}
