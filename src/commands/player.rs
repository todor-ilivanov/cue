use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use rspotify::model::{PlayableId, PlayableItem, SearchResult, SearchType, TrackId};
use rspotify::prelude::*;
use rspotify::AuthCodeSpotify;

use super::queue::{fetch_queue_context, QueueContext, SongEntry};
use super::{api_error, join_artist_names};
use crate::lyrics::{self, LyricsState};
use crate::ui;

use std::sync::mpsc;
use std::time::{Duration, Instant};

// Amber/gold accent palette
const ACCENT: Color = Color::Rgb(255, 191, 0);
const SEPARATOR_COLOR: Color = Color::Rgb(60, 60, 60);

struct SearchResultEntry {
    title: String,
    artist: String,
    track_id: TrackId<'static>,
}

enum PlayerMode {
    Normal,
    SearchInput {
        query: String,
    },
    SearchLoading {
        query: String,
    },
    SearchResults {
        #[allow(dead_code)]
        query: String,
        results: Vec<SearchResultEntry>,
        selected: usize,
    },
}

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

fn build_queue_separator(width: u16) -> Line<'static> {
    let label = " queue ";
    let remaining = (width as usize).saturating_sub(label.len());
    let left = remaining / 2;
    let right = remaining - left;
    Line::from(vec![
        Span::styled("─".repeat(left), Style::new().fg(SEPARATOR_COLOR)),
        Span::styled(
            label,
            Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        ),
        Span::styled("─".repeat(right), Style::new().fg(SEPARATOR_COLOR)),
    ])
}

fn queue_entry_line(entry: &SongEntry) -> Line<'_> {
    Line::from(vec![
        Span::styled("  ", Style::new()),
        Span::styled(&entry.title, Style::new().fg(Color::DarkGray)),
        Span::styled(" \u{2014} ", Style::new().fg(Color::DarkGray)),
        Span::styled(
            &entry.artist,
            Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
        ),
    ])
}

fn draw_queue(frame: &mut Frame, area: Rect, ctx: &QueueContext) {
    if area.height == 0 {
        return;
    }

    let sep_area = Rect { height: 1, ..area };
    frame.render_widget(Paragraph::new(build_queue_separator(area.width)), sep_area);

    let mut y = area.y + 1;
    for entry in &ctx.next {
        if y >= area.y + area.height {
            break;
        }
        let row = Rect {
            y,
            height: 1,
            ..area
        };
        frame.render_widget(Paragraph::new(queue_entry_line(entry)), row);
        y += 1;
    }
}

