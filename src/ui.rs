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

const AUTO_PICK_SCORE_RATIO: f64 = 1.5;

/// Fuzzy-rank `labels` against `query`, then auto-pick or show interactive picker.
/// Returns the original index of the selected item.
pub fn pick_result(query: &str, labels: Vec<String>, prompt: &str) -> Result<usize> {
    if labels.is_empty() {
        bail!("no results for \"{query}\"");
    }

    let matcher = SkimMatcherV2::default();
    let mut scored: Vec<(usize, &str, i64)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let score = matcher.fuzzy_match(label, query).unwrap_or(0);
            (i, label.as_str(), score)
        })
        .collect();

    scored.sort_by(|a, b| b.2.cmp(&a.2));

    if scored.len() == 1 {
        return Ok(scored[0].0);
    }

    let top_score = scored[0].2;
    let second_score = scored[1].2;

    if second_score == 0 || (top_score as f64 / second_score as f64) > AUTO_PICK_SCORE_RATIO {
        return Ok(scored[0].0);
    }

    let sorted_labels: Vec<String> = scored.iter().map(|(_, l, _)| l.to_string()).collect();
    match select(prompt, &sorted_labels)? {
        Some(idx) => Ok(scored[idx].0),
        None => Ok(scored[0].0),
    }
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
