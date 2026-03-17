use anyhow::{bail, Context, Result};
use rspotify::{prelude::OAuthClient, AuthCodeSpotify};

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
        spotify.volume(target, None).map_err(anyhow::Error::from)
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

    if let Some(delta) = input.strip_prefix('+') {
        let delta: u32 = delta.parse().context("invalid volume adjustment")?;
        let current = get_volume(spotify)?;
        let target = (current + delta).min(100);
        return Ok(target as u8);
    }

    if let Some(delta) = input.strip_prefix('-') {
        let delta: u32 = delta.parse().context("invalid volume adjustment")?;
        let current = get_volume(spotify)?;
        let target = current.saturating_sub(delta).min(100);
        return Ok(target as u8);
    }

    let level: u32 = input.parse().context("invalid volume level")?;
    if level > 100 {
        bail!("volume must be 0-100, got {level}");
    }
    Ok(level as u8)
}
