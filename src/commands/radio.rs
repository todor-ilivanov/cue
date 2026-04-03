use anyhow::{bail, Context, Result};
use rspotify::model::{PlayableId, PlayableItem, TrackId};
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use super::{api_error, join_artist_names};
use crate::ui;

pub fn radio(spotify: &AuthCodeSpotify) -> Result<()> {
    let context = ui::with_spinner("Fetching current track...", || {
        spotify
            .current_playing(None, None::<&[_]>)
            .context("failed to get currently playing track")
    })?;

    let item = match context.and_then(|c| c.item) {
        Some(item) => item,
        None => bail!("no track is currently playing — play something first"),
    };

    let track = match item {
        PlayableItem::Track(track) => track,
        PlayableItem::Episode(_) => bail!("radio is only available for tracks, not episodes"),
    };

    let track_id = track.id.context("track has no ID")?;
    let track_id_str = track_id.id().to_string();
    let artist_id = track
        .artists
        .first()
        .and_then(|a| a.id.as_ref())
        .map(|id| id.id().to_string());
    let track_name = track.name.clone();
    let artist_name = join_artist_names(&track.artists);

    let recommendations = ui::with_spinner("Finding similar tracks...", || {
        crate::client::fetch_recommendations(spotify, &track_id_str, artist_id.as_deref(), 50)
    })?;

    if recommendations.is_empty() {
        bail!("Spotify returned no recommendations for this track");
    }

    let mut uris: Vec<PlayableId> = vec![PlayableId::Track(track_id)];
    for rec in &recommendations {
        if let Ok(id) = TrackId::from_id(&rec.id) {
            uris.push(PlayableId::Track(id));
        }
    }

    ui::with_spinner("Starting radio...", || {
        spotify
            .start_uris_playback(uris, None, None, None)
            .map_err(|e| api_error(e, "start radio playback"))
    })?;

    println!(
        "Radio based on: {}",
        ui::styled_song(&track_name, &artist_name)
    );
    if ui::is_interactive() {
        println!(
            "{}",
            console::style(format!("{} tracks queued", recommendations.len())).dim()
        );
    }

    Ok(())
}
