use anyhow::{Context, Result};
use rspotify::{prelude::OAuthClient, AuthCodeSpotify};

use super::api_error;
use crate::ui;

pub fn volume(spotify: &AuthCodeSpotify, level: Option<&str>) -> Result<()> {
    match level {
        Some(s) => set_volume(spotify, s),
        None => show_volume(spotify),
    }
}

fn show_volume(spotify: &AuthCodeSpotify) -> Result<()> {
    let vol = get_volume(spotify)?;
    println!("Volume: {vol}%");
    Ok(())
}

fn set_volume(spotify: &AuthCodeSpotify, input: &str) -> Result<()> {
    let target = parse_level(spotify, input)?;

    ui::with_spinner("Setting volume...", || {
        spotify
            .volume(target, None)
            .map_err(|e| api_error(e, "set volume"))
    })?;
    println!("Volume: {target}%");
    Ok(())
}

fn get_volume(spotify: &AuthCodeSpotify) -> Result<u32> {
    let playback = ui::with_spinner("Fetching volume...", || {
        spotify
            .current_playback(None, None::<&[_]>)
            .context("failed to get current playback")
    })?;

    let vol = playback
        .and_then(|p| p.device.volume_percent)
        .context("no active device — use `cue device` to select one")?;

    Ok(vol)
}

fn parse_level(spotify: &AuthCodeSpotify, input: &str) -> Result<u8> {
    let input = input.trim();
    let needs_current = input.starts_with('+') || input.starts_with('-');
    let current = if needs_current {
        get_volume(spotify)?
    } else {
        0
    };
    ui::parse_volume(input, current)
}
