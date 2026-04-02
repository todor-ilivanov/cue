use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use rspotify::model::{
    AlbumId, PlayContextId, PlayableId, PlayableItem, PlaylistId, SearchResult, SearchType, TrackId,
};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchCategory {
    Track,
    Album,
    Playlist,
}

impl SearchCategory {
    fn next(self) -> Self {
        match self {
            Self::Track => Self::Album,
            Self::Album => Self::Playlist,
            Self::Playlist => Self::Track,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Track => "Track",
            Self::Album => "Album",
            Self::Playlist => "Playlist",
        }
    }
}

#[derive(Clone)]
enum SearchPlayTarget {
    Track(TrackId<'static>),
    Album(AlbumId<'static>),
    Playlist(PlaylistId<'static>),
}

#[derive(Clone)]
struct SearchResultEntry {
    title: String,
    subtitle: String,
    target: SearchPlayTarget,
}

enum PlayerMode {
    Normal,
    SearchInput {
        query: String,
        category: SearchCategory,
    },
    SearchLoading {
        query: String,
        category: SearchCategory,
    },
    SearchResults {
        #[allow(dead_code)]
        query: String,
        #[allow(dead_code)]
        category: SearchCategory,
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
    track_id: Option<String>,
    artist_id: Option<String>,
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

    let (artist, title, album, duration_secs, track_id, artist_id) = match &item {
        PlayableItem::Track(track) => (
            join_artist_names(&track.artists),
            track.name.clone(),
            track.album.name.clone(),
            track.duration.num_seconds(),
            track.id.as_ref().map(|id| id.id().to_string()),
            track
                .artists
                .first()
                .and_then(|a| a.id.as_ref())
                .map(|id| id.id().to_string()),
        ),
        PlayableItem::Episode(episode) => (
            episode.show.name.clone(),
            episode.name.clone(),
            String::new(),
            episode.duration.num_seconds(),
            None,
            None,
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
        track_id,
        artist_id,
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
    status_message: Option<(&str, Color)>,
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
    if let Some((msg, color)) = status_message {
        let status_line = Line::from(Span::styled(msg, Style::new().fg(color)));
        frame.render_widget(Paragraph::new(status_line), rows[lyrics_row + 1]);
    }

    // Hints
    let hints = build_hints_playing(width);
    frame.render_widget(Paragraph::new(hints), rows[hints_row]);
}

fn perform_search(
    spotify: &AuthCodeSpotify,
    query: &str,
    category: SearchCategory,
) -> Result<Vec<SearchResultEntry>, String> {
    match category {
        SearchCategory::Track => perform_track_search(spotify, query),
        SearchCategory::Album => perform_album_search(spotify, query),
        SearchCategory::Playlist => perform_playlist_search(spotify, query),
    }
}

fn perform_track_search(
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
                subtitle: join_artist_names(&t.artists),
                target: SearchPlayTarget::Track(id),
            })
        })
        .collect();

    if entries.is_empty() {
        Err(format!("no tracks for \"{query}\""))
    } else {
        Ok(entries)
    }
}

fn perform_album_search(
    spotify: &AuthCodeSpotify,
    query: &str,
) -> Result<Vec<SearchResultEntry>, String> {
    let result = spotify
        .search(query, SearchType::Album, None, None, Some(5), None)
        .map_err(|e| format!("search failed: {e}"))?;

    let albums = match result {
        SearchResult::Albums(page) => page,
        _ => return Err("unexpected search result type".to_string()),
    };

    let entries: Vec<SearchResultEntry> = albums
        .items
        .into_iter()
        .filter_map(|a| {
            let id = a.id?;
            Some(SearchResultEntry {
                title: a.name,
                subtitle: join_artist_names(&a.artists),
                target: SearchPlayTarget::Album(id),
            })
        })
        .collect();

    if entries.is_empty() {
        Err(format!("no albums for \"{query}\""))
    } else {
        Ok(entries)
    }
}

fn perform_playlist_search(
    spotify: &AuthCodeSpotify,
    query: &str,
) -> Result<Vec<SearchResultEntry>, String> {
    let playlists = crate::client::search_playlists(spotify, query, 5)
        .map_err(|e| format!("search failed: {e}"))?;

    let entries: Vec<SearchResultEntry> = playlists
        .items
        .into_iter()
        .map(|p| SearchResultEntry {
            title: p.name,
            subtitle: format!(
                "by {}",
                p.owner
                    .display_name
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            target: SearchPlayTarget::Playlist(p.id),
        })
        .collect();

    if entries.is_empty() {
        Err(format!("no playlists for \"{query}\""))
    } else {
        Ok(entries)
    }
}

fn draw_search_input_bar(frame: &mut Frame, query: &str, category: SearchCategory) {
    let content = content_rect(frame.area());
    let hints_area = Rect {
        y: content.y + content.height.saturating_sub(1),
        height: 1,
        ..content
    };

    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);
    let dim_style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM);
    let active_style = Style::new().fg(ACCENT);

    let categories = [
        SearchCategory::Track,
        SearchCategory::Album,
        SearchCategory::Playlist,
    ];
    let mut left: Vec<Span> = Vec::new();
    let mut left_width: usize = 0;
    for (i, &cat) in categories.iter().enumerate() {
        if i > 0 {
            left.push(Span::styled("/", dim_style));
            left_width += 1;
        }
        let label = cat.label();
        if cat == category {
            left.push(Span::styled(label, active_style));
        } else {
            left.push(Span::styled(label, dim_style));
        }
        left_width += label.len();
    }
    left.push(Span::styled(" ", Style::new()));
    left_width += 1;

    left.push(Span::styled("/ ", key_style));
    left.push(Span::styled(query, Style::new().fg(Color::White)));
    left.push(Span::styled("_", Style::new().fg(Color::DarkGray)));
    left_width += 2 + query.len() + 1;

    let right_parts = vec![
        Span::styled("Tab", key_style),
        Span::styled(" type  ", desc_style),
        Span::styled("Enter", key_style),
        Span::styled(" search  ", desc_style),
        Span::styled("Esc", key_style),
        Span::styled(" cancel", desc_style),
    ];
    let right_width: usize = right_parts.iter().map(|s| s.content.len()).sum();

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
            Span::styled(&entry.subtitle, artist_style),
        ]);
        frame.render_widget(Paragraph::new(line), row);
    }

    let hints_y = content.y + content.height.saturating_sub(1);
    let hints_row = Rect {
        y: hints_y,
        height: 1,
        ..content
    };
    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);
    let hints = Line::from(vec![
        Span::styled("Enter", key_style),
        Span::styled(" play  ", desc_style),
        Span::styled("a", key_style),
        Span::styled(" queue  ", desc_style),
        Span::styled("Esc", key_style),
        Span::styled(" cancel", desc_style),
    ]);
    frame.render_widget(Paragraph::new(hints), hints_row);
}

