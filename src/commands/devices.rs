use anyhow::{bail, Context, Result};
use rspotify::model::{Device, DeviceType};
use rspotify::prelude::OAuthClient;
use rspotify::AuthCodeSpotify;

use crate::ui;

fn fetch_devices(spotify: &AuthCodeSpotify) -> Result<Vec<Device>> {
    ui::with_spinner("Fetching devices...", || {
        spotify.device().context("failed to fetch devices")
    })
}

fn fetch_devices_silent(spotify: &AuthCodeSpotify) -> Result<Vec<Device>> {
    spotify.device().context("failed to fetch devices")
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

/// Silently ensures a device is active before running a command.
/// Prefers a Computer matching the local hostname, then any single Computer,
/// then the first available device.
pub fn ensure_device(spotify: &AuthCodeSpotify) -> Result<()> {
    match spotify.current_playback(None, None::<&[_]>) {
        Ok(Some(pb)) if pb.device.id.is_some() => return Ok(()),
        Ok(_) => {}
        Err(rspotify::ClientError::ParseJson(_)) => return Ok(()),
        Err(e) => return Err(anyhow::Error::from(e).context("failed to check current playback")),
    }

    let devices = fetch_devices_silent(spotify)?;
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

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Prefers a Computer matching the local hostname, then any single Computer,
/// then the first device.
fn pick_best_device(devices: &[Device]) -> &Device {
    let computers: Vec<&Device> = devices
        .iter()
        .filter(|d| d._type == DeviceType::Computer)
        .collect();

    if let Some(hostname) = local_hostname() {
        let hostname_norm = normalize(&hostname);

        if let Some(device) = computers.iter().find(|d| {
            let name_norm = normalize(&d.name);
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
    spotify.transfer_playback(device_id, None)?;
    println!("Transferred playback to {}", device.name);
    Ok(())
}

fn show_active_device(spotify: &AuthCodeSpotify) -> Result<()> {
    if let Some(ctx) = super::current_playback(spotify)? {
        if ctx.device.id.is_some() {
            println!(
                "{} ({})",
                ctx.device.name,
                device_type_label(&ctx.device._type)
            );
            return Ok(());
        }
    }

    // No active device — auto-resolve one
    let devices = fetch_devices_silent(spotify)?;
    if devices.is_empty() {
        bail!("no devices found — open Spotify on a device first");
    }

    let device = pick_best_device(&devices);
    let device_id = require_device_id(device)?;
    spotify.transfer_playback(device_id, None)?;
    println!("{} ({})", device.name, device_type_label(&device._type));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, dtype: DeviceType) -> Device {
        Device {
            id: Some("id".to_string()),
            is_active: false,
            is_private_session: false,
            is_restricted: false,
            name: name.to_string(),
            _type: dtype,
            volume_percent: Some(50),
        }
    }

    #[test]
    fn device_type_labels() {
        assert_eq!(device_type_label(&DeviceType::Computer), "computer");
        assert_eq!(device_type_label(&DeviceType::Tv), "TV");
        assert_eq!(device_type_label(&DeviceType::Speaker), "speaker");
        assert_eq!(device_type_label(&DeviceType::GameConsole), "game console");
    }

    #[test]
    fn normalize_strips_and_lowercases() {
        assert_eq!(normalize("MacBook Pro"), "macbookpro");
        assert_eq!(normalize("my-comp.local"), "mycomplocal");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("ABC 123"), "abc123");
    }

    #[test]
    fn pick_best_device_single_computer() {
        let devices = vec![
            device("Speaker", DeviceType::Speaker),
            device("My Computer", DeviceType::Computer),
        ];
        let best = pick_best_device(&devices);
        assert_eq!(best.name, "My Computer");
    }

    #[test]
    fn pick_best_device_no_computers() {
        let devices = vec![
            device("Kitchen Speaker", DeviceType::Speaker),
            device("iPhone", DeviceType::Smartphone),
        ];
        let best = pick_best_device(&devices);
        assert_eq!(best.name, "Kitchen Speaker");
    }

    #[test]
    fn pick_best_device_multiple_computers() {
        let devices = vec![
            device("zzztesthost999alpha", DeviceType::Computer),
            device("zzztesthost999beta", DeviceType::Computer),
            device("Speaker", DeviceType::Speaker),
        ];
        let best = pick_best_device(&devices);
        // Either hostname matches one, or falls through to first device
        assert_eq!(best._type, DeviceType::Computer);
    }
}