fn content_rect(area: Rect) -> Rect {
    let max_width = 80u16;
    if area.width > max_width {
        let margin = (area.width - max_width) / 2;
        Rect::new(area.x + margin, area.y, max_width, area.height)
    } else {
        area
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_playing(
    frame: &mut Frame,
    info: &TrackInfo,
    progress_ms: i64,
    lyrics_state: &LyricsState,
    show_lyrics: bool,
    lyrics_scroll_center: Option<usize>,
    queue_context: Option<&QueueContext>,
    status_message: Option<&str>,
) {
    let content_area = content_rect(frame.area());
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

    // Queue section (separator + items)
    let queue_row = if let Some(ctx) = queue_context {
        if ctx.next.is_empty() {
            None
        } else {
            let r = constraints.len();
            constraints.push(Constraint::Length(1 + ctx.next.len() as u16));
            Some(r)
        }
    } else {
        None
    };

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

    // Queue
    if let Some(qr) = queue_row {
        if let Some(ctx) = queue_context {
            draw_queue(frame, rows[qr], ctx);
        }
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

    // Status message (transient, in blank line above hints)
    if let Some(msg) = status_message {
        let status_line = Line::from(Span::styled(msg, Style::new().fg(Color::Red)));
        frame.render_widget(Paragraph::new(status_line), rows[lyrics_row + 1]);
    }

    // Hints
    let hints = build_hints_playing(width);
    frame.render_widget(Paragraph::new(hints), rows[hints_row]);
}

fn perform_search(
    spotify: &AuthCodeSpotify,
    query: &str,
) -> Result<Vec<SearchResultEntry>, String> {
    let result = spotify
        .search(query, SearchType::Track, None, None, Some(5), None)
        .map_err(|e| format!("search failed: {e}"))?;

    let tracks = match result {
        SearchResult::Tracks(page) => page,
        _ => return Err("unexpected search result type".to_string()),
    };

    let entries: Vec<SearchResultEntry> = tracks
        .items
        .into_iter()
        .filter_map(|t| {
            let id = t.id?;
            Some(SearchResultEntry {
                title: t.name,
                artist: join_artist_names(&t.artists),
                track_id: id,
            })
        })
        .collect();

    if entries.is_empty() {
        Err(format!("no results for \"{query}\""))
    } else {
        Ok(entries)
    }
}

fn draw_search_input_bar(frame: &mut Frame, query: &str) {
    let content = content_rect(frame.area());
    let hints_area = Rect {
        y: content.y + content.height.saturating_sub(1),
        height: 1,
        ..content
    };

    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);

    let left = vec![
        Span::styled("/ ", key_style),
        Span::styled(query.to_string(), Style::new().fg(Color::White)),
        Span::styled("_", Style::new().fg(Color::DarkGray)),
    ];
    let left_width = 2 + query.len() + 1;

    let right_parts = vec![
        Span::styled("Enter", key_style),
        Span::styled(" search  ", desc_style),
        Span::styled("Esc", key_style),
        Span::styled(" cancel", desc_style),
    ];
    let right_width: usize = "Enter search  Esc cancel".len();

    let padding = (hints_area.width as usize).saturating_sub(left_width + right_width);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(padding)));
    spans.extend(right_parts);

    frame.render_widget(Paragraph::new(Line::from(spans)), hints_area);
}

fn draw_search_loading_overlay(frame: &mut Frame) {
    let content = content_rect(frame.area());
    let mid_y = content.y + content.height / 2;
    let row = Rect {
        y: mid_y,
        height: 1,
        ..content
    };
    let text = Line::from(Span::styled(
        "Searching...",
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    ));
    frame.render_widget(Paragraph::new(text), row);
}

fn draw_search_results_overlay(frame: &mut Frame, results: &[SearchResultEntry], selected: usize) {
    let content = content_rect(frame.area());
    let compact = content.height < 10;
    let top_margin: u16 = if content.height > 12 { 1 } else { 0 };
    let card_height: u16 = top_margin + 1 + 1 + if compact { 0 } else { 1 } + 1 + 1;
    let bottom_reserve = 2u16;
    let start_y = content.y + card_height;
    let available = content.height.saturating_sub(card_height + bottom_reserve);

    for (i, entry) in results.iter().enumerate() {
        if i as u16 >= available {
            break;
        }
        let row = Rect {
            y: start_y + i as u16,
            height: 1,
            ..content
        };
        let is_selected = i == selected;
        let prefix = if is_selected { "\u{25b6} " } else { "  " };
        let num = format!("{}. ", i + 1);
        let (title_style, artist_style) = if is_selected {
            (
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
                Style::new().fg(ACCENT),
            )
        } else {
            (
                Style::new().fg(Color::DarkGray),
                Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            )
        };
        let line = Line::from(vec![
            Span::styled(prefix, title_style),
            Span::styled(num, title_style),
            Span::styled(&entry.title, title_style),
            Span::styled(" \u{2014} ", Style::new().fg(Color::DarkGray)),
            Span::styled(&entry.artist, artist_style),
        ]);
        frame.render_widget(Paragraph::new(line), row);
    }
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
        Span::styled("esc", key_style),
        Span::styled(" quit", desc_style),
    ]);
    frame.render_widget(Paragraph::new(hints), rows[3]);
}

fn progress_bar_width(
    content_width: u16,
    progress_secs: i64,
    duration_secs: i64,
    volume: Option<u32>,
) -> usize {
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
        Span::styled(
            "\u{25cf}",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "\u{2500}".repeat(remaining),
            Style::new().fg(SEPARATOR_COLOR),
        ),
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
            Span::styled(" queue  ", desc_style),
            Span::styled("/", key_style),
            Span::styled(" srch  ", desc_style),
            Span::styled("esc", key_style),
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
            Span::styled(" queue  ", desc_style),
            Span::styled("/", key_style),
            Span::styled(" search  ", desc_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", desc_style),
        ])
    }
}

