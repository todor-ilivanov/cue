use anyhow::{bail, Context, Result};
use rspotify::Token;
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
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

pub struct Config {
    pub client_id: String,
    pub client_secret: String,
}

pub fn token_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("token.json"))
}

pub fn load_token() -> Result<Option<Token>> {
    let path = token_path()?;
    let contents = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("could not read token file: {}", path.display()))
        }
    };
    let token = serde_json::from_str(&contents)
        .with_context(|| format!("malformed token file: {}", path.display()))?;
    Ok(Some(token))
}

fn write_secure_file(path: &std::path::Path, data: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?
        .write_all(data)
        .with_context(|| format!("could not write to {}", path.display()))
}

pub fn save_token(token: &Token) -> Result<()> {
    let path = token_path()?;
    let json = serde_json::to_string(token).context("could not serialize token")?;
    write_secure_file(&path, json.as_bytes())
}

pub fn delete_token() -> Result<()> {
    let path = token_path()?;
    fs::remove_file(&path)
        .or_else(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(e)
            }
        })
        .with_context(|| format!("could not delete token file: {}", path.display()))
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
