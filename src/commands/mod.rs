pub mod devices;
pub mod play;
pub mod player;
pub mod queue;
pub mod search;
pub mod volume;

use anyhow::{anyhow, Result};
use rspotify::model::CurrentPlaybackContext;
use rspotify::prelude::OAuthClient;
use rspotify::{AuthCodeSpotify, ClientError};

/// Fetch current playback, treating JSON parse failures as None.
/// rspotify can fail to deserialize when Spotify returns null for
/// fields like shuffle_state; a non-empty response that fails to
/// parse still implies a device is active.
pub fn current_playback(spotify: &AuthCodeSpotify) -> Result<Option<CurrentPlaybackContext>> {
    match spotify.current_playback(None, None::<&[_]>) {
        Ok(ctx) => Ok(ctx),
        Err(ClientError::ParseJson(_)) => Ok(None),
        Err(e) => Err(anyhow::Error::from(e).context("failed to get current playback")),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use rspotify::model::SimplifiedArtist;

    fn artist(name: &str) -> SimplifiedArtist {
        SimplifiedArtist {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn join_artist_names_variants() {
        assert_eq!(join_artist_names(&[artist("Radiohead")]), "Radiohead");
        assert_eq!(
            join_artist_names(&[artist("A"), artist("B"), artist("C")]),
            "A, B, C"
        );
        assert_eq!(join_artist_names(&[]), "");
    }

    #[test]
    fn release_year_extracts_four_chars() {
        assert_eq!(release_year(Some("2024-01-15")), Some("2024"));
        assert_eq!(release_year(Some("2024")), Some("2024"));
    }

    #[test]
    fn release_year_edge_cases() {
        assert_eq!(release_year(Some("abcd")), Some("abcd"));
        assert_eq!(release_year(Some("abc")), None);
        assert_eq!(release_year(Some("")), None);
        assert_eq!(release_year(None), None);
    }
}
