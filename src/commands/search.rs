use anyhow::{Context, Result};
use rspotify::model::PlayableItem;
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

fn format_duration_secs(total_secs: i64) -> String {
    let total_secs = total_secs.max(0);
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("{}:{:02}", minutes, seconds)
}

pub fn now(spotify: &AuthCodeSpotify) -> Result<()> {
    let context = spotify
        .current_playing(None, None::<&[_]>)
        .context("failed to get currently playing track")?;

    let item = match context.and_then(|c| c.item.map(|i| (c.progress, i))) {
        Some(pair) => pair,
        None => {
            println!("Not playing");
            return Ok(());
        }
    };

    let (progress, item) = item;
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
        "{} — {} [{} / {}]",
        artist,
        title,
        format_duration_secs(progress_secs),
        format_duration_secs(duration_secs)
    );

    Ok(())
}
