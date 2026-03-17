use anyhow::{bail, Result};
use console::style;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::OnceLock;
use std::time::Duration;

static IS_INTERACTIVE: OnceLock<bool> = OnceLock::new();

pub fn is_interactive() -> bool {
    *IS_INTERACTIVE.get_or_init(|| console::Term::stderr().is_term())
}

pub fn with_spinner<T>(message: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if !is_interactive() {
        return f();
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.dim} {msg}")
            .expect("invalid spinner template"),
    );
    pb.set_message(message.to_owned());
    pb.enable_steady_tick(Duration::from_millis(80));

    let result = f();
    pb.finish_and_clear();
    result
}

pub fn styled_song(title: &str, artist: &str) -> String {
    if is_interactive() {
        format!("{} — {}", style(title).bold(), style(artist).dim())
    } else {
        format!("{title} — {artist}")
    }
}

pub fn select(prompt: &str, items: &[String]) -> Result<Option<usize>> {
    if !is_interactive() {
        return Ok(None);
    }

    let selection = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact_opt()?;

    Ok(selection)
}

pub struct PickCandidate {
    pub name: String,
    pub label: String,
    pub popularity: Option<u32>,
}

/// Auto-pick or show interactive picker. Returns the index of the selected candidate.
pub fn pick_result(
    query: &str,
    candidates: Vec<PickCandidate>,
    prompt: &str,
    force_pick: bool,
) -> Result<usize> {
    if candidates.is_empty() {
        bail!("no results for \"{query}\"");
    }

    if candidates.len() == 1 {
        return Ok(0);
    }

    if force_pick {
        return show_picker(query, &candidates, prompt);
    }

    let query_lower = query.trim().to_lowercase();
    let lower_names: Vec<String> = candidates.iter().map(|c| c.name.to_lowercase()).collect();

    // Exact match on name
    let exact: Vec<usize> = lower_names
        .iter()
        .enumerate()
        .filter(|(_, name)| *name == &query_lower)
        .map(|(i, _)| i)
        .collect();

    if !exact.is_empty() {
        return Ok(most_popular(&candidates, &exact));
    }

    // Suffix-variant match (query length >= 3)
    if query_lower.len() >= 3 {
        let prefix_matches: Vec<usize> = lower_names
            .iter()
            .enumerate()
            .filter(|(_, name)| name.starts_with(&query_lower))
            .map(|(i, _)| i)
            .collect();

        if !prefix_matches.is_empty() {
            let all_suffix_variants = prefix_matches.iter().all(|&i| {
                let rest = &lower_names[i][query_lower.len()..];
                rest.is_empty() || rest.starts_with(" -") || rest.starts_with(" (")
            });

            if all_suffix_variants {
                return Ok(most_popular(&candidates, &prefix_matches));
            }
        }
    }

    show_picker(query, &candidates, prompt)
}

fn most_popular(candidates: &[PickCandidate], indices: &[usize]) -> usize {
    *indices
        .iter()
        .max_by_key(|&&i| candidates[i].popularity.unwrap_or(0))
        .unwrap_or(&0)
}

fn show_picker(query: &str, candidates: &[PickCandidate], prompt: &str) -> Result<usize> {
    let matcher = SkimMatcherV2::default();
    let mut scored: Vec<(usize, i64)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let score = matcher.fuzzy_match(&c.label, query).unwrap_or(0);
            (i, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1));

    let sorted_labels: Vec<String> = scored
        .iter()
        .map(|&(i, _)| candidates[i].label.clone())
        .collect();
    match select(prompt, &sorted_labels)? {
        Some(idx) => Ok(scored[idx].0),
        None => bail!("cancelled"),
    }
}

pub fn format_duration(total_secs: i64) -> String {
    let total_secs = total_secs.max(0);
    format!("{}:{:02}", total_secs / 60, total_secs % 60)
}

