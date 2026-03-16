use anyhow::{bail, Context, Result};
use rspotify::model::SearchResult;
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use crate::ui;

pub fn queue(spotify: &AuthCodeSpotify, query: &str) -> Result<()> {
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

    let valid: Vec<(usize, String)> = tracks
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            t.id.as_ref()?;
            let artists = t
                .artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Some((i, format!("{} — {artists}", t.name)))
        })
        .collect();

    let labels: Vec<String> = valid.iter().map(|(_, l)| l.clone()).collect();
    let pick = ui::pick_result(query, labels, "Select a track to queue")?;
    let idx = valid[pick].0;

    let track = &tracks.items[idx];
    let track_id = track.id.as_ref().context("track has no ID")?;
    let artists = track
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    ui::with_spinner("Adding to queue...", || {
        let playable = PlayableId::Track(track_id.clone());
        spotify
            .add_item_to_queue(playable, None)
            .map_err(anyhow::Error::from)
    })?;

    println!("Queued: {}", ui::styled_song(&track.name, &artists));
    Ok(())
}
