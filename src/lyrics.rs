use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use serde::Deserialize;

const ACCENT: Color = Color::Rgb(255, 191, 0);
const SEPARATOR_COLOR: Color = Color::Rgb(60, 60, 60);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

pub struct LyricLine {
    pub timestamp_ms: u64,
    pub text: String,
}

pub struct SyncedLyrics {
    pub lines: Vec<LyricLine>,
}

impl SyncedLyrics {
    /// Binary search for the active line at the given playback position.
    pub fn active_line_index(&self, position_ms: u64) -> Option<usize> {
        match self
            .lines
            .partition_point(|l| l.timestamp_ms <= position_ms)
        {
            0 => None,
            i => Some(i - 1),
        }
    }
}

pub enum LyricsState {
    Loading,
    Synced(SyncedLyrics),
    Plain(String),
    Instrumental,
    None,
}

// ---------------------------------------------------------------------------
// LRCLIB API client
// ---------------------------------------------------------------------------

const USER_AGENT: &str = "cue/0.1.0 (https://github.com/cue-rs/cue)";
const API_BASE: &str = "https://lrclib.net/api";

fn shared_agent() -> &'static ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(build_agent)
}

fn build_agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(5));

    // Use system certificate store so TLS-intercepting proxies are trusted.
    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = root_store.add(cert);
    }
    if !root_store.is_empty() {
        if let Ok(config_builder) = rustls::ClientConfig::builder_with_provider(
            std::sync::Arc::new(rustls::crypto::ring::default_provider()),
        )
        .with_safe_default_protocol_versions()
        {
            let tls_config = config_builder
                .with_root_certificates(root_store)
                .with_no_client_auth();
            builder = builder.tls_config(std::sync::Arc::new(tls_config));
        }
    }

    if let Ok(url) = std::env::var("https_proxy").or_else(|_| std::env::var("HTTPS_PROXY")) {
        if let Ok(proxy) = ureq::Proxy::new(&url) {
            builder = builder.proxy(proxy);
        }
    }

    builder.build()
}

#[derive(Deserialize)]
struct LrclibResponse {
    #[serde(default)]
    instrumental: bool,
    #[serde(rename = "syncedLyrics")]
    synced_lyrics: Option<String>,
    #[serde(rename = "plainLyrics")]
    plain_lyrics: Option<String>,
}

pub fn fetch_lyrics(title: &str, artist: &str, album: &str, duration_secs: i64) -> LyricsState {
    let agent = shared_agent();

    if let Some(state) = try_exact_match(agent, title, artist, album, duration_secs) {
        return state;
    }
    if let Some(state) = try_search(agent, title, artist) {
        return state;
    }
    LyricsState::None
}

fn try_exact_match(
    agent: &ureq::Agent,
    title: &str,
    artist: &str,
    album: &str,
    duration_secs: i64,
) -> Option<LyricsState> {
    let resp = agent
        .get(&format!("{API_BASE}/get"))
        .set("User-Agent", USER_AGENT)
        .query("track_name", title)
        .query("artist_name", artist)
        .query("album_name", album)
        .query("duration", &duration_secs.to_string())
        .call();

    match resp {
        Ok(r) => match r.into_json::<LrclibResponse>() {
            Ok(parsed) => response_to_state(parsed),
            Err(e) => {
                eprintln!("lyrics: failed to parse exact match response: {e}");
                None
            }
        },
        Err(ureq::Error::Status(404, _)) => None,
        Err(e) => {
            eprintln!("lyrics: exact match request failed: {e}");
            None
        }
    }
}

