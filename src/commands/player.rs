use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use rspotify::model::PlayableItem;
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use super::join_artist_names;
use crate::lyrics::{self, LyricsState};
use crate::ui;

use std::sync::mpsc;
use std::time::{Duration, Instant};

// Amber/gold accent palette
const ACCENT: Color = Color::Rgb(255, 191, 0);
const SEPARATOR_COLOR: Color = Color::Rgb(60, 60, 60);

struct TrackInfo {
    title: String,
    artist: String,
    album: String,
    duration_secs: i64,
    progress_ms: i64,
    is_playing: bool,
    volume_percent: Option<u32>,
}

fn fetch_now_playing(spotify: &AuthCodeSpotify) -> Result<Option<TrackInfo>> {
    let Some(ctx) = super::current_playback(spotify)? else {
        return Ok(None);
    };

    let is_playing = ctx.is_playing;
    let progress_ms = ctx.progress.map(|d| d.num_milliseconds()).unwrap_or(0);
    let volume_percent = ctx.device.volume_percent;

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
        progress_ms,
        is_playing,
        volume_percent,
    }))
}

fn current_progress_ms(track: &TrackInfo, fetch_anchor: Instant) -> i64 {
    if track.is_playing {
        let elapsed = fetch_anchor.elapsed().as_millis() as i64;
        (track.progress_ms + elapsed).min(track.duration_secs * 1000)
    } else {
        track.progress_ms
    }
}

fn build_separator(width: u16) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width as usize),
        Style::new().fg(SEPARATOR_COLOR),
    ))
}

fn draw_playing(
    frame: &mut Frame,
    info: &TrackInfo,
    progress_ms: i64,
    lyrics_state: &LyricsState,
    show_lyrics: bool,
    lyrics_scroll_center: Option<usize>,
) {
    let area = frame.area();
    let max_width = 80u16;
    let content_area = if area.width > max_width {
        let margin = (area.width - max_width) / 2;
        Rect::new(area.x + margin, area.y, max_width, area.height)
    } else {
        area
    };
    let height = content_area.height;
    let width = content_area.width;

    let compact = height < 10;
    let show_album = !compact;
    let show_top_margin = height > 12;

    let mut constraints: Vec<Constraint> = Vec::new();

    // Top margin for breathing room
    if show_top_margin {
        constraints.push(Constraint::Length(1));
    }

    // Separator line above now-playing card
    let sep_above_row = constraints.len();
    constraints.push(Constraint::Length(1));

    let track_row = constraints.len();
    constraints.push(Constraint::Length(1)); // current track

    let album_row = if show_album {
        let r = constraints.len();
        constraints.push(Constraint::Length(1)); // album
        Some(r)
    } else {
        None
    };

    let progress_row = constraints.len();
    constraints.push(Constraint::Length(1)); // progress bar

    // Separator line below now-playing card
    let sep_below_row = constraints.len();
    constraints.push(Constraint::Length(1));

    let lyrics_row = constraints.len();
    constraints.push(Constraint::Min(0)); // lyrics or spacer
    constraints.push(Constraint::Length(1)); // blank line above hints
    let hints_row = constraints.len();
    constraints.push(Constraint::Length(1)); // hints

    let rows = Layout::vertical(constraints).split(content_area);

    // Separator above now-playing
    frame.render_widget(Paragraph::new(build_separator(width)), rows[sep_above_row]);

    // Current track — amber title, subtle artist
    let track_line = Line::from(vec![
        Span::styled(
            &info.title,
            Style::new().add_modifier(Modifier::BOLD).fg(ACCENT),
        ),
        Span::styled(" \u{2014} ", Style::new().fg(Color::DarkGray)),
        Span::styled(&info.artist, Style::new().fg(Color::Gray)),
    ]);
    frame.render_widget(Paragraph::new(track_line), rows[track_row]);

    // Album
    if let Some(ar) = album_row {
        let album_line = Line::from(Span::styled(&info.album, Style::new().fg(Color::DarkGray)));
        frame.render_widget(Paragraph::new(album_line), rows[ar]);
    }

    // Progress bar
    let progress_line = build_progress_line(
        progress_ms,
        info.duration_secs,
        info.is_playing,
        info.volume_percent,
        rows[progress_row],
    );
    frame.render_widget(Paragraph::new(progress_line), rows[progress_row]);

    // Separator below now-playing
    frame.render_widget(Paragraph::new(build_separator(width)), rows[sep_below_row]);

    // Lyrics
    if show_lyrics {
        lyrics::draw_lyrics(
            frame,
            rows[lyrics_row],
            lyrics_state,
            progress_ms as u64,
            lyrics_scroll_center,
        );
    }

    // Hints
    let hints = build_hints_playing(width);
    frame.render_widget(Paragraph::new(hints), rows[hints_row]);
}

