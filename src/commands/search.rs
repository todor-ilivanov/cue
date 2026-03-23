use anyhow::{bail, Context, Result};
use rspotify::model::{PlayableItem, SearchResult};
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use crate::commands::{join_artist_names, release_year};
use crate::ui;

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

    if ui::is_interactive() {
        println!("{}", ui::styled_song(title, &artist));
        if let Some(name) = album_name.filter(|n| !n.is_empty()) {
            println!("{}", console::style(name).dim());
        }
        println!("{}", ui::progress_bar(progress_secs, duration_secs));
    } else {
        let album_suffix = match album_name.filter(|n| !n.is_empty()) {
            Some(name) => format!(" — {name}"),
            None => String::new(),
        };
        println!(
            "{}{album_suffix} {}",
            ui::styled_song(title, &artist),
            ui::progress_bar(progress_secs, duration_secs)
        );
    }

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
                Some(10),
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

fn search_albums(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
    let result = ui::with_spinner("Searching...", || {
        spotify
            .search(
                query,
                rspotify::model::SearchType::Album,
                None,
                None,
                Some(10),
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

    let candidates: Vec<ui::PickCandidate> = albums
        .items
        .iter()
        .enumerate()
        .map(|(i, a)| ui::PickCandidate {
            name: a.name.clone(),
            label: format!("{} — {}", a.name, join_artist_names(&a.artists)),
            popularity: Some(super::positional_popularity(i)),
        })
        .collect();

    let ranked = ui::rank_candidates(query, &candidates, 5);

    for (display_idx, &(orig_idx, _)) in ranked.iter().enumerate() {
        let album = &albums.items[orig_idx];
        let artists = join_artist_names(&album.artists);

        let year_suffix = match release_year(album.release_date.as_deref()) {
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