fn draw_empty(frame: &mut Frame, status_message: Option<(&str, Color)>) {
    let content = content_rect(frame.area());

    let rows = Layout::vertical([
        Constraint::Min(0),    // top spacer
        Constraint::Length(1), // "Not playing"
        Constraint::Length(2), // gap + action hint
        Constraint::Length(1), // status message
        Constraint::Min(0),    // bottom spacer
        Constraint::Length(1), // hints
    ])
    .split(content);

    let msg = Line::from(Span::styled("Not playing", Style::new().fg(Color::Gray)));
    frame.render_widget(
        Paragraph::new(msg).alignment(ratatui::layout::Alignment::Center),
        rows[1],
    );

    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);

    let action_hint = Line::from(vec![
        Span::styled("space", key_style),
        Span::styled(" resume  ", desc_style),
        Span::styled("/", key_style),
        Span::styled(" search", desc_style),
    ]);
    frame.render_widget(
        Paragraph::new(action_hint).alignment(ratatui::layout::Alignment::Center),
        rows[2],
    );

    if let Some((msg, color)) = status_message {
        let status = Line::from(Span::styled(msg, Style::new().fg(color)));
        frame.render_widget(
            Paragraph::new(status).alignment(ratatui::layout::Alignment::Center),
            rows[3],
        );
    }

    let hints = build_hints_empty(content.width);
    frame.render_widget(Paragraph::new(hints), rows[5]);
}