fn draw_empty(frame: &mut Frame) {
    let area = frame.area();

    let rows = Layout::vertical([
        Constraint::Length(1), // blank
        Constraint::Length(1), // "Not playing"
        Constraint::Min(0),    // spacer
        Constraint::Length(1), // hints
    ])
    .split(area);

    let msg = Line::from(Span::styled(
        "Not playing",
        Style::new().fg(Color::DarkGray),
    ));
    frame.render_widget(Paragraph::new(msg), rows[1]);

    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);
    let hints = Line::from(vec![
        Span::styled("r", key_style),
        Span::styled(" refresh  ", desc_style),
        Span::styled("q", key_style),
        Span::styled(" quit", desc_style),
    ]);
    frame.render_widget(Paragraph::new(hints), rows[3]);
}

fn progress_bar_width(content_width: u16, progress_secs: i64, duration_secs: i64, volume: Option<u32>) -> usize {
    let left_len = ui::format_duration(progress_secs).len();
    let right_len = ui::format_duration(duration_secs).len();
    let vol_len = volume.map(|v| format!("  vol {v}%").len()).unwrap_or(0);
    let label_width = 2 + left_len + 1 + 1 + right_len + vol_len;
    (content_width as usize).saturating_sub(label_width)
}

fn build_progress_line<'a>(
    progress_ms: i64,
    duration_secs: i64,
    is_playing: bool,
    volume: Option<u32>,
    area: Rect,
) -> Line<'a> {
    let progress_secs = progress_ms / 1000;
    let status = if is_playing { "\u{25b6}" } else { "\u{23f8}" };
    let left = ui::format_duration(progress_secs);
    let right = ui::format_duration(duration_secs);
    let vol_label = volume.map(|v| format!("  vol {v}%")).unwrap_or_default();

    let bar_width = progress_bar_width(area.width, progress_secs, duration_secs, volume);

    let ratio = if duration_secs > 0 {
        (progress_ms as f64 / (duration_secs as f64 * 1000.0)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let bar_inner = bar_width.saturating_sub(1); // reserve 1 for dot
    let filled = (bar_inner as f64 * ratio).floor() as usize;
    let remaining = bar_inner.saturating_sub(filled);

    Line::from(vec![
        Span::styled(format!("{status} "), Style::new().fg(ACCENT)),
        Span::styled(left, Style::new().fg(Color::White)),
        Span::raw(" "),
        Span::styled("\u{2501}".repeat(filled), Style::new().fg(ACCENT)),
        Span::styled("\u{25cf}", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("\u{2500}".repeat(remaining), Style::new().fg(SEPARATOR_COLOR)),
        Span::raw(" "),
        Span::styled(right, Style::new().fg(Color::DarkGray)),
        Span::styled(vol_label, Style::new().fg(Color::DarkGray)),
    ])
}

fn build_hints_playing(width: u16) -> Line<'static> {
    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);

    if width < 85 {
        // Compact hints for narrow terminals
        Line::from(vec![
            Span::styled("spc", key_style),
            Span::styled(" \u{23ef}  ", desc_style),
            Span::styled("n/p", key_style),
            Span::styled(" \u{23ed}/\u{23ee}  ", desc_style),
            Span::styled("\u{2190}/\u{2192}", key_style),
            Span::styled(" seek  ", desc_style),
            Span::styled("\u{2191}/\u{2193}", key_style),
            Span::styled(" vol  ", desc_style),
            Span::styled("j/k", key_style),
            Span::styled(" scroll  ", desc_style),
            Span::styled("s", key_style),
            Span::styled(" sync  ", desc_style),
            Span::styled("q", key_style),
            Span::styled(" quit", desc_style),
        ])
    } else {
        Line::from(vec![
            Span::styled("space", key_style),
            Span::styled(" pause/resume  ", desc_style),
            Span::styled("n/p", key_style),
            Span::styled(" next/prev  ", desc_style),
            Span::styled("</>", key_style),
            Span::styled(" seek  ", desc_style),
            Span::styled("\u{2191}/\u{2193}", key_style),
            Span::styled(" volume  ", desc_style),
            Span::styled("j/k", key_style),
            Span::styled(" scroll lyrics  ", desc_style),
            Span::styled("s", key_style),
            Span::styled(" sync  ", desc_style),
            Span::styled("q", key_style),
            Span::styled(" quit", desc_style),
        ])
    }
}

