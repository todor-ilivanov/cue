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
use super::queue::{fetch_queue_context, QueueContext};
use crate::lyrics::{self, LyricsState};
use crate::ui;

use std::sync::mpsc;
use std::time::{Duration, Instant};

struct TrackInfo {
    title: String,
    artist: String,
    album: String,
    duration_secs: i64,
    progress_ms: i64,
    is_playing: bool,
}

/// How many prev/next queue items to show based on available terminal height.
/// Fixed rows always present: track(1) + album(1) + progress(1) + hints(1) = 4.
/// Each direction with items also adds a 1-row separator.
const FIXED_ROWS: u16 = 4;
const MAX_QUEUE_PER_DIRECTION: usize = 3;

fn queue_depth(area_height: u16) -> (usize, usize) {
    let available = area_height.saturating_sub(FIXED_ROWS) as usize;
    // Each direction costs count + 1 separator when count > 0.
    // Try increasing total items until we run out of space.
    let mut next = 0;
    let mut prev = 0;
    let mut used = 0;
    // Alternate adding next, then prev
    loop {
        if next < MAX_QUEUE_PER_DIRECTION {
            let cost = if next == 0 { 2 } else { 1 }; // first item includes separator
            if used + cost > available {
                break;
            }
            next += 1;
            used += cost;
        }
        if prev < MAX_QUEUE_PER_DIRECTION {
            let cost = if prev == 0 { 2 } else { 1 };
            if used + cost > available {
                break;
            }
            prev += 1;
            used += cost;
        }
        if next >= MAX_QUEUE_PER_DIRECTION && prev >= MAX_QUEUE_PER_DIRECTION {
            break;
        }
    }
    (prev, next)
}