fn build_hints_empty(width: u16) -> Line<'static> {
    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::DarkGray);

    if width < 50 {
        Line::from(vec![
            Span::styled("spc", key_style),
            Span::styled(" resume  ", desc_style),
            Span::styled("/", key_style),
            Span::styled(" search  ", desc_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", desc_style),
        ])
    } else {
        Line::from(vec![
            Span::styled("space", key_style),
            Span::styled(" resume  ", desc_style),
            Span::styled("/", key_style),
            Span::styled(" search  ", desc_style),
            Span::styled("r", key_style),
            Span::styled(" refresh  ", desc_style),
            Span::styled("?", key_style),
            Span::styled(" help  ", desc_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", desc_style),
        ])
    }
}

fn draw_mode_overlays(frame: &mut Frame, mode: &PlayerMode, show_help: bool) {
    match mode {
        PlayerMode::SearchInput { query, category } => {
            draw_search_input_bar(frame, query, *category);
        }
        PlayerMode::SearchLoading { .. } => {
            draw_search_loading_overlay(frame);
        }
        PlayerMode::SearchResults {
            results, selected, ..
        } => {
            draw_search_results_overlay(frame, results, *selected);
        }
        PlayerMode::Normal => {
            if show_help {
                draw_help_overlay(frame);
            }
        }
    }
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

    if width < 45 {
        Line::from(vec![
            Span::styled("spc", key_style),
            Span::styled(" \u{23ef}  ", desc_style),
            Span::styled("n/p", key_style),
            Span::styled(" \u{23ed}/\u{23ee}  ", desc_style),
            Span::styled("?", key_style),
            Span::styled(" help  ", desc_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", desc_style),
        ])
    } else if width < 75 {
        Line::from(vec![
            Span::styled("spc", key_style),
            Span::styled(" play  ", desc_style),
            Span::styled("n/p", key_style),
            Span::styled(" next/prev  ", desc_style),
            Span::styled("\u{2190}/\u{2192}", key_style),
            Span::styled(" seek  ", desc_style),
            Span::styled("\u{2191}/\u{2193}", key_style),
            Span::styled(" vol  ", desc_style),
            Span::styled("?", key_style),
            Span::styled(" help  ", desc_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", desc_style),
        ])
    } else {
        Line::from(vec![
            Span::styled("space", key_style),
            Span::styled(" pause/resume  ", desc_style),
            Span::styled("n/p", key_style),
            Span::styled(" next/prev  ", desc_style),
            Span::styled("\u{2190}/\u{2192}", key_style),
            Span::styled(" seek  ", desc_style),
            Span::styled("\u{2191}/\u{2193}", key_style),
            Span::styled(" volume  ", desc_style),
            Span::styled("?", key_style),
            Span::styled(" help  ", desc_style),
            Span::styled("esc", key_style),
            Span::styled(" quit", desc_style),
        ])
    }
}

fn draw_help_overlay(frame: &mut Frame) {
    let area = frame.area();
    let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
    let desc_style = Style::new().fg(Color::Gray);
    let dim_style = Style::new().fg(Color::DarkGray);

    let bindings: &[(&str, &str)] = &[
        ("space", "Pause / resume"),
        ("n / p", "Next / previous track"),
        ("← / →", "Seek backward / forward 5s"),
        ("↑ / ↓", "Volume up / down"),
        ("l", "Toggle lyrics"),
        ("j / k", "Scroll lyrics"),
        ("s", "Sync lyrics to playback"),
        ("q", "Toggle queue"),
        ("/", "Search tracks, albums, playlists"),
        ("a", "Queue track from search results"),
        ("R", "Start song radio"),
        ("r", "Refresh now playing"),
        ("esc", "Quit"),
    ];

    let box_width: u16 = 48;
    let box_height: u16 = (bindings.len() as u16) + 4;

    let x = area.x + area.width.saturating_sub(box_width) / 2;
    let y = area.y + area.height.saturating_sub(box_height) / 2;
    let overlay = Rect::new(x, y, box_width.min(area.width), box_height.min(area.height));

    frame.render_widget(ratatui::widgets::Clear, overlay);

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::new().fg(SEPARATOR_COLOR))
        .title(Span::styled(
            " Keyboard shortcuts ",
            Style::new().fg(ACCENT),
        ));
    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    for (i, &(key, desc)) in bindings.iter().enumerate() {
        let row = Rect {
            x: inner.x + 2,
            y: inner.y + 1 + i as u16,
            width: inner.width.saturating_sub(2),
            height: 1,
        };
        if row.y >= inner.y + inner.height {
            break;
        }
        let cols = Layout::horizontal([Constraint::Length(10), Constraint::Min(0)]).split(row);
        frame.render_widget(Paragraph::new(Span::styled(key, key_style)), cols[0]);
        frame.render_widget(Paragraph::new(Span::styled(desc, desc_style)), cols[1]);
    }

    let dismiss_area = Rect {
        y: overlay.y + overlay.height,
        height: 1,
        ..overlay
    };
    if dismiss_area.y < area.y + area.height {
        let dismiss = Line::from(vec![
            Span::styled("Press ", dim_style),
            Span::styled("?", key_style),
            Span::styled(" or ", dim_style),
            Span::styled("esc", key_style),
            Span::styled(" to close", dim_style),
        ]);
        frame.render_widget(
            Paragraph::new(dismiss).alignment(ratatui::layout::Alignment::Center),
            dismiss_area,
        );
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
    let mut show_help = false;
    let mut queue_context: Option<QueueContext> = None;

    // Search state
    let mut mode = PlayerMode::Normal;
    let mut search_rx: Option<mpsc::Receiver<Result<Vec<SearchResultEntry>, String>>> = None;

    // Transient status message (auto-clears after 3 seconds)
    let mut status_message: Option<(String, Instant, Color)> = None;

    loop {
        let mut needs_redraw = false;

        // Clear expired status message
        if let Some((_, when, _)) = &status_message {
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
                    let mut submit_search: Option<(String, SearchCategory)> = None;
                    let mut play_target: Option<SearchPlayTarget> = None;
                    let mut queue_target: Option<SearchResultEntry> = None;
                    #[allow(clippy::type_complexity)]
                    let mut start_radio: Option<(
                        Option<String>,
                        Option<String>,
                        String,
                        String,
                    )> = None;

                    match &mut mode {
                        PlayerMode::Normal => match key.code {
                            KeyCode::Esc => return Ok(()),
                            KeyCode::Char('/') => {
                                mode = PlayerMode::SearchInput {
                                    query: String::new(),
                                    category: SearchCategory::Track,
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
                                                Color::Red,
                                            ));
                                        } else {
                                            t.is_playing = false;
                                        }
                                    } else if let Err(e) = spotify.resume_playback(None, None) {
                                        status_message = Some((
                                            format!("resume failed: {e}"),
                                            Instant::now(),
                                            Color::Red,
                                        ));
                                    } else {
                                        t.is_playing = true;
                                    }
                                    fetch_anchor = Instant::now();
                                    needs_redraw = true;
                                } else {
                                    match spotify.resume_playback(None, None) {
                                        Err(e) => {
                                            status_message = Some((
                                                format!("resume failed: {e}"),
                                                Instant::now(),
                                                Color::Red,
                                            ));
                                        }
                                        Ok(()) => {
                                            deferred_fetch =
                                                Some(Instant::now() + Duration::from_millis(800));
                                        }
                                    }
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
                                    status_message = Some((
                                        format!("{action} failed: {e}"),
                                        Instant::now(),
                                        Color::Red,
                                    ));
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
                                        status_message = Some((
                                            format!("seek failed: {e}"),
                                            Instant::now(),
                                            Color::Red,
                                        ));
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
                                                Color::Red,
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
                                    let bounds = match lyrics_state {
                                        LyricsState::Synced(ref synced) => {
                                            let current =
                                                lyrics_scroll_center.unwrap_or_else(|| {
                                                    info.as_ref()
                                                        .map(|t| {
                                                            let pm = current_progress_ms(
                                                                t,
                                                                fetch_anchor,
                                                            )
                                                                as u64;
                                                            synced
                                                                .active_line_index(pm)
                                                                .unwrap_or(0)
                                                        })
                                                        .unwrap_or(0)
                                                });
                                            Some((current, synced.lines.len().saturating_sub(1)))
                                        }
                                        LyricsState::Plain(ref text) => Some((
                                            lyrics_scroll_center.unwrap_or(0),
                                            text.lines().count().saturating_sub(1),
                                        )),
                                        _ => None,
                                    };
                                    if let Some((current, max_line)) = bounds {
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
                            KeyCode::Char('R') => {
                                if let Some(ref t) = info {
                                    start_radio = Some((
                                        t.track_id.clone(),
                                        t.artist_id.clone(),
                                        t.title.clone(),
                                        t.artist.clone(),
                                    ));
                                } else {
                                    status_message = Some((
                                        "no track is currently playing".to_string(),
                                        Instant::now(),
                                        Color::Red,
                                    ));
                                    needs_redraw = true;
                                }
                            }
                            KeyCode::Char('?') => {
                                show_help = !show_help;
                                needs_redraw = true;
                            }
                            _ => {}
                        },
                        PlayerMode::SearchInput { query, category } => match key.code {
                            KeyCode::Char(c) => {
                                query.push(c);
                                needs_redraw = true;
                            }
                            KeyCode::Backspace => {
                                query.pop();
                                needs_redraw = true;
                            }
                            KeyCode::Tab | KeyCode::BackTab => {
                                *category = category.next();
                                needs_redraw = true;
                            }
                            KeyCode::Enter => {
                                if !query.is_empty() {
                                    submit_search = Some((query.clone(), *category));
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
                                play_target = Some(results[*selected].target.clone());
                            }
                            KeyCode::Char(c @ '1'..='9') => {
                                let idx = (c as usize) - ('1' as usize);
                                if idx < results.len() {
                                    play_target = Some(results[idx].target.clone());
                                }
                            }
                            KeyCode::Char('a') => {
                                queue_target = Some(results[*selected].clone());
                            }
                            KeyCode::Esc => {
                                mode = PlayerMode::Normal;
                                needs_redraw = true;
                            }
                            _ => {}
                        },
                    }

                    // Handle deferred search submission (avoids borrow conflict)
                    if let Some((q, cat)) = submit_search {
                        let sp = spotify.clone();
                        let (tx, rx) = mpsc::channel();
                        search_rx = Some(rx);
                        mode = PlayerMode::SearchLoading {
                            query: q.clone(),
                            category: cat,
                        };
                        std::thread::spawn(move || {
                            let result = perform_search(&sp, &q, cat);
                            let _ = tx.send(result);
                        });
                        needs_redraw = true;
                    }

                    // Handle deferred play action (avoids borrow conflict)
                    if let Some(target) = play_target {
                        let result = match target {
                            SearchPlayTarget::Track(id) => spotify.start_uris_playback(
                                [PlayableId::Track(id)],
                                None,
                                None,
                                None,
                            ),
                            SearchPlayTarget::Album(id) => spotify.start_context_playback(
                                PlayContextId::Album(id),
                                None,
                                None,
                                None,
                            ),
                            SearchPlayTarget::Playlist(id) => spotify.start_context_playback(
                                PlayContextId::Playlist(id),
                                None,
                                None,
                                None,
                            ),
                        };
                        match result {
                            Err(e) => {
                                status_message = Some((
                                    format!("{}", api_error(e, "start playback")),
                                    Instant::now(),
                                    Color::Red,
                                ));
                            }
                            Ok(()) => {
                                deferred_fetch = Some(Instant::now() + Duration::from_millis(800));
                            }
                        }
                        mode = PlayerMode::Normal;
                        needs_redraw = true;
                    }

                    if let Some(entry) = queue_target {
                        match entry.target {
                            SearchPlayTarget::Track(id) => {
                                let playable = PlayableId::Track(id);
                                match spotify.add_item_to_queue(playable, None) {
                                    Ok(()) => {
                                        status_message = Some((
                                            format!(
                                                "Queued: {} \u{2014} {}",
                                                entry.title, entry.subtitle
                                            ),
                                            Instant::now(),
                                            Color::Green,
                                        ));
                                    }
                                    Err(e) => {
                                        status_message = Some((
                                            format!("{}", api_error(e, "add to queue")),
                                            Instant::now(),
                                            Color::Red,
                                        ));
                                    }
                                }
                            }
                            SearchPlayTarget::Album(_) | SearchPlayTarget::Playlist(_) => {
                                status_message = Some((
                                    "Only tracks can be added to queue".to_string(),
                                    Instant::now(),
                                    Color::Red,
                                ));
                            }
                        }
                        mode = PlayerMode::Normal;
                        needs_redraw = true;
                    }

                    if let Some((tid, aid, title, artist)) = start_radio {
                        match tid {
                            Some(track_id) => {
                                match crate::client::fetch_recommendations(
                                    spotify,
                                    &track_id,
                                    aid.as_deref(),
                                    50,
                                ) {
                                    Ok(recs) if recs.is_empty() => {
                                        status_message = Some((
                                            "no recommendations found for this track".to_string(),
                                            Instant::now(),
                                            Color::Red,
                                        ));
                                    }
                                    Ok(recs) => {
                                        let mut uris: Vec<PlayableId> = Vec::new();
                                        if let Ok(id) = TrackId::from_id(&track_id) {
                                            uris.push(PlayableId::Track(id));
                                        }
                                        for rec in &recs {
                                            if let Ok(id) = TrackId::from_id(&rec.id) {
                                                uris.push(PlayableId::Track(id));
                                            }
                                        }
                                        match spotify.start_uris_playback(uris, None, None, None) {
                                            Ok(()) => {
                                                status_message = Some((
                                                    format!(
                                                        "Radio: {} tracks queued for {} \u{2014} {}",
                                                        recs.len(),
                                                        title,
                                                        artist
                                                    ),
                                                    Instant::now(),
                                                    Color::Green,
                                                ));
                                                deferred_fetch = Some(
                                                    Instant::now() + Duration::from_millis(800),
                                                );
                                            }
                                            Err(e) => {
                                                status_message = Some((
                                                    format!(
                                                        "{}",
                                                        api_error(e, "start radio playback")
                                                    ),
                                                    Instant::now(),
                                                    Color::Red,
                                                ));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        status_message = Some((
                                            format!("radio failed: {e}"),
                                            Instant::now(),
                                            Color::Red,
                                        ));
                                    }
                                }
                                needs_redraw = true;
                            }
                            None => {
                                status_message = Some((
                                    "current track has no ID".to_string(),
                                    Instant::now(),
                                    Color::Red,
                                ));
                                needs_redraw = true;
                            }
                        }
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
                    if let PlayerMode::SearchLoading { query, category } = &mode {
                        mode = PlayerMode::SearchResults {
                            query: query.clone(),
                            category: *category,
                            results,
                            selected: 0,
                        };
                    }
                    needs_redraw = true;
                }
                Ok(Err(msg)) => {
                    search_rx = None;
                    status_message = Some((msg, Instant::now(), Color::Red));
                    if let PlayerMode::SearchLoading { query, category } = &mode {
                        mode = PlayerMode::SearchInput {
                            query: query.clone(),
                            category: *category,
                        };
                    }
                    needs_redraw = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    search_rx = None;
                    status_message =
                        Some(("search failed".to_string(), Instant::now(), Color::Red));
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
                            status_message
                                .as_ref()
                                .map(|(msg, _, color)| (msg.as_str(), *color)),
                        );
                        draw_mode_overlays(frame, &mode, show_help);
                    })?;
                    last_drawn = Some(state);
                }
            }
            None => {
                if needs_redraw || last_drawn.is_some() {
                    terminal.draw(|frame| {
                        draw_empty(
                            frame,
                            status_message
                                .as_ref()
                                .map(|(msg, _, color)| (msg.as_str(), *color)),
                        );
                        draw_mode_overlays(frame, &mode, show_help);
                    })?;
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

    #[test]
    fn search_category_cycles_forward() {
        assert_eq!(SearchCategory::Track.next(), SearchCategory::Album);
        assert_eq!(SearchCategory::Album.next(), SearchCategory::Playlist);
        assert_eq!(SearchCategory::Playlist.next(), SearchCategory::Track);
    }
}
