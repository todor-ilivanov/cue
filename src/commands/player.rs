use anyhow::{Context, Result};
use console::{Key, Term};
use rspotify::model::PlayableItem;
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use super::{api_error, join_artist_names};
use crate::ui;

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

struct TrackInfo {
    title: String,
    artist: String,
    album: String,
    duration_secs: i64,
    progress_secs: i64,
    is_playing: bool,
}

fn fetch_now_playing(spotify: &AuthCodeSpotify) -> Result<Option<TrackInfo>> {
    let context = spotify
        .current_playing(None, None::<&[_]>)
        .context("failed to get currently playing track")?;

    let Some(ctx) = context else {
        return Ok(None);
    };

    let is_playing = ctx.is_playing;
    let progress_secs = ctx.progress.map(|d| d.num_seconds()).unwrap_or(0);

    let Some(item) = ctx.item else {
        return Ok(None);
    };

    let (artist, title, album, duration_secs) = match &item {
        PlayableItem::Track(track) => (
            join_artist_names(&track.artists),
            track.name.clone(),
            track.album.name.clone(),
            track.duration.num_seconds(),
        ),
        PlayableItem::Episode(episode) => (
            episode.show.name.clone(),
            episode.name.clone(),
            String::new(),
            episode.duration.num_seconds(),
        ),
    };

    Ok(Some(TrackInfo {
        title,
        artist,
        album,
        duration_secs,
        progress_secs,
        is_playing,
    }))
}

fn draw(term: &Term, info: &TrackInfo, progress: i64) {
    let _ = term.clear_last_lines(4);

    let song_line = ui::styled_song(&info.title, &info.artist);
    let _ = term.write_line(&song_line);

    if !info.album.is_empty() {
        let _ = term.write_line(&format!("{}", console::style(&info.album).dim()));
    } else {
        let _ = term.write_line("");
    }

    let bar = ui::progress_bar(progress, info.duration_secs);
    let status = if info.is_playing { ">" } else { "||" };
    let _ = term.write_line(&format!("{status}  {bar}"));

    let hints = format!(
        "{}  {}  {}  {}  {}",
        console::style("space").bold(),
        console::style("pause/resume").dim(),
        console::style("n/p").bold(),
        console::style("next/prev").dim(),
        console::style("q quit").dim(),
    );
    let _ = term.write_line(&hints);
}

fn draw_empty(term: &Term) {
    let _ = term.clear_last_lines(4);
    let _ = term.write_line(&format!("{}", console::style("Not playing").dim()));
    let _ = term.write_line("");
    let _ = term.write_line("");
    let hints = format!(
        "{}  {}  {}",
        console::style("r").bold(),
        console::style("refresh").dim(),
        console::style("q quit").dim(),
    );
    let _ = term.write_line(&hints);
}

pub fn player(spotify: &AuthCodeSpotify) -> Result<()> {
    if !ui::is_interactive() {
        anyhow::bail!("player requires an interactive terminal");
    }

    let term = Term::stderr();
    let _ = term.hide_cursor();

    // Print initial blank lines so clear_last_lines has something to clear
    for _ in 0..4 {
        let _ = term.write_line("");
    }

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let key_term = Term::stderr();
        while let Ok(key) = key_term.read_key() {
            if tx.send(key).is_err() {
                break;
            }
        }
    });

    let poll_interval = Duration::from_secs(5);
    let mut last_fetch = Instant::now() - poll_interval;
    let mut fetch_anchor = Instant::now();
    let mut info: Option<TrackInfo> = None;

    loop {
        // Process all pending keys
        while let Ok(key) = rx.try_recv() {
            match key {
                Key::Char('q') | Key::Escape => {
                    let _ = term.show_cursor();
                    return Ok(());
                }
                Key::Char(' ') => {
                    if let Some(ref mut t) = info {
                        if t.is_playing {
                            let _ = spotify
                                .pause_playback(None)
                                .map_err(|e| api_error(e, "pause playback"));
                            t.is_playing = false;
                        } else {
                            let _ = spotify
                                .resume_playback(None, None)
                                .map_err(|e| api_error(e, "resume playback"));
                            t.is_playing = true;
                        }
                        fetch_anchor = Instant::now();
                    }
                }
                Key::Char('n') => {
                    let _ = spotify
                        .next_track(None)
                        .map_err(|e| api_error(e, "skip to next track"));
                    // Force refetch after a brief delay for Spotify to update
                    thread::sleep(Duration::from_millis(400));
                    last_fetch = Instant::now() - poll_interval;
                }
                Key::Char('p') => {
                    let _ = spotify
                        .previous_track(None)
                        .map_err(|e| api_error(e, "go to previous track"));
                    thread::sleep(Duration::from_millis(400));
                    last_fetch = Instant::now() - poll_interval;
                }
                Key::Char('r') => {
                    last_fetch = Instant::now() - poll_interval;
                }
                _ => {}
            }
        }

        // Poll API
        if last_fetch.elapsed() >= poll_interval {
            match fetch_now_playing(spotify) {
                Ok(new_info) => {
                    fetch_anchor = Instant::now();
                    info = new_info;
                }
                Err(_) => {
                    // Silently skip failed fetches, keep showing last state
                }
            }
            last_fetch = Instant::now();
        }

        // Draw
        match &info {
            Some(track) => {
                let progress = current_progress(&info, fetch_anchor);
                draw(&term, track, progress);
            }
            None => {
                draw_empty(&term);
            }
        }

        thread::sleep(Duration::from_millis(200));
    }
}

fn current_progress(info: &Option<TrackInfo>, fetch_anchor: Instant) -> i64 {
    match info {
        Some(t) => {
            if t.is_playing {
                let elapsed = fetch_anchor.elapsed().as_secs() as i64;
                (t.progress_secs + elapsed).min(t.duration_secs)
            } else {
                t.progress_secs
            }
        }
        None => 0,
    }
}
