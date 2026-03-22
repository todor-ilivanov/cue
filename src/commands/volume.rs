use anyhow::{bail, Context, Result};
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
    let playback = ui::with_spinner("Fetching volume...", || super::current_playback(spotify))?;

    playback
        .and_then(|p| p.device.volume_percent)
        .context("no active device — open Spotify on a device first")
}

fn parse_level(spotify: &AuthCodeSpotify, input: &str) -> Result<u8> {
    let input = input.trim();
    let current = if input.starts_with('+') || input.starts_with('-') {
        get_volume(spotify)?
    } else {
        0
    };
    parse_volume(input, current)
}

fn parse_volume(input: &str, current: u32) -> Result<u8> {
    let input = input.trim();

    if input.starts_with('+') || input.starts_with('-') {
        let delta: i32 = input
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid volume adjustment: {input}"))?;
        return Ok((current as i32 + delta).clamp(0, 100) as u8);
    }

    let level: u32 = input
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid volume level: {input}"))?;
    if level > 100 {
        bail!("volume must be 0-100, got {level}");
    }
    Ok(level as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute() {
        assert_eq!(parse_volume("50", 0).unwrap(), 50);
        assert_eq!(parse_volume("0", 80).unwrap(), 0);
        assert_eq!(parse_volume("100", 0).unwrap(), 100);
    }

    #[test]
    fn rejects_over_100() {
        assert!(parse_volume("101", 0).is_err());
        assert!(parse_volume("200", 0).is_err());
    }

    #[test]
    fn relative() {
        assert_eq!(parse_volume("+10", 50).unwrap(), 60);
        assert_eq!(parse_volume("-10", 50).unwrap(), 40);
    }

    #[test]
    fn clamps() {
        assert_eq!(parse_volume("+20", 90).unwrap(), 100);
        assert_eq!(parse_volume("-20", 10).unwrap(), 0);
    }

    #[test]
    fn invalid() {
        assert!(parse_volume("abc", 0).is_err());
        assert!(parse_volume("+abc", 0).is_err());
    }
}
