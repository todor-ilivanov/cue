use anyhow::{anyhow, bail, Context, Result};
use rspotify::{
    prelude::{BaseClient, OAuthClient},
    scopes, AuthCodeSpotify, Credentials, OAuth,
};
use std::io::{Read, Write};
use std::net::TcpListener;

use crate::auth::{self, Config};

const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";

fn lock_token(
    spotify: &AuthCodeSpotify,
) -> Result<std::sync::MutexGuard<'_, Option<rspotify::Token>>> {
    spotify
        .token
        .lock()
        .map_err(|_| anyhow!("token lock failed"))
}

pub fn build_client(config: Config) -> Result<AuthCodeSpotify> {
    let creds = Credentials::new(&config.client_id, &config.client_secret);
    let oauth = OAuth {
        redirect_uri: REDIRECT_URI.to_owned(),
        scopes: scopes!(
            "user-read-playback-state",
            "user-modify-playback-state",
            "user-read-currently-playing"
        ),
        ..Default::default()
    };
    let spotify = AuthCodeSpotify::new(creds, oauth);

    if let Some(token) = auth::load_token()? {
        let expired = token.is_expired();
        *lock_token(&spotify)? = Some(token);
        if expired {
            match spotify.refresh_token() {
                Ok(()) => {
                    if let Some(t) = lock_token(&spotify)?.as_ref() {
                        auth::save_token(t)?;
                    }
                }
                Err(_) => {
                    auth::delete_token()?;
                    bail!("token refresh failed — re-run the command to re-authenticate");
                }
            }
        }
        return Ok(spotify);
    }

    // No saved token — run the full OAuth flow.
    let auth_url = spotify
        .get_authorize_url(false)
        .context("could not build authorization URL")?;
    eprintln!("Open this URL in your browser to authenticate:\n\n{auth_url}\n");

    let code = wait_for_callback(&spotify)?;
    spotify
        .request_token(&code)
        .context("failed to exchange authorization code for token")?;

    if let Some(t) = lock_token(&spotify)?.as_ref() {
        auth::save_token(t)?;
    }

    Ok(spotify)
}

pub fn persist_token(spotify: &AuthCodeSpotify) -> Result<()> {
    if let Some(t) = lock_token(spotify)?.as_ref() {
        auth::save_token(t)?;
    }
    Ok(())
}

fn wait_for_callback(spotify: &AuthCodeSpotify) -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:8888")
        .context("could not listen on 127.0.0.1:8888 for OAuth callback")?;
    eprintln!("Waiting for authentication in your browser...");

    let (mut stream, _) = listener
        .accept()
        .context("failed to accept OAuth callback")?;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .context("failed to read callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h1>Authenticated!</h1><p>You can close this tab.</p></body></html>";
    stream.write_all(response).ok();

    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("malformed HTTP request in OAuth callback")?;

    let url = format!("http://127.0.0.1:8888{path}");
    spotify
        .parse_response_code(&url)
        .context("could not parse authorization code from callback URL")
}