fn fetch_now_playing(spotify: &AuthCodeSpotify) -> Result<Option<TrackInfo>> {
    let context = spotify
        .current_playing(None, None::<&[_]>)
        .context("failed to get currently playing track")?;

    let Some(ctx) = context else {
        return Ok(None);
    };

    let is_playing = ctx.is_playing;
    let progress_ms = ctx.progress.map(|d| d.num_milliseconds()).unwrap_or(0);

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

fn draw_playing(
    frame: &mut Frame,
    info: &TrackInfo,
    progress_ms: i64,
    queue: &QueueContext,
    lyrics_state: &LyricsState,
    show_lyrics: bool,
) {
    let progress = progress_ms / 1000;
    let area = frame.area();
    let (prev_count, next_count) = queue_depth(area.height);

    let mut constraints: Vec<Constraint> = Vec::new();

    // Previous tracks
    for _ in 0..prev_count {
        constraints.push(Constraint::Length(1));
    }
    // Blank separator after prev tracks (only if we have prev tracks)
    if prev_count > 0 {
        constraints.push(Constraint::Length(1));
    }

    let track_row = constraints.len();
    constraints.push(Constraint::Length(1)); // current track

    let album_row = constraints.len();
    constraints.push(Constraint::Length(1)); // album

    let progress_row = constraints.len();
    constraints.push(Constraint::Length(1)); // progress bar

    // Blank separator before next tracks (only if we have next tracks)
    if next_count > 0 {
        constraints.push(Constraint::Length(1));
    }

    // Next tracks
    let next_start = constraints.len();
    for _ in 0..next_count {
        constraints.push(Constraint::Length(1));
    }

    let lyrics_row = constraints.len();
    constraints.push(Constraint::Min(0)); // lyrics or spacer
    let hints_row = constraints.len();
    constraints.push(Constraint::Length(1)); // hints

    let rows = Layout::vertical(constraints).split(area);

    // Previous tracks (dim, oldest first)
    let prev_items: Vec<&String> = queue.previous.iter().rev().take(prev_count).collect();
    for (i, line) in prev_items.iter().rev().enumerate() {
        let text = Line::from(Span::styled(
            format!("  {line}"),
            Style::new().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(text), rows[i]);
    }

    // Current track
    let track_line = Line::from(vec![
        Span::styled(
            &info.title,
            Style::new().add_modifier(Modifier::BOLD).fg(Color::White),
        ),
        Span::styled(" — ", Style::new().fg(Color::DarkGray)),
        Span::styled(&info.artist, Style::new().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(track_line), rows[track_row]);

    // Album
    let album_line = Line::from(Span::styled(&info.album, Style::new().fg(Color::DarkGray)));
    frame.render_widget(Paragraph::new(album_line), rows[album_row]);

    // Progress bar
    let progress_line = build_progress_line(
        progress,
        info.duration_secs,
        info.is_playing,
        rows[progress_row],
    );
    frame.render_widget(Paragraph::new(progress_line), rows[progress_row]);

    // Next tracks
    for (i, line) in queue.next.iter().take(next_count).enumerate() {
        let text = Line::from(Span::styled(
            format!("  {line}"),
            Style::new().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(text), rows[next_start + i]);
    }

    // Lyrics
    if show_lyrics {
        lyrics::draw_lyrics(frame, rows[lyrics_row], lyrics_state, progress_ms as u64);
    }

    // Hints
    let hints = build_hints_playing();
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

    let hints = Line::from(vec![
        Span::styled("r", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" refresh  ", Style::new().fg(Color::DarkGray)),
        Span::styled("q", Style::new().fg(Color::DarkGray)),
        Span::styled(" quit", Style::new().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(hints), rows[3]);
}

fn build_progress_line<'a>(progress: i64, duration: i64, is_playing: bool, area: Rect) -> Line<'a> {
    let status = if is_playing { ">" } else { "||" };
    let left = ui::format_duration(progress);
    let right = ui::format_duration(duration);

    let label_width = status.len() + 2 + left.len() + 1 + right.len() + 1;
    let bar_width = (area.width as usize).saturating_sub(label_width).min(50);

    let ratio = if duration > 0 {
        (progress as f64 / duration as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let filled = (bar_width as f64 * ratio).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    let filled_str: String = "━".repeat(filled);
    let empty_str: String = "─".repeat(empty);

    Line::from(vec![
        Span::raw(format!("{status}  {left} ")),
        Span::raw(filled_str),
        Span::styled(empty_str, Style::new().fg(Color::DarkGray)),
        Span::raw(format!(" {right}")),
    ])
}

fn build_hints_playing<'a>() -> Line<'a> {
    Line::from(vec![
        Span::styled("space", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" pause/resume  ", Style::new().fg(Color::DarkGray)),
        Span::styled("n/p", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" next/prev  ", Style::new().fg(Color::DarkGray)),
        Span::styled("</>", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" seek  ", Style::new().fg(Color::DarkGray)),
        Span::styled("l", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" lyrics  ", Style::new().fg(Color::DarkGray)),
        Span::styled("q", Style::new().fg(Color::DarkGray)),
        Span::styled(" quit", Style::new().fg(Color::DarkGray)),
    ])
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
    let mut queue = QueueContext {
        previous: Vec::new(),
        current: None,
        next: Vec::new(),
    };
    let mut last_drawn: Option<(i64, bool)> = None;
    let mut last_queue_depth: (usize, usize) = (0, 0);

    // Lyrics state
    let mut lyrics_state = LyricsState::None;
    let mut lyrics_rx: Option<mpsc::Receiver<LyricsState>> = None;
    let mut lyrics_track: Option<(String, String, String)> = None;
    let mut show_lyrics = !slim;
    let mut last_lyric_index: Option<Option<usize>> = None;

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
                    KeyCode::Char('l') => {
                        show_lyrics = !show_lyrics;
                        needs_redraw = true;
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

        // Track terminal size changes for layout (no API fetch — next poll handles it)
        let current_depth = queue_depth(terminal.size()?.height);
        if current_depth != last_queue_depth {
            last_queue_depth = current_depth;
            needs_redraw = true;
        }

        // Poll API
        if last_fetch.elapsed() >= poll_interval {
            if let Ok(new_info) = fetch_now_playing(spotify) {
                fetch_anchor = Instant::now();
                info = new_info;
                needs_redraw = true;
                if let Ok(ctx) =
                    fetch_queue_context(spotify, last_queue_depth.0, last_queue_depth.1)
                {
                    queue = ctx;
                }
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
                let progress_secs = progress_ms / 1000;

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

                let state = (progress_secs, track.is_playing);
                if needs_redraw || last_drawn.as_ref() != Some(&state) {
                    terminal.draw(|frame| {
                        draw_playing(
                            frame,
                            track,
                            progress_ms,
                            &queue,
                            &lyrics_state,
                            show_lyrics,
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

        std::thread::sleep(Duration::from_millis(200));
    }
}
