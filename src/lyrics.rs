use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use serde::Deserialize;

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
    if let Some(state) = try_exact_match(title, artist, album, duration_secs) {
        return state;
    }
    if let Some(state) = try_search(title, artist) {
        return state;
    }
    LyricsState::None
}

fn try_exact_match(
    title: &str,
    artist: &str,
    album: &str,
    duration_secs: i64,
) -> Option<LyricsState> {
    let resp = ureq::get(&format!("{API_BASE}/get"))
        .set("User-Agent", USER_AGENT)
        .query("track_name", title)
        .query("artist_name", artist)
        .query("album_name", album)
        .query("duration", &duration_secs.to_string())
        .timeout(std::time::Duration::from_secs(5))
        .call();

    match resp {
        Ok(r) => {
            let body: LrclibResponse = r.into_json().ok()?;
            Some(response_to_state(body))
        }
        Err(ureq::Error::Status(404, _)) => Option::None,
        Err(_) => Option::None,
    }
}

fn try_search(title: &str, artist: &str) -> Option<LyricsState> {
    let resp = ureq::get(&format!("{API_BASE}/search"))
        .set("User-Agent", USER_AGENT)
        .query("track_name", title)
        .query("artist_name", artist)
        .timeout(std::time::Duration::from_secs(5))
        .call()
        .ok()?;

    let results: Vec<LrclibResponse> = resp.into_json().ok()?;

    // Prefer the first result with synced lyrics.
    for r in results {
        if r.synced_lyrics.is_some() {
            return Some(response_to_state(r));
        }
    }
    // Fall back to first result with any lyrics.
    None
}

fn response_to_state(resp: LrclibResponse) -> LyricsState {
    if resp.instrumental {
        return LyricsState::Instrumental;
    }
    if let Some(ref synced) = resp.synced_lyrics {
        let parsed = parse_lrc(synced);
        if !parsed.lines.is_empty() {
            return LyricsState::Synced(parsed);
        }
    }
    if let Some(plain) = resp.plain_lyrics {
        if !plain.trim().is_empty() {
            return LyricsState::Plain(plain);
        }
    }
    LyricsState::None
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

pub fn draw_lyrics(frame: &mut Frame, area: Rect, state: &LyricsState, position_ms: u64) {
    match state {
        LyricsState::Loading => {
            draw_centered_dim(frame, area, "Loading lyrics...");
        }
        LyricsState::None => {
            draw_centered_dim(frame, area, "No lyrics available");
        }
        LyricsState::Instrumental => {
            draw_centered_dim(frame, area, "Instrumental");
        }
        LyricsState::Plain(text) => {
            draw_plain(frame, area, text);
        }
        LyricsState::Synced(synced) => {
            draw_synced(frame, area, synced, position_ms);
        }
    }
}

fn draw_centered_dim(frame: &mut Frame, area: Rect, msg: &str) {
    if area.height == 0 {
        return;
    }
    let y = area.height / 2;
    if y >= area.height {
        return;
    }
    let centered_area = Rect {
        x: area.x,
        y: area.y + y,
        width: area.width,
        height: 1,
    };
    let line = Line::from(Span::styled(
        msg,
        Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    ));
    frame.render_widget(Paragraph::new(line), centered_area);
}

fn draw_plain(frame: &mut Frame, area: Rect, text: &str) {
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
    let paragraph = Paragraph::new(plain_lines).wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(paragraph, text_area);
}

fn draw_synced(frame: &mut Frame, area: Rect, synced: &SyncedLyrics, position_ms: u64) {
    let height = area.height as usize;
    if height == 0 {
        return;
    }

    let active = synced.active_line_index(position_ms);

    // Current line sits at ~1/3 from top.
    let anchor_row = height / 3;

    let active_idx = active.unwrap_or(0);
    let start_idx = if active.is_some() {
        active_idx.saturating_sub(anchor_row)
    } else {
        0
    };

    let mut rendered_lines: Vec<Line> = Vec::with_capacity(height);

    for row in 0..height {
        let line_idx = start_idx + row;
        if line_idx >= synced.lines.len() {
            rendered_lines.push(Line::from(""));
            continue;
        }

        let lyric = &synced.lines[line_idx];
        let text = if lyric.text.is_empty() {
            "..."
        } else {
            &lyric.text
        };

        let style = if active == Some(line_idx) {
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if active.is_some() && line_idx < active.unwrap() {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new().fg(Color::Gray).add_modifier(Modifier::DIM)
        };

        rendered_lines.push(Line::from(Span::styled(text, style)));
    }

    let paragraph = Paragraph::new(rendered_lines);
    frame.render_widget(paragraph, area);
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
}
