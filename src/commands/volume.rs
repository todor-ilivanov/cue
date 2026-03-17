use anyhow::{anyhow, bail, Context, Result};
use rspotify::{prelude::OAuthClient, AuthCodeSpotify, ClientError};

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
        spotify.volume(target, None).map_err(volume_error)
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

    if input.starts_with('+') || input.starts_with('-') {
        let delta: i32 = input.parse().context("invalid volume adjustment")?;
        let current = get_volume(spotify)? as i32;
        return Ok((current + delta).clamp(0, 100) as u8);
    }

    let level: u32 = input.parse().context("invalid volume level")?;
    if level > 100 {
        bail!("volume must be 0-100, got {level}");
    }
    Ok(level as u8)
}

fn volume_error(err: ClientError) -> anyhow::Error {
    if let ClientError::Http(ref e) = err {
        let msg = e.to_string();
        if msg.contains("status code 403") {
            return anyhow!(
                "cannot set volume — Spotify Premium is required, or this device does not support volume control"
            );
        }
        if msg.contains("status code 404") {
            return anyhow!(
                "no active device — use `cue devices` to list devices, then `cue device <name>` to select one"
            );
        }
    }
    anyhow::Error::from(err).context("failed to set volume")
}