fn try_search(agent: &ureq::Agent, title: &str, artist: &str) -> Option<LyricsState> {
    let resp = match agent
        .get(&format!("{API_BASE}/search"))
        .set("User-Agent", USER_AGENT)
        .query("track_name", title)
        .query("artist_name", artist)
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lyrics: search request failed: {e}");
            return None;
        }
    };

    let results: Vec<LrclibResponse> = match resp.into_json() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("lyrics: failed to parse search response: {e}");
            return None;
        }
    };

    // Prefer the first result with synced lyrics, fall back to plain.
    let mut fallback: Option<LrclibResponse> = None;

    for r in results {
        if r.synced_lyrics.is_some() {
            if let Some(state) = response_to_state(r) {
                return Some(state);
            }
            continue;
        }
        if fallback.is_none() && (r.plain_lyrics.is_some() || r.instrumental) {
            fallback = Some(r);
        }
    }

    fallback.and_then(response_to_state)
}

fn response_to_state(resp: LrclibResponse) -> Option<LyricsState> {
    if resp.instrumental {
        return Some(LyricsState::Instrumental);
    }
    if let Some(ref synced) = resp.synced_lyrics {
        let parsed = parse_lrc(synced);
        if !parsed.lines.is_empty() {
            return Some(LyricsState::Synced(parsed));
        }
    }
    if let Some(plain) = resp.plain_lyrics {
        if !plain.trim().is_empty() {
            return Some(LyricsState::Plain(plain));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// LRC parser
// ---------------------------------------------------------------------------

fn parse_lrc(input: &str) -> SyncedLyrics {
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut offset_ms: i64 = 0;

    for raw in input.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }

        // Check for offset metadata tag: [offset:+/-N]
        if let Some(val) = raw
            .strip_prefix("[offset:")
            .and_then(|s| s.strip_suffix(']'))
        {
            offset_ms = val.trim().parse::<i64>().unwrap_or(0);
            continue;
        }

        // Extract all timestamps and the trailing text.
        let mut timestamps = Vec::new();
        let mut rest = raw;

        while let Some(start) = rest.find('[') {
            let after_bracket = &rest[start + 1..];
            let Some(end) = after_bracket.find(']') else {
                break;
            };
            let tag = &after_bracket[..end];
            rest = &after_bracket[end + 1..];

            if let Some(ms) = parse_timestamp(tag) {
                timestamps.push(ms);
            } else {
                // Non-timestamp tag (metadata) — skip entire line.
                break;
            }
        }

        if timestamps.is_empty() {
            continue;
        }

        let text = rest.trim().to_string();

        for ts in timestamps {
            lines.push(LyricLine {
                timestamp_ms: ts,
                text: text.clone(),
            });
        }
    }

    // Apply offset: positive offset means lyrics appear earlier (subtract from timestamps).
    if offset_ms != 0 {
        for line in &mut lines {
            let adjusted = line.timestamp_ms as i64 - offset_ms;
            line.timestamp_ms = adjusted.max(0) as u64;
        }
    }

    lines.sort_by_key(|l| l.timestamp_ms);
    SyncedLyrics { lines }
}

/// Parse a timestamp tag like `01:23.45` or `01:23.456` into milliseconds.
fn parse_timestamp(tag: &str) -> Option<u64> {
    let (min_str, rest) = tag.split_once(':')?;
    let (sec_str, frac_str) = rest.split_once('.')?;

    let minutes: u64 = min_str.parse().ok()?;
    let seconds: u64 = sec_str.parse().ok()?;

    let frac_ms: u64 = match frac_str.len() {
        2 => frac_str.parse::<u64>().ok()? * 10, // centiseconds
        3 => frac_str.parse::<u64>().ok()?,      // milliseconds
        1 => frac_str.parse::<u64>().ok()? * 100,
        _ => return None,
    };

    Some(minutes * 60_000 + seconds * 1000 + frac_ms)
}

// ---------------------------------------------------------------------------
// Lyrics widget
// ---------------------------------------------------------------------------

fn build_lyrics_separator(width: u16) -> Line<'static> {
    let label = " lyrics ";
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

pub fn draw_lyrics(
    frame: &mut Frame,
    area: Rect,
    state: &LyricsState,
    position_ms: u64,
    scroll_center: Option<usize>,
) {
    if area.height < 2 {
        return;
    }

    let has_content = matches!(state, LyricsState::Synced(_) | LyricsState::Plain(_));

    // Only show the lyrics separator when there is actual content.
    // For status states (Loading/None/Instrumental), the separator below
    // the now-playing card already provides visual separation.
    let content_y_offset = if has_content {
        let sep_area = Rect { height: 1, ..area };
        frame.render_widget(Paragraph::new(build_lyrics_separator(area.width)), sep_area);
        if area.height > 4 {
            2
        } else {
            1
        }
    } else {
        0
    };

    let content_area = Rect {
        y: area.y + content_y_offset,
        height: area.height.saturating_sub(content_y_offset),
        ..area
    };

    if content_area.height == 0 {
        return;
    }

    match state {
        LyricsState::Loading => {
            draw_centered_dim(frame, content_area, "Loading lyrics...");
        }
        LyricsState::None => {
            draw_centered_dim(frame, content_area, "No lyrics available");
        }
        LyricsState::Instrumental => {
            draw_centered_dim(frame, content_area, "Instrumental");
        }
        LyricsState::Plain(text) => {
            draw_plain(frame, content_area, text, scroll_center.unwrap_or(0));
        }
        LyricsState::Synced(synced) => {
            draw_synced(frame, content_area, synced, position_ms, scroll_center);
        }
    }
}

fn draw_centered_dim(frame: &mut Frame, area: Rect, msg: &str) {
    if area.height == 0 {
        return;
    }
    let centered_area = Rect {
        x: area.x,
        y: area.y + area.height / 2,
        width: area.width,
        height: 1,
    };
    let line = Line::from(Span::styled(
        msg,
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    ));
    frame.render_widget(Paragraph::new(line), centered_area);
}

fn draw_plain(frame: &mut Frame, area: Rect, text: &str, scroll_offset: usize) {
    if area.height == 0 {
        return;
    }
    // Show header
    let header = Line::from(Span::styled(
        "(unsynced)",
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    ));
    frame.render_widget(Paragraph::new(header), Rect { height: 1, ..area });

    if area.height <= 1 {
        return;
    }
    let text_area = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let plain_lines: Vec<Line> = text
        .lines()
        .map(|l| Line::from(Span::styled(l, Style::new().fg(Color::Gray))))
        .collect();
    let paragraph = Paragraph::new(plain_lines)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .scroll((scroll_offset.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(paragraph, text_area);
}

/// Compute the first visible line index for the lyrics viewport.
/// Both auto and manual modes clamp to [0, total_lines - window] so we
/// never show blank lines above the first lyric or below the last.
fn viewport_start(center: usize, window: usize, total_lines: usize, manual: bool) -> isize {
    let anchor = window / 2;
    let raw = if manual {
        center as isize - anchor as isize
    } else {
        center.saturating_sub(anchor) as isize
    };
    let max_start = (total_lines as isize - window as isize).max(0);
    raw.max(0).min(max_start)
}

fn draw_synced(
    frame: &mut Frame,
    area: Rect,
    synced: &SyncedLyrics,
    position_ms: u64,
    scroll_center: Option<usize>,
) {
    let height = area.height as usize;
    if height == 0 {
        return;
    }

    let active = synced.active_line_index(position_ms);

    let window = height;

    let center = scroll_center.or(active).unwrap_or(0);
    let virtual_start = viewport_start(center, window, synced.lines.len(), scroll_center.is_some());

    let num_lines = synced.lines.len() as isize;
    let mut rendered_lines: Vec<Line> = Vec::with_capacity(window);

    for row in 0..window {
        let line_idx_signed = virtual_start + row as isize;
        if line_idx_signed < 0 || line_idx_signed >= num_lines {
            rendered_lines.push(Line::from(""));
            continue;
        }
        let line_idx = line_idx_signed as usize;

        let lyric = &synced.lines[line_idx];
        let text = if lyric.text.is_empty() {
            "..."
        } else {
            &lyric.text
        };

        let style = match active {
            Some(ai) if line_idx == ai => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            Some(ai) if line_idx < ai => Style::new().fg(Color::DarkGray),
            _ => Style::new().fg(Color::Gray).add_modifier(Modifier::DIM),
        };

        rendered_lines.push(Line::from(Span::styled(text, style)));
    }

    let lyrics_area = Rect {
        height: window as u16,
        ..area
    };
    let paragraph = Paragraph::new(rendered_lines);
    frame.render_widget(paragraph, lyrics_area);

    // Distance indicator when in manual scroll mode
    if scroll_center.is_some() {
        if let Some(ai) = active {
            let visible_start = virtual_start.max(0) as usize;
            let visible_end = (virtual_start + window as isize).max(0) as usize;
            let (arrow, dist, at_top) = if ai < visible_start {
                ("\u{25b2}", visible_start - ai, true)
            } else if ai >= visible_end {
                ("\u{25bc}", ai - visible_end + 1, false)
            } else {
                return;
            };

            let label = if dist == 1 {
                format!("{arrow} {dist} line \u{00b7} s to sync")
            } else {
                format!("{arrow} {dist} lines \u{00b7} s to sync")
            };

            let indicator = Line::from(Span::styled(
                label,
                Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
            ));

            let indicator_area = Rect {
                y: if at_top {
                    lyrics_area.y
                } else {
                    lyrics_area.y + lyrics_area.height.saturating_sub(1)
                },
                height: 1,
                ..lyrics_area
            };
            frame.render_widget(Paragraph::new(indicator), indicator_area);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_lrc() {
        let input = "[00:12.34]Hello world\n[00:17.89]Second line";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines.len(), 2);
        assert_eq!(synced.lines[0].timestamp_ms, 12340);
        assert_eq!(synced.lines[0].text, "Hello world");
        assert_eq!(synced.lines[1].timestamp_ms, 17890);
        assert_eq!(synced.lines[1].text, "Second line");
    }

    #[test]
    fn parse_millisecond_precision() {
        let input = "[01:23.456]Three digit frac";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines.len(), 1);
        assert_eq!(synced.lines[0].timestamp_ms, 83456);
    }

    #[test]
    fn parse_multiple_timestamps() {
        let input = "[00:21.10][00:45.10]Chorus";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines.len(), 2);
        assert_eq!(synced.lines[0].timestamp_ms, 21100);
        assert_eq!(synced.lines[0].text, "Chorus");
        assert_eq!(synced.lines[1].timestamp_ms, 45100);
        assert_eq!(synced.lines[1].text, "Chorus");
    }

    #[test]
    fn parse_offset_tag() {
        let input = "[offset:500]\n[00:10.00]Line one\n[00:20.00]Line two";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines[0].timestamp_ms, 9500); // 10000 - 500
        assert_eq!(synced.lines[1].timestamp_ms, 19500);
    }

    #[test]
    fn parse_negative_offset() {
        let input = "[offset:-200]\n[00:05.00]Early line";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines[0].timestamp_ms, 5200); // 5000 + 200
    }

    #[test]
    fn parse_empty_text_line() {
        let input = "[01:30.00]\n[01:35.00]After gap";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines.len(), 2);
        assert_eq!(synced.lines[0].text, "");
        assert_eq!(synced.lines[1].text, "After gap");
    }

    #[test]
    fn parse_skips_metadata_tags() {
        let input = "[ar:Artist Name]\n[ti:Track Title]\n[00:05.00]Actual lyric";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines.len(), 1);
        assert_eq!(synced.lines[0].text, "Actual lyric");
    }

    #[test]
    fn parse_empty_input() {
        let synced = parse_lrc("");
        assert!(synced.lines.is_empty());
    }

    #[test]
    fn parse_garbage_input() {
        let synced = parse_lrc("not a valid lrc file\nrandom text");
        assert!(synced.lines.is_empty());
    }

    #[test]
    fn active_line_before_first() {
        let synced = SyncedLyrics {
            lines: vec![
                LyricLine {
                    timestamp_ms: 5000,
                    text: "First".into(),
                },
                LyricLine {
                    timestamp_ms: 10000,
                    text: "Second".into(),
                },
            ],
        };
        assert_eq!(synced.active_line_index(0), None);
        assert_eq!(synced.active_line_index(4999), None);
    }

    #[test]
    fn active_line_at_and_between() {
        let synced = SyncedLyrics {
            lines: vec![
                LyricLine {
                    timestamp_ms: 5000,
                    text: "First".into(),
                },
                LyricLine {
                    timestamp_ms: 10000,
                    text: "Second".into(),
                },
                LyricLine {
                    timestamp_ms: 15000,
                    text: "Third".into(),
                },
            ],
        };
        assert_eq!(synced.active_line_index(5000), Some(0));
        assert_eq!(synced.active_line_index(7500), Some(0));
        assert_eq!(synced.active_line_index(10000), Some(1));
        assert_eq!(synced.active_line_index(99999), Some(2));
    }

    #[test]
    fn lines_sorted_after_parse() {
        let input = "[00:20.00]Second\n[00:10.00]First\n[00:30.00]Third";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines[0].text, "First");
        assert_eq!(synced.lines[1].text, "Second");
        assert_eq!(synced.lines[2].text, "Third");
    }

    #[test]
    fn offset_clamps_to_zero() {
        let input = "[offset:99999]\n[00:01.00]Line";
        let synced = parse_lrc(input);
        assert_eq!(synced.lines[0].timestamp_ms, 0);
    }

    #[test]
    fn auto_scroll_clamps_at_top() {
        assert_eq!(viewport_start(3, 40, 100, false), 0);
        assert_eq!(viewport_start(0, 40, 100, false), 0);
        assert_eq!(viewport_start(19, 40, 100, false), 0);
    }

    #[test]
    fn auto_scroll_scrolls_past_anchor() {
        assert_eq!(viewport_start(20, 40, 100, false), 0);
        assert_eq!(viewport_start(21, 40, 100, false), 1);
        assert_eq!(viewport_start(30, 40, 100, false), 10);
    }

    #[test]
    fn manual_scroll_clamps_at_top() {
        // Manual scroll no longer goes negative — clamped to 0
        assert_eq!(viewport_start(3, 40, 100, true), 0);
        assert_eq!(viewport_start(0, 40, 100, true), 0);
    }

    #[test]
    fn manual_scroll_shifts_by_one() {
        // Use center values past the anchor so clamping doesn't flatten them
        let a = viewport_start(25, 40, 100, true);
        let b = viewport_start(26, 40, 100, true);
        assert_eq!(b - a, 1);

        let c = viewport_start(45, 40, 100, true);
        let d = viewport_start(46, 40, 100, true);
        assert_eq!(d - c, 1);
    }

    #[test]
    fn manual_and_auto_agree_past_anchor() {
        assert_eq!(
            viewport_start(30, 40, 100, true),
            viewport_start(30, 40, 100, false)
        );
    }

    #[test]
    fn scroll_clamps_at_bottom() {
        // With 50 lines and window 40, max start is 10
        assert_eq!(viewport_start(45, 40, 50, true), 10);
        assert_eq!(viewport_start(49, 40, 50, false), 10);
    }

    #[test]
    fn small_window_manual_scroll() {
        // With 30 lines and window 10, center 7 → raw 2, clamped to 2
        assert_eq!(viewport_start(7, 10, 30, true), 2);
        assert_eq!(viewport_start(8, 10, 30, true), 3);
    }
}