pub fn player(spotify: &AuthCodeSpotify, slim: bool) -> Result<()> {
    if !ui::is_interactive() {
        anyhow::bail!("player requires an interactive terminal");
    }

    crossterm::terminal::enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::cursor::Hide
    )
    .context("failed to enter alternate screen")?;

    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).context("failed to create terminal")?;

    let result = run_player_loop(&mut terminal, spotify, slim);

    // Restore terminal (always, even on error)
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );

    result
}

fn run_player_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    spotify: &AuthCodeSpotify,
    slim: bool,
) -> Result<()> {
    let poll_interval = Duration::from_secs(5);
    let mut last_fetch = Instant::now() - poll_interval;
    let mut fetch_anchor = Instant::now();
    let mut deferred_fetch: Option<Instant> = None;
    let mut info: Option<TrackInfo> = None;
    let mut last_drawn: Option<(i64, i64, bool, Option<u32>)> = None;

    // Lyrics state
    let mut lyrics_state = LyricsState::None;
    let mut lyrics_rx: Option<mpsc::Receiver<LyricsState>> = None;
    let mut lyrics_track: Option<(String, String, String)> = None;
    let mut show_lyrics = !slim;
    let mut last_lyric_index: Option<Option<usize>> = None;
    let mut lyrics_scroll_center: Option<usize> = None;

    loop {
        let mut needs_redraw = false;

        // Process all pending key events
        while crossterm::event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(' ') => {
                        if let Some(ref mut t) = info {
                            if t.is_playing {
                                let _ = spotify.pause_playback(None);
                                t.is_playing = false;
                            } else {
                                let _ = spotify.resume_playback(None, None);
                                t.is_playing = true;
                            }
                            fetch_anchor = Instant::now();
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('p') => {
                        let _ = if key.code == KeyCode::Char('n') {
                            spotify.next_track(None)
                        } else {
                            spotify.previous_track(None)
                        };
                        deferred_fetch = Some(Instant::now() + Duration::from_millis(400));
                        needs_redraw = true;
                    }
                    KeyCode::Left | KeyCode::Right => {
                        if let Some(ref mut t) = info {
                            let delta_ms = if key.code == KeyCode::Right {
                                5000
                            } else {
                                -5000
                            };
                            let current_ms = current_progress_ms(t, fetch_anchor);
                            let new_ms = (current_ms + delta_ms).clamp(0, t.duration_secs * 1000);
                            let pos = chrono::Duration::milliseconds(new_ms);
                            if spotify.seek_track(pos, None).is_ok() {
                                t.progress_ms = new_ms;
                                fetch_anchor = Instant::now();
                                needs_redraw = true;
                            }
                        }
                    }
                    KeyCode::Up | KeyCode::Down => {
                        if let Some(ref mut t) = info {
                            if let Some(current_vol) = t.volume_percent {
                                let delta: i32 = if key.code == KeyCode::Up { 5 } else { -5 };
                                let new_vol = (current_vol as i32 + delta).clamp(0, 100) as u8;
                                if spotify.volume(new_vol, None).is_ok() {
                                    t.volume_percent = Some(new_vol as u32);
                                    needs_redraw = true;
                                }
                            }
                        }
                    }
                    KeyCode::Char('l') => {
                        show_lyrics = !show_lyrics;
                        needs_redraw = true;
                    }
                    KeyCode::Char('j') | KeyCode::Char('k') => {
                        if show_lyrics {
                            if let LyricsState::Synced(ref synced) = lyrics_state {
                                let current = lyrics_scroll_center.unwrap_or_else(|| {
                                    info.as_ref()
                                        .map(|t| {
                                            let pm = current_progress_ms(t, fetch_anchor) as u64;
                                            synced.active_line_index(pm).unwrap_or(0)
                                        })
                                        .unwrap_or(0)
                                });
                                let max_line = synced.lines.len().saturating_sub(1);
                                lyrics_scroll_center = Some(if key.code == KeyCode::Char('j') {
                                    current.saturating_add(1).min(max_line)
                                } else {
                                    current.saturating_sub(1)
                                });
                                needs_redraw = true;
                            }
                        }
                    }
                    KeyCode::Char('s') => {
                        if lyrics_scroll_center.is_some() {
                            lyrics_scroll_center = None;
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Char('r') => {
                        last_fetch = Instant::now() - poll_interval;
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {
                    needs_redraw = true;
                }
                _ => {}
            }
        }

        // Handle deferred fetch (after next/prev)
        if let Some(at) = deferred_fetch {
            if Instant::now() >= at {
                deferred_fetch = None;
                last_fetch = Instant::now() - poll_interval;
            }
        }

        // Poll API
        if last_fetch.elapsed() >= poll_interval {
            if let Ok(new_info) = fetch_now_playing(spotify) {
                fetch_anchor = Instant::now();
                info = new_info;
                needs_redraw = true;
            }
            last_fetch = Instant::now();

            // Detect track change and trigger lyrics fetch.
            if let Some(ref track) = info {
                let identity = (
                    track.title.as_str(),
                    track.artist.as_str(),
                    track.album.as_str(),
                );
                let stored = lyrics_track
                    .as_ref()
                    .map(|(t, a, al)| (t.as_str(), a.as_str(), al.as_str()));
                if stored != Some(identity) {
                    let title = track.title.clone();
                    let artist = track.artist.clone();
                    let album = track.album.clone();
                    lyrics_track = Some((title.clone(), artist.clone(), album.clone()));
                    lyrics_state = LyricsState::Loading;
                    last_lyric_index = None;
                    lyrics_scroll_center = None;
                    let duration = track.duration_secs;
                    let (tx, rx) = mpsc::channel();
                    lyrics_rx = Some(rx);
                    std::thread::spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            lyrics::fetch_lyrics(&title, &artist, &album, duration)
                        }));
                        let state = match result {
                            Ok(s) => s,
                            Err(_) => {
                                eprintln!("lyrics: fetch thread panicked");
                                LyricsState::None
                            }
                        };
                        let _ = tx.send(state);
                    });
                }
            }
        }

        // Check for lyrics fetch result.
        if let Some(ref rx) = lyrics_rx {
            match rx.try_recv() {
                Ok(state) => {
                    lyrics_state = state;
                    lyrics_rx = None;
                    needs_redraw = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // catch_unwind ensures tx.send() always runs, so this is
                    // only reachable if panic=abort or the thread is killed.
                    lyrics_state = LyricsState::None;
                    lyrics_rx = None;
                    needs_redraw = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        // Draw
        match &info {
            Some(track) => {
                let progress_ms = current_progress_ms(track, fetch_anchor);
                let elapsed_secs = progress_ms / 1000;

                // Check if active lyric line changed.
                if show_lyrics {
                    let cur_idx = match &lyrics_state {
                        LyricsState::Synced(synced) => synced.active_line_index(progress_ms as u64),
                        _ => None,
                    };
                    if last_lyric_index != Some(cur_idx) {
                        last_lyric_index = Some(cur_idx);
                        needs_redraw = true;
                    }
                }

                let state = (elapsed_secs, elapsed_secs, track.is_playing, track.volume_percent);
                if needs_redraw || last_drawn.as_ref() != Some(&state) {
                    terminal.draw(|frame| {
                        draw_playing(
                            frame,
                            track,
                            progress_ms,
                            &lyrics_state,
                            show_lyrics,
                            lyrics_scroll_center,
                        );
                    })?;
                    last_drawn = Some(state);
                }
            }
            None => {
                if needs_redraw || last_drawn.is_some() {
                    terminal.draw(draw_empty)?;
                    last_drawn = None;
                }
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}
