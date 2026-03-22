use anyhow::{bail, Context, Result};
use rspotify::model::{Device, DeviceType};
use rspotify::{prelude::OAuthClient, AuthCodeSpotify};

use crate::ui;

fn fetch_devices(spotify: &AuthCodeSpotify) -> Result<Vec<Device>> {
    ui::with_spinner("Fetching devices...", || {
        spotify.device().context("failed to fetch devices")
    })
}

fn do_transfer(spotify: &AuthCodeSpotify, device_id: &str, device_name: &str) -> Result<()> {
    spotify.transfer_playback(device_id, None)?;
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

/// Ensure a device is active before running a command.
/// If one is already active, this is a no-op. Otherwise, it picks the best
/// available device (preferring a Computer matching the local hostname) and
/// silently transfers playback to it.
pub fn ensure_device(spotify: &AuthCodeSpotify) -> Result<()> {
    let playback = spotify
        .current_playback(None, None::<&[_]>)
        .context("failed to check current playback")?;

    if playback.and_then(|p| p.device.id).is_some() {
        return Ok(());
    }

    let devices = fetch_devices(spotify)?;
    if devices.is_empty() {
        bail!("no devices found — open Spotify on a device first");
    }

    let device = pick_best_device(&devices);
    let device_id = require_device_id(device)?;
    spotify.transfer_playback(device_id, None)?;
    Ok(())
}

fn local_hostname() -> Option<String> {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Pick the best device from a non-empty list.
/// Prefers a Computer whose name matches the local hostname, then any Computer,
/// then the first device.
fn pick_best_device(devices: &[Device]) -> &Device {
    let computers: Vec<&Device> = devices
        .iter()
        .filter(|d| d._type == DeviceType::Computer)
        .collect();

    if let Some(hostname) = local_hostname() {
        let hostname_lower = hostname.to_lowercase();
        // Normalize hostname: "macbook-pro" -> "macbookpro", "macbook pro" -> "macbookpro"
        let hostname_norm: String = hostname_lower
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect();

        if let Some(device) = computers.iter().find(|d| {
            let name_norm: String = d
                .name
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            name_norm.contains(&hostname_norm) || hostname_norm.contains(&name_norm)
        }) {
            return device;
        }
    }

    if computers.len() == 1 {
        return computers[0];
    }

    &devices[0]
}

pub fn transfer(spotify: &AuthCodeSpotify, name: Option<&str>) -> Result<()> {
    match name {
        Some(n) => transfer_by_name(spotify, n),
        None => show_active_device(spotify),
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

fn show_active_device(spotify: &AuthCodeSpotify) -> Result<()> {
    ensure_device(spotify)?;

    let playback = spotify
        .current_playback(None, None::<&[_]>)
        .context("failed to check current playback")?;

    match playback.map(|p| (p.device.name.clone(), p.device._type)) {
        Some((name, dtype)) => {
            println!("{name} ({})", device_type_label(&dtype));
        }
        None => bail!("no active device"),
    }
    Ok(())
}
