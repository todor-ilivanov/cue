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

fn playable_song_parts(item: &PlayableItem) -> (String, String) {
    match item {
        PlayableItem::Track(track) => (track.name.clone(), join_artist_names(&track.artists)),
        PlayableItem::Episode(episode) => (episode.name.clone(), episode.show.name.clone()),
    }
}

pub struct SongEntry {
    pub title: String,
    pub artist: String,
}

pub struct QueueContext {
    pub previous: Vec<SongEntry>,
    pub current: Option<SongEntry>,
    pub next: Vec<SongEntry>,
}

pub fn fetch_queue_context(
    spotify: &AuthCodeSpotify,
    prev_count: usize,
    next_count: usize,
) -> Result<QueueContext> {
    let queue = spotify
        .current_user_queue()
        .context("failed to get queue")?;

    let current = queue.currently_playing.as_ref().map(|item| {
        let (title, artist) = playable_song_parts(item);
        SongEntry { title, artist }
    });

    let next: Vec<SongEntry> = queue
        .queue
        .iter()
        .take(next_count)
        .map(|item| {
            let (title, artist) = playable_song_parts(item);
            SongEntry { title, artist }
        })
        .collect();

    let previous = if prev_count == 0 {
        Vec::new()
    } else {
        let recent = spotify
            .current_user_recently_played(Some(prev_count as u32), None)
            .context("failed to get recently played")?;

        let mut items: Vec<SongEntry> = recent
            .items
            .iter()
            .take(prev_count)
            .map(|h| SongEntry {
                title: h.track.name.clone(),
                artist: join_artist_names(&h.track.artists),
            })
            .collect();
        items.reverse();
        items
    };

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

fn print_queue_context(ctx: &QueueContext) {
    let interactive = ui::is_interactive();

    for entry in &ctx.previous {
        let line = ui::styled_song(&entry.title, &entry.artist);
        if interactive {
            println!("  {}", console::style(line).dim());
        } else {
            println!("  {line}");
        }
    }

    match &ctx.current {
        Some(entry) => println!("  {}", ui::styled_song(&entry.title, &entry.artist)),
        None => println!(
            "  {}",
            if interactive {
                console::style("Not playing").dim().to_string()
            } else {
                "Not playing".to_string()
            }
        ),
    }

    for entry in &ctx.next {
        let line = ui::styled_song(&entry.title, &entry.artist);
        if interactive {
            println!("  {}", console::style(line).dim());
        } else {
            println!("  {line}");
        }
    }
}
