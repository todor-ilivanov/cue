use anyhow::{bail, Context, Result};
use rspotify::model::SearchResult;
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use super::{api_error, join_artist_names};
use crate::ui;

pub fn queue(spotify: &AuthCodeSpotify, query: &str, force_pick: bool) -> Result<()> {
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

    let pick = ui::pick_result(query, candidates, "Select a track to queue", force_pick)?;
    let idx = indices[pick];

    let track = &tracks.items[idx];
    let track_id = track.id.as_ref().context("track has no ID")?;
    let artists = join_artist_names(&track.artists);

    ui::with_spinner("Adding to queue...", || {
        let playable = PlayableId::Track(track_id.clone());
        spotify
            .add_item_to_queue(playable, None)
            .map_err(|e| api_error(e, "add track to queue"))
    })?;

    println!("Queued: {}", ui::styled_song(&track.name, &artists));
    Ok(())
}
