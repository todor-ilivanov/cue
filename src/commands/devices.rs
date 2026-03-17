use anyhow::{bail, Context, Result};
use rspotify::model::{Device, DeviceType};
use rspotify::{prelude::OAuthClient, AuthCodeSpotify};

use crate::{auth, ui};

fn fetch_devices(spotify: &AuthCodeSpotify) -> Result<Vec<Device>> {
    ui::with_spinner("Fetching devices...", || {
        spotify.device().context("failed to fetch devices")
    })
}

fn do_transfer(spotify: &AuthCodeSpotify, device_id: &str, device_name: &str) -> Result<()> {
    spotify.transfer_playback(device_id, None)?;
    auth::save_last_device(device_id).ok();
    println!("Transferred playback to {device_name}");
    Ok(())
}

fn require_device_id(device: &Device) -> Result<&str> {
    match &device.id {
        Some(id) => Ok(id),
        None => bail!("device \"{}\" has no ID", device.name),
    }
}

fn device_type_label(dt: &DeviceType) -> &'static str {
    match dt {
        DeviceType::Computer => "computer",
        DeviceType::Tablet => "tablet",
        DeviceType::Smartphone => "smartphone",
        DeviceType::Speaker => "speaker",
        DeviceType::Tv => "TV",
        DeviceType::Avr => "AVR",
        DeviceType::Stb => "STB",
        DeviceType::AudioDongle => "audio dongle",
        DeviceType::GameConsole => "game console",
        DeviceType::CastVideo => "cast video",
        DeviceType::CastAudio => "cast audio",
        DeviceType::Automobile => "automobile",
        _ => "unknown",
    }
}

pub fn devices(spotify: &AuthCodeSpotify) -> Result<()> {
    let devices = fetch_devices(spotify)?;

    if devices.is_empty() {
        println!("No devices available");
        return Ok(());
    }

    for device in &devices {
        let name = if device.is_active {
            if ui::is_interactive() {
                format!("* {}", console::style(&device.name).bold())
            } else {
                format!("* {}", device.name)
            }
        } else {
            format!("  {}", device.name)
        };
        println!("{name} ({})", device_type_label(&device._type));
    }

    Ok(())
}

pub fn transfer(spotify: &AuthCodeSpotify, name: Option<&str>) -> Result<()> {
    match name {
        Some(n) => transfer_by_name(spotify, n),
        None => transfer_interactive(spotify),
    }
}

fn transfer_by_name(spotify: &AuthCodeSpotify, name: &str) -> Result<()> {
    let devices = fetch_devices(spotify)?;
    let lower = name.to_lowercase();

    let device = devices
        .iter()
        .find(|d| d.name.to_lowercase().contains(&lower));

    let device = match device {
        Some(d) => d,
        None => bail!("no device matching \"{name}\" — run `cue devices` to see available devices"),
    };

    let device_id = require_device_id(device)?;
    do_transfer(spotify, device_id, &device.name)
}

fn transfer_interactive(spotify: &AuthCodeSpotify) -> Result<()> {
    let devices = fetch_devices(spotify)?;

    if devices.is_empty() {
        bail!("no devices found — open Spotify on a device first");
    }

    if devices.len() == 1 {
        let device_id = require_device_id(&devices[0])?;
        return do_transfer(spotify, device_id, &devices[0].name);
    }

    // Check last device
    if let Ok(Some(last_id)) = auth::load_last_device() {
        if let Some(device) = devices.iter().find(|d| d.id.as_deref() == Some(&last_id)) {
            return do_transfer(spotify, &last_id, &device.name);
        }
    }

    // Interactive picker or fallback
    let labels: Vec<String> = devices
        .iter()
        .map(|d| {
            let active = if d.is_active { " (active)" } else { "" };
            format!("{}{active}", d.name)
        })
        .collect();

    let idx = match ui::select("Select a device", &labels)? {
        Some(i) => i,
        None => bail!("cancelled"),
    };

    let device_id = require_device_id(&devices[idx])?;
    do_transfer(spotify, device_id, &devices[idx].name)
}
