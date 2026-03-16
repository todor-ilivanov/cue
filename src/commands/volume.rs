use anyhow::Result;
use rspotify::{prelude::OAuthClient, AuthCodeSpotify};

use crate::ui;

pub fn volume(spotify: &AuthCodeSpotify, level: u8) -> Result<()> {
    ui::with_spinner("Setting volume...", || {
        spotify.volume(level, None).map_err(anyhow::Error::from)
    })?;
    println!("Volume: {level}%");
    Ok(())
}