pub fn player(spotify: &AuthCodeSpotify, slim: bool) -> Result<()> {
    if !ui::is_interactive() {
        anyhow::bail!("player requires an interactive terminal");
    }

    crossterm::terminal::enable_raw_mode().context("failed to enable raw mode")?;

    let setup_result = (|| {
        let mut stdout = std::io::stdout();
        crossterm::execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )
        .context("failed to enter alternate screen")?;
        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        ratatui::Terminal::new(backend).context("failed to create terminal")
    })();

    let mut terminal = match setup_result {
        Ok(t) => t,
        Err(e) => {
            let _ = crossterm::terminal::disable_raw_mode();
            return Err(e);
        }
    };

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

    // Queue state
    let mut show_queue = false;
    let mut queue_context: Option<QueueContext> = None;

    // Search state
    let mut mode = PlayerMode::Normal;
    let mut search_rx: Option<mpsc::Receiver<Result<Vec<SearchResultEntry>, String>>> = None;

    // Transient status message (auto-clears after 3 seconds)
    let mut status_message: Option<(String, Instant)> = None;

    loop {
        let mut needs_redraw = false;

        // Clear expired status message
        if let Some((_, when)) = &status_message {
            if when.elapsed() > Duration::from_secs(3) {
                status_message = None;
                needs_redraw = true;
            }
        }

        // Process all pending key events
        while crossterm::event::poll(Duration::ZERO)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Collect deferred actions to avoid borrow conflicts with mode
                    let mut submit_search: Option<String> = None;
                    let mut play_track_id: Option<TrackId<'static>> = None;

                    match &mut mode {
                        PlayerMode::Normal => match key.code {
                            KeyCode::Esc => return Ok(()),
                            KeyCode::Char('/') => {
                                mode = PlayerMode::SearchInput {
                                    query: String::new(),
                                };
                                needs_redraw = true;
                            }
                            KeyCode::Char('q') => {
                                show_queue = !show_queue;
                                if show_queue && queue_context.is_none() {
                                    if let Ok(ctx) = fetch_queue_context(spotify, 0, 5) {
                                        queue_context = Some(ctx);
                                    }
                                }
                                needs_redraw = true;
                            }
                            KeyCode::Char(' ') => {
                                if let Some(ref mut t) = info {
                                    if t.is_playing {
                                        if let Err(e) = spotify.pause_playback(None) {
                                            status_message = Some((
                                                format!("pause failed: {e}"),
                                                Instant::now(),
                                            ));
                                        } else {
                                            t.is_playing = false;
                                        }
                                    } else {
                                        if let Err(e) = spotify.resume_playback(None, None) {
                                            status_message = Some((
                                                format!("resume failed: {e}"),
                                                Instant::now(),
                                            ));
                                        } else {
                                            t.is_playing = true;
                                        }
                                    }
                                    fetch_anchor = Instant::now();
                                    needs_redraw = true;
                                }
                            }
                            KeyCode::Char('n') | KeyCode::Char('p') => {
                                let result = if key.code == KeyCode::Char('n') {
                                    spotify.next_track(None)
                                } else {
                                    spotify.previous_track(None)
                                };
                                if let Err(e) = result {
                                    let action = if key.code == KeyCode::Char('n') {
                                        "next"
                                    } else {
                                        "prev"
                                    };
                                    status_message =
                                        Some((format!("{action} failed: {e}"), Instant::now()));
                                } else {
                                    deferred_fetch =
                                        Some(Instant::now() + Duration::from_millis(800));
                                }
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
                                    let new_ms =
                                        (current_ms + delta_ms).clamp(0, t.duration_secs * 1000);
                                    let pos = chrono::Duration::milliseconds(new_ms);
                                    if let Err(e) = spotify.seek_track(pos, None) {
                                        status_message =
                                            Some((format!("seek failed: {e}"), Instant::now()));
                                    } else {
                                        t.progress_ms = new_ms;
                                        fetch_anchor = Instant::now();
                                    }
                                    needs_redraw = true;
                                }
                            }
                            KeyCode::Up | KeyCode::Down => {
                                if let Some(ref mut t) = info {
                                    if let Some(current_vol) = t.volume_percent {
                                        let delta: i32 =
                                            if key.code == KeyCode::Up { 5 } else { -5 };
                                        let new_vol =
                                            (current_vol as i32 + delta).clamp(0, 100) as u8;
                                        if let Err(e) = spotify.volume(new_vol, None) {
                                            status_message = Some((
                                                format!("volume failed: {e}"),
                                                Instant::now(),
                                            ));
                                        } else {
                                            t.volume_percent = Some(new_vol as u32);
                                        }
                                        needs_redraw = true;
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
                                                    let pm =
                                                        current_progress_ms(t, fetch_anchor) as u64;
                                                    synced.active_line_index(pm).unwrap_or(0)
                                                })
                                                .unwrap_or(0)
                                        });
                                        let max_line = synced.lines.len().saturating_sub(1);
                                        lyrics_scroll_center =
                                            Some(if key.code == KeyCode::Char('j') {
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
                        PlayerMode::SearchInput { query } => match key.code {
                            KeyCode::Char(c) => {
                                query.push(c);
                                needs_redraw = true;
                            }
                            KeyCode::Backspace => {
                                query.pop();
                                needs_redraw = true;
                            }
                            KeyCode::Enter => {
                                if !query.is_empty() {
                                    submit_search = Some(query.clone());
                                }
                            }
                            KeyCode::Esc => {
                                mode = PlayerMode::Normal;
                                needs_redraw = true;
                            }
                            _ => {}
                        },
                        PlayerMode::SearchLoading { .. } => {
                            if key.code == KeyCode::Esc {
                                search_rx = None;
                                mode = PlayerMode::Normal;
                                needs_redraw = true;
                            }
                        }
                        PlayerMode::SearchResults {
                            results, selected, ..
                        } => match key.code {
                            KeyCode::Up | KeyCode::Char('k') => {
                                *selected = selected.saturating_sub(1);
                                needs_redraw = true;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                *selected = (*selected + 1).min(results.len().saturating_sub(1));
                                needs_redraw = true;
                            }
                            KeyCode::Enter => {
                                play_track_id = Some(results[*selected].track_id.clone());
                            }
                            KeyCode::Char(c @ '1'..='9') => {
                                let idx = (c as usize) - ('1' as usize);
                                if idx < results.len() {
                                    play_track_id = Some(results[idx].track_id.clone());
                                }
                            }
                            KeyCode::Esc => {
                                mode = PlayerMode::Normal;
                                needs_redraw = true;
                            }
                            _ => {}
                        },
                    }

                    // Handle deferred search submission (avoids borrow conflict)
                    if let Some(q) = submit_search {
                        let sp = spotify.clone();
                        let query = q.clone();
                        let (tx, rx) = mpsc::channel();
                        search_rx = Some(rx);
                        std::thread::spawn(move || {
                            let result = perform_search(&sp, &query);
                            let _ = tx.send(result);
                        });
                        mode = PlayerMode::SearchLoading { query: q };
                        needs_redraw = true;
                    }

                    // Handle deferred track play (avoids borrow conflict)
                    if let Some(track_id) = play_track_id {
                        let playable = PlayableId::Track(track_id);
                        match spotify.start_uris_playback([playable], None, None, None) {
                            Err(e) => {
                                status_message = Some((
                                    format!("{}", api_error(e, "start playback")),
                                    Instant::now(),
                                ));
                            }
                            Ok(()) => {
                                deferred_fetch = Some(Instant::now() + Duration::from_millis(800));
                            }
                        }
                        mode = PlayerMode::Normal;
                        needs_redraw = true;
                    }
                }
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

            // Re-fetch queue while visible.
            if show_queue {
                if let Ok(ctx) = fetch_queue_context(spotify, 0, 5) {
                    queue_context = Some(ctx);
                }
            }

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
                    lyrics_state = LyricsState::None;
                    lyrics_rx = None;
                    needs_redraw = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        // Check for search result.
        if let Some(ref rx) = search_rx {
            match rx.try_recv() {
                Ok(Ok(results)) => {
                    search_rx = None;
                    if let PlayerMode::SearchLoading { query } = &mode {
                        mode = PlayerMode::SearchResults {
                            query: query.clone(),
                            results,
                            selected: 0,
                        };
                    }
                    needs_redraw = true;
                }
                Ok(Err(msg)) => {
                    search_rx = None;
                    status_message = Some((msg, Instant::now()));
                    if let PlayerMode::SearchLoading { query } = &mode {
                        mode = PlayerMode::SearchInput {
                            query: query.clone(),
                        };
                    }
                    needs_redraw = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    search_rx = None;
                    status_message = Some(("search failed".to_string(), Instant::now()));
                    mode = PlayerMode::Normal;
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

                let state = (
                    elapsed_secs,
                    elapsed_secs,
                    track.is_playing,
                    track.volume_percent,
                );
                if needs_redraw
                    || last_drawn.as_ref() != Some(&state)
                    || !matches!(mode, PlayerMode::Normal)
                {
                    // Suppress lyrics/queue during search modes
                    let (effective_lyrics, effective_queue) = if matches!(mode, PlayerMode::Normal)
                    {
                        (
                            show_lyrics,
                            if show_queue {
                                queue_context.as_ref()
                            } else {
                                None
                            },
                        )
                    } else {
                        (false, None)
                    };

                    terminal.draw(|frame| {
                        draw_playing(
                            frame,
                            track,
                            progress_ms,
                            &lyrics_state,
                            effective_lyrics,
                            lyrics_scroll_center,
                            effective_queue,
                            status_message.as_ref().map(|(msg, _)| msg.as_str()),
                        );
                        match &mode {
                            PlayerMode::SearchInput { query } => {
                                draw_search_input_bar(frame, query);
                            }
                            PlayerMode::SearchLoading { .. } => {
                                draw_search_loading_overlay(frame);
                            }
                            PlayerMode::SearchResults {
                                results, selected, ..
                            } => {
                                draw_search_results_overlay(frame, results, *selected);
                            }
                            PlayerMode::Normal => {}
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use std::time::Instant;

    fn track(
        progress_ms: i64,
        duration_secs: i64,
        is_playing: bool,
        volume: Option<u32>,
    ) -> TrackInfo {
        TrackInfo {
            title: "Test".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            duration_secs,
            progress_ms,
            is_playing,
            volume_percent: volume,
        }
    }

    #[test]
    fn progress_paused() {
        let t = track(30_000, 240, false, None);
        let anchor = Instant::now();
        assert_eq!(current_progress_ms(&t, anchor), 30_000);
    }

    #[test]
    fn progress_playing() {
        let t = track(30_000, 240, true, None);
        let anchor = Instant::now();
        let result = current_progress_ms(&t, anchor);
        assert!(result >= 30_000);
    }

    #[test]
    fn progress_clamps_to_duration() {
        let t = track(239_990, 240, true, None);
        let anchor = Instant::now();
        std::thread::sleep(Duration::from_millis(50));
        let result = current_progress_ms(&t, anchor);
        assert!(result <= 240 * 1000);
    }

    #[test]
    fn progress_bar_width_with_volume() {
        // progress "1:05" (len=4), duration "4:00" (len=4), vol "  vol 75%" (len=9)
        // label_width = 2 + 4 + 1 + 1 + 4 + 9 = 21
        let w = progress_bar_width(80, 65, 240, Some(75));
        assert_eq!(w, 80 - 21);
    }

    #[test]
    fn progress_bar_width_no_volume() {
        // label_width = 2 + 4 + 1 + 1 + 4 + 0 = 12
        let w = progress_bar_width(80, 65, 240, None);
        assert_eq!(w, 80 - 12);
    }

    #[test]
    fn build_progress_line_span_count() {
        let area = Rect::new(0, 0, 80, 1);
        let line = build_progress_line(60_000, 240, true, Some(50), area);
        assert_eq!(line.spans.len(), 9);
    }
}
