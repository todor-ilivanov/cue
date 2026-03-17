use anyhow::{bail, Context, Result};
use rspotify::model::{PlayableItem, SearchResult};
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use super::{api_error, join_artist_names};
use crate::ui;

pub fn queue_add(spotify: &AuthCodeSpotify, query: &str, force_pick: bool) -> Result<()> {
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

fn playable_song_line(item: &PlayableItem) -> String {
    match item {
        PlayableItem::Track(track) => {
            ui::styled_song(&track.name, &join_artist_names(&track.artists))
        }
        PlayableItem::Episode(episode) => ui::styled_song(&episode.name, &episode.show.name),
    }
}

pub struct QueueContext {
    pub previous: Vec<String>,
    pub current: Option<String>,
    pub next: Vec<String>,
}

pub fn fetch_queue_context(
    spotify: &AuthCodeSpotify,
    prev_count: usize,
    next_count: usize,
) -> Result<QueueContext> {
    let queue = spotify
        .current_user_queue()
        .context("failed to get queue")?;

    let current = queue.currently_playing.as_ref().map(playable_song_line);

    let next: Vec<String> = queue
        .queue
        .iter()
        .take(next_count)
        .map(playable_song_line)
        .collect();

    let recent = spotify
        .current_user_recently_played(Some(prev_count as u32), None)
        .context("failed to get recently played")?;

    let mut previous: Vec<String> = recent
        .items
        .iter()
        .take(prev_count)
        .map(|h| ui::styled_song(&h.track.name, &join_artist_names(&h.track.artists)))
        .collect();
    previous.reverse();

    Ok(QueueContext {
        previous,
        current,
        next,
    })
}

pub fn queue_show(spotify: &AuthCodeSpotify) -> Result<()> {
    let ctx = ui::with_spinner("Fetching queue...", || fetch_queue_context(spotify, 2, 2))?;

    print_queue_context(&ctx);
    Ok(())
}

pub fn print_queue_context(ctx: &QueueContext) {
    let interactive = ui::is_interactive();

    for line in &ctx.previous {
        if interactive {
            eprintln!("  {}", console::style(line).dim());
        } else {
            println!("  {line}");
        }
    }

    match &ctx.current {
        Some(line) => println!("  {line}"),
        None => println!(
            "  {}",
            if interactive {
                console::style("Not playing").dim().to_string()
            } else {
                "Not playing".to_string()
            }
        ),
    }

    for line in &ctx.next {
        if interactive {
            eprintln!("  {}", console::style(line).dim());
        } else {
            println!("  {line}");
        }
    }
}
