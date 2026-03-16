use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::io;
use std::path::PathBuf;

const CONFIG_FORMAT: &str =
    "  [spotify]\n  client_id = \"<your_client_id>\"\n  client_secret = \"<your_client_secret>\"";

#[derive(Deserialize)]
struct ConfigFile {
    spotify: SpotifyConfig,
}

#[derive(Deserialize)]
struct SpotifyConfig {
    client_id: String,
    client_secret: String,
}

#[allow(dead_code)]
pub struct Config {
    pub client_id: String,
    pub client_secret: String,
}

pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("could not determine config directory")?;
    Ok(base.join("cue"))
}

pub fn load_config() -> Result<Config> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("could not create config directory: {}", dir.display()))?;

    let path = dir.join("config.toml");

    let contents = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => bail!(
            "config file not found: {}\n\nCreate it with:\n\n{CONFIG_FORMAT}\n\nGet credentials at https://developer.spotify.com/dashboard",
            path.display()
        ),
        Err(e) => {
            return Err(e).with_context(|| format!("could not read config file: {}", path.display()))
        }
    };

    let file: ConfigFile = toml::from_str(&contents).with_context(|| {
        format!(
            "malformed config file: {}\n\nExpected format:\n\n{CONFIG_FORMAT}",
            path.display()
        )
    })?;

    if file.spotify.client_id.is_empty() || file.spotify.client_secret.is_empty() {
        bail!(
            "client_id and client_secret must not be empty in {}",
            path.display()
        );
    }

    Ok(Config {
        client_id: file.spotify.client_id,
        client_secret: file.spotify.client_secret,
    })
}
