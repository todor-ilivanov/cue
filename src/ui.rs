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
