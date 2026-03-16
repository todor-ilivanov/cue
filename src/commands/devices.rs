use anyhow::{bail, Result};
use rspotify::{prelude::OAuthClient, AuthCodeSpotify};

pub fn devices(spotify: &AuthCodeSpotify) -> Result<()> {
    let devices = spotify.device()?;

    if devices.is_empty() {
        println!("No devices available");
        return Ok(());
    }

    for device in &devices {
        let prefix = if device.is_active { "* " } else { "  " };
        println!("{prefix}{} ({:?})", device.name, device._type);
    }

    Ok(())
}

pub fn transfer(spotify: &AuthCodeSpotify, name: &str) -> Result<()> {
    let devices = spotify.device()?;
    let lower = name.to_lowercase();

    let device = devices
        .iter()
        .find(|d| d.name.to_lowercase().contains(&lower));

    let device = match device {
        Some(d) => d,
        None => {
            bail!("no device matching \"{name}\" — run \"cue devices\" to see available devices")
        }
    };

    let device_id = match &device.id {
        Some(id) => id,
        None => bail!("device \"{}\" has no ID", device.name),
    };

    spotify.transfer_playback(device_id, None)?;
    println!("Transferred playback to {}", device.name);

    Ok(())
}
