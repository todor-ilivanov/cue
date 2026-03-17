use anyhow::{bail, Context, Result};
use rspotify::model::{PlayableItem, SearchResult};
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use crate::commands::{join_artist_names, release_year};
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

    let (artist, title, album_name, duration_secs) = match &item {
        PlayableItem::Track(track) => (
            join_artist_names(&track.artists),
            track.name.as_str(),
            Some(track.album.name.as_str()),
            track.duration.num_seconds(),
        ),
        PlayableItem::Episode(episode) => (
            episode.show.name.clone(),
            episode.name.as_str(),
            None,
            episode.duration.num_seconds(),
        ),
    };

    let album_suffix = match album_name.filter(|n| !n.is_empty()) {
        Some(name) if ui::is_interactive() => {
            console::style(format!(" ({name})")).dim().to_string()
        }
        Some(name) => format!(" ({name})"),
        None => String::new(),
    };

    println!(
        "{}{album_suffix} [{} / {}]",
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
        let artists = join_artist_names(&track.artists);

        let album_info = {
            let name = &track.album.name;
            let year = release_year(track.album.release_date.as_deref());
            match (name.is_empty(), year) {
                (true, _) => String::new(),
                (false, Some(y)) => format!(" ({name}, {y})"),
                (false, None) => format!(" ({name})"),
            }
        };

        if ui::is_interactive() {
            println!(
                "  {}. {}{}",
                i + 1,
                ui::styled_song(&track.name, &artists),
                console::style(&album_info).dim()
            );
        } else {
            println!("  {}. {} — {}{album_info}", i + 1, track.name, artists);
        }
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
        let artists = join_artist_names(&album.artists);

        let year_suffix = match release_year(album.release_date.as_deref()) {
            Some(y) => format!(" ({y})"),
            None => String::new(),
        };

        if ui::is_interactive() {
            println!(
                "  {}. {}{}",
                i + 1,
                ui::styled_song(&album.name, &artists),
                console::style(&year_suffix).dim()
            );
        } else {
            println!("  {}. {} — {}{year_suffix}", i + 1, album.name, artists);
        }
    }

    Ok(())
}