pub fn progress_bar(progress_secs: i64, total_secs: i64) -> String {
    let left = format_duration(progress_secs);
    let right = format_duration(total_secs);

    if !is_interactive() {
        return format!("[{left} / {right}]");
    }

    let term_width = console::Term::stderr().size().1 as usize;
    let label_width = left.len() + right.len() + 3;
    let bar_width = term_width.saturating_sub(label_width).clamp(10, 50);

    let ratio = if total_secs > 0 {
        (progress_secs as f64 / total_secs as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let filled = (bar_width as f64 * ratio).round() as usize;
    let empty = bar_width - filled;

    let filled_str: String = "━".repeat(filled);
    let empty_str: String = "─".repeat(empty);

    format!("{left} {filled_str}{} {right}", style(empty_str).dim())
}

/// Parse a volume string into an absolute level, given the current volume.
/// Handles absolute ("50"), relative ("+10", "-10"), and clamping.
pub fn parse_volume(input: &str, current: u32) -> Result<u8> {
    let input = input.trim();

    if input.starts_with('+') || input.starts_with('-') {
        let delta: i32 = input
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid volume adjustment: {input}"))?;
        return Ok((current as i32 + delta).clamp(0, 100) as u8);
    }

    let level: u32 = input
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid volume level: {input}"))?;
    if level > 100 {
        bail!("volume must be 0-100, got {level}");
    }
    Ok(level as u8)
}

pub fn open_browser(url: &str) -> Result<bool> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Ok(false);

    match std::process::Command::new(cmd)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_basic() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(59), "0:59");
        assert_eq!(format_duration(60), "1:00");
        assert_eq!(format_duration(125), "2:05");
        assert_eq!(format_duration(3661), "61:01");
    }

    #[test]
    fn format_duration_negative_clamps_to_zero() {
        assert_eq!(format_duration(-5), "0:00");
    }

    #[test]
    fn parse_volume_absolute() {
        assert_eq!(parse_volume("50", 0).unwrap(), 50);
        assert_eq!(parse_volume("0", 80).unwrap(), 0);
        assert_eq!(parse_volume("100", 0).unwrap(), 100);
    }

    #[test]
    fn parse_volume_rejects_over_100() {
        assert!(parse_volume("101", 0).is_err());
        assert!(parse_volume("200", 0).is_err());
    }

    #[test]
    fn parse_volume_relative() {
        assert_eq!(parse_volume("+10", 50).unwrap(), 60);
        assert_eq!(parse_volume("-10", 50).unwrap(), 40);
    }

    #[test]
    fn parse_volume_clamps() {
        assert_eq!(parse_volume("+20", 90).unwrap(), 100);
        assert_eq!(parse_volume("-20", 10).unwrap(), 0);
    }

    #[test]
    fn parse_volume_invalid() {
        assert!(parse_volume("abc", 0).is_err());
        assert!(parse_volume("+abc", 0).is_err());
    }

    #[test]
    fn pick_result_empty_candidates() {
        let result = pick_result("test", vec![], "Pick", false);
        assert!(result.is_err());
    }

    #[test]
    fn pick_result_single_candidate() {
        let candidates = vec![PickCandidate {
            name: "Song".to_string(),
            label: "Song — Artist".to_string(),
            popularity: Some(50),
        }];
        assert_eq!(pick_result("song", candidates, "Pick", false).unwrap(), 0);
    }

    #[test]
    fn pick_result_exact_match() {
        let candidates = vec![
            PickCandidate {
                name: "Creep".to_string(),
                label: "Creep — Radiohead".to_string(),
                popularity: Some(80),
            },
            PickCandidate {
                name: "Creepy".to_string(),
                label: "Creepy — Other".to_string(),
                popularity: Some(20),
            },
        ];
        assert_eq!(pick_result("creep", candidates, "Pick", false).unwrap(), 0);
    }

    #[test]
    fn pick_result_exact_match_picks_most_popular() {
        let candidates = vec![
            PickCandidate {
                name: "Starboy".to_string(),
                label: "Starboy — The Weeknd".to_string(),
                popularity: Some(40),
            },
            PickCandidate {
                name: "Starboy".to_string(),
                label: "Starboy — The Weeknd (Deluxe)".to_string(),
                popularity: Some(90),
            },
        ];
        assert_eq!(
            pick_result("starboy", candidates, "Pick", false).unwrap(),
            1
        );
    }
}
