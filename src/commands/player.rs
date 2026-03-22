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

// Amber/gold accent palette
const ACCENT: Color = Color::Rgb(255, 191, 0);
const ACCENT_DIM: Color = Color::Rgb(153, 115, 0);
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

/// How many prev/next queue items to show based on available terminal height.
/// Fixed rows always present: track(1) + album(1) + progress(1) + hints(1) = 4.
/// Each direction with items also adds a 1-row separator line.
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
    let context = match spotify.current_playback(None, None::<&[_]>) {
        Ok(ctx) => ctx,
        Err(rspotify::ClientError::ParseJson(_)) => return Ok(None),
        Err(e) => return Err(anyhow::Error::from(e).context("failed to get current playback")),
    };

    let Some(ctx) = context else {
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
    queue: &QueueContext,
    lyrics_state: &LyricsState,
    show_lyrics: bool,
    lyrics_scroll_center: Option<usize>,
) {
    let progress = progress_ms / 1000;
    let area = frame.area();
    let height = area.height;
    let width = area.width;

    let compact = height < 10;
    let show_album = !compact;
    let show_top_margin = height > 12;
    let show_queue = height >= 8;

    let (prev_count, next_count) = if show_queue {
        queue_depth(area.height)
    } else {
        (0, 0)
    };

    let mut constraints: Vec<Constraint> = Vec::new();

    // Top margin for breathing room
    if show_top_margin {
        constraints.push(Constraint::Length(1));
    }

    // Previous tracks
    for _ in 0..prev_count {
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
    let prev_offset = if show_top_margin { 1 } else { 0 };
    for (i, line) in prev_items.iter().rev().enumerate() {
        let text = Line::from(Span::styled(
            format!("  {line}"),
            Style::new().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(text), rows[prev_offset + i]);
    }

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
        progress,
        info.duration_secs,
        info.is_playing,
        info.volume_percent,
        rows[progress_row],
    );
    frame.render_widget(Paragraph::new(progress_line), rows[progress_row]);

    // Separator below now-playing
    frame.render_widget(Paragraph::new(build_separator(width)), rows[sep_below_row]);

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

// Fractional block characters for smooth progress bar (eighths: 1/8 to 7/8)
const BLOCK_EIGHTHS: [char; 8] = [
    ' ', '\u{258f}', '\u{258e}', '\u{258d}', '\u{258c}', '\u{258b}', '\u{258a}', '\u{2589}',
];

fn build_progress_line<'a>(
    progress: i64,
    duration: i64,
    is_playing: bool,
    volume: Option<u32>,
    area: Rect,
) -> Line<'a> {
    let status = if is_playing { "\u{25b6}" } else { "\u{23f8}" };
    let left = ui::format_duration(progress);
    let right = ui::format_duration(duration);
    let vol_label = volume.map(|v| format!("  vol {v}%")).unwrap_or_default();

    let label_width = 2 + 1 + left.len() + 1 + right.len() + 1 + vol_label.len();
    let bar_width = (area.width as usize).saturating_sub(label_width);

    let ratio = if duration > 0 {
        (progress as f64 / duration as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Sub-character precision using fractional blocks
    let total_eighths = (bar_width as f64 * 8.0 * ratio).round() as usize;
    let full_blocks = total_eighths / 8;
    let remainder = total_eighths % 8;

    let filled_str: String = "\u{2588}".repeat(full_blocks);
    let frac_char = if remainder > 0 && full_blocks < bar_width {
        BLOCK_EIGHTHS[remainder].to_string()
    } else {
        String::new()
    };
    let empty_count = bar_width.saturating_sub(full_blocks + if remainder > 0 { 1 } else { 0 });
    let empty_str: String = " ".repeat(empty_count);

    Line::from(vec![
        Span::styled(format!("{status} "), Style::new().fg(ACCENT)),
        Span::styled(left, Style::new().fg(Color::White)),
        Span::raw(" "),
        Span::styled(filled_str, Style::new().fg(ACCENT)),
        Span::styled(frac_char, Style::new().fg(ACCENT_DIM)),
        Span::styled(empty_str, Style::new().fg(SEPARATOR_COLOR)),
        Span::raw(" "),
        Span::styled(right, Style::new().fg(Color::DarkGray)),
        Span::styled(vol_label, Style::new().fg(Color::DarkGray)),
    ])
}

fn build_hints_playing(width: u16) -> Line<'static> {
    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);

    if width < 60 {
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
    let mut queue = QueueContext {
        previous: Vec::new(),
        current: None,
        next: Vec::new(),
    };
    let mut last_drawn: Option<(i64, bool, Option<u32>)> = None;
    let mut last_queue_depth: (usize, usize) = (0, 0);

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
                // 250ms granularity for smooth progress bar updates
                let progress_tick = progress_ms / 250;

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

                let state = (progress_tick, track.is_playing, track.volume_percent);
                if needs_redraw || last_drawn.as_ref() != Some(&state) {
                    terminal.draw(|frame| {
                        draw_playing(
                            frame,
                            track,
                            progress_ms,
                            &queue,
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
