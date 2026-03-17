pub mod devices;
pub mod play;
pub mod player;
pub mod queue;
pub mod search;
pub mod volume;

use anyhow::anyhow;
use rspotify::ClientError;

pub fn join_artist_names(artists: &[rspotify::model::SimplifiedArtist]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn release_year(date: Option<&str>) -> Option<&str> {
    date.and_then(|d| d.get(..4))
}

/// Map common Spotify API errors to user-friendly messages.
pub fn api_error(err: ClientError, action: &str) -> anyhow::Error {
    if let ClientError::Http(ref e) = err {
        let msg = e.to_string();
        if msg.contains("status code 404") {
            return anyhow!(
                "no active device — use `cue devices` to list devices, then `cue device <name>` to select one"
            );
        }
        if msg.contains("status code 403") {
            return anyhow!("cannot {action} — Spotify may require Premium, or the device does not support this action");
        }
    }
    anyhow::Error::from(err).context(format!("failed to {action}"))
}
