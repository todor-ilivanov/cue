use anyhow::{bail, Context, Result};
use rspotify::Token;
use serde::{Deserialize, Serialize};
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

fn load_json_list<T: serde::de::DeserializeOwned>(filename: &str) -> Vec<T> {
    let path = match config_dir() {
        Ok(d) => d.join(filename),
        Err(_) => return Vec::new(),
    };
    let contents = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

fn save_json_list<T: serde::Serialize>(filename: &str, entries: &[T]) -> Result<()> {
    let path = config_dir()?.join(filename);
    let json = serde_json::to_string(entries).context("could not serialize data")?;
    write_secure_file(&path, json.as_bytes())
}

const MAX_SEARCH_HISTORY: usize = 50;

#[derive(Serialize, Deserialize, Clone)]
pub struct SearchHistoryEntry {
    pub query: String,
    pub category: String,
}

pub fn load_search_history() -> Vec<SearchHistoryEntry> {
    load_json_list("search_history.json")
}

pub fn save_search_history(entries: &[SearchHistoryEntry]) -> Result<()> {
    save_json_list("search_history.json", entries)
}

pub fn add_search_history(history: &mut Vec<SearchHistoryEntry>, query: &str, category: &str) {
    history.retain(|e| !(e.query == query && e.category == category));
    history.insert(
        0,
        SearchHistoryEntry {
            query: query.to_string(),
            category: category.to_string(),
        },
    );
    history.truncate(MAX_SEARCH_HISTORY);
}

const MAX_RECENT_PLAYS: usize = 10;

#[derive(Serialize, Deserialize, Clone)]
pub struct RecentPlayEntry {
    pub title: String,
    pub subtitle: String,
    pub target_uri: String,
    pub target_type: String,
}

pub fn load_recent_plays() -> Vec<RecentPlayEntry> {
    load_json_list("recent_plays.json")
}

pub fn save_recent_plays(entries: &[RecentPlayEntry]) -> Result<()> {
    save_json_list("recent_plays.json", entries)
}

pub fn add_recent_play(recents: &mut Vec<RecentPlayEntry>, entry: RecentPlayEntry) {
    recents.retain(|e| e.target_uri != entry.target_uri);
    recents.insert(0, entry);
    recents.truncate(MAX_RECENT_PLAYS);
}
