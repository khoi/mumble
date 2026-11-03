//! Read-only, release-available transcript commands. No app services or migrations.

use std::io::{self, Read, Write};
use std::path::Path;

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};

use crate::dictation::vocabulary::VocabularyEntry;
use crate::history::search::{TextHit, search_texts};
use crate::history::stats::HistoryStats;
use crate::history::time::TimeRange;
use crate::history::{HistoryEntry, HistoryKind, HistoryRetention};

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Lower time bound: 3d, today, YYYY-MM-DD, or RFC 3339.
    #[arg(long)]
    since: Option<String>,
    /// Exclusive upper bound; a date includes that entire local day.
    #[arg(long)]
    until: Option<String>,
    /// Exact application name, ignoring case.
    #[arg(long)]
    app: Option<String>,
    /// Case-insensitive substring in final or raw text.
    #[arg(long)]
    grep: Option<String>,
    /// Rank by relevance with stemmed, fuzzy BM25.
    #[arg(long, conflicts_with = "oldest_first")]
    search: Option<String>,
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=10_000))]
    limit: u32,
    /// Include the stored pre-processing transcript.
    #[arg(long)]
    raw: bool,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    oldest_first: bool,
    #[command(subcommand)]
    pub action: Option<HistoryAction>,
}

/// History changes, carried out by the running Hex (`crate::control`).
#[derive(Subcommand, Debug)]
pub enum HistoryAction {
    /// Delete every retained dictation.
    Clear {
        /// Confirm deleting the history.
        #[arg(long)]
        yes: bool,
        #[command(flatten)]
        output: crate::control::cli::Output,
    },
    /// Sum up the retained dictations: words, speaking speed, top apps,
    /// how often Output changed the text, and the daily streak (Labs ›
    /// Dictation stats). Read-only.
    Stats {
        /// Lower time bound: 3d, today, YYYY-MM-DD, or RFC 3339.
        #[arg(long)]
        since: Option<String>,
        /// Exclusive upper bound; a date includes that entire local day.
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Serialize)]
struct HistoryRow<'a> {
    id: u64,
    timestamp_ms: u64,
    application: Option<&'a str>,
    mode: Option<&'a str>,
    kind: HistoryKind,
    final_text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    snippet: Option<String>,
}

#[derive(Serialize)]
struct HistoryOutput<'a> {
    version: u32,
    retention: HistoryRetention,
    entries: Vec<HistoryRow<'a>>,
    truncated: bool,
}

fn history_output<'a>(
    entries: &'a [HistoryEntry],
    args: &HistoryArgs,
    retention: HistoryRetention,
    now: u64,
    vocabulary: &[VocabularyEntry],
) -> Result<HistoryOutput<'a>> {
    let range = TimeRange::parse(args.since.as_deref(), args.until.as_deref(), now)?;
    let cutoff = retention.max_age_ms().map(|age| now.saturating_sub(age));
    let app = args.app.as_ref().map(|s| s.to_lowercase());
    let grep = args.grep.as_ref().map(|s| s.to_lowercase());
    let mut matching: Vec<_> = entries
        .iter()
        .filter(|entry| cutoff.is_none_or(|cutoff| entry.timestamp_ms >= cutoff))
        .filter(|entry| range.contains(entry.timestamp_ms))
        .filter(|entry| {
            app.as_ref().is_none_or(|app| {
                entry
                    .application
                    .as_ref()
                    .is_some_and(|name| name.to_lowercase() == *app)
            })
        })
        .filter(|entry| {
            grep.as_ref().is_none_or(|grep| {
                entry.final_text.to_lowercase().contains(grep)
                    || entry.raw_text.to_lowercase().contains(grep)
            })
        })
        .collect();
    matching.sort_by_key(|entry| (entry.timestamp_ms, entry.id));
    if !args.oldest_first {
        matching.reverse();
    }
    let ranked: Vec<(&HistoryEntry, Option<TextHit>)> = match &args.search {
        Some(query) => rank_history(&matching, query, vocabulary)?
            .into_iter()
            .map(|(entry, hit)| (entry, Some(hit)))
            .collect(),
        None => matching.into_iter().map(|entry| (entry, None)).collect(),
    };
    let truncated = ranked.len() > args.limit as usize;
    let entries = ranked
        .into_iter()
        .take(args.limit as usize)
        .map(|(entry, hit)| HistoryRow {
            id: entry.id,
            timestamp_ms: entry.timestamp_ms,
            application: entry.application.as_deref(),
            mode: entry.processing.as_ref().map(|p| p.profile.as_str()),
            kind: entry.kind,
            final_text: &entry.final_text,
            raw_text: args.raw.then_some(entry.raw_text.as_str()),
            score: hit.as_ref().map(|hit| (hit.score * 100.0).round() / 100.0),
            snippet: hit.map(|hit| hit.snippet),
        })
        .collect();
    Ok(HistoryOutput {
        version: 1,
        retention,
        entries,
        truncated,
    })
}

/// Rank newest-first entries by their final text and, when it differs, their
/// raw transcript, as one document per entry with the better text's match.
/// Ties stay newest first.
fn rank_history<'a>(
    entries: &[&'a HistoryEntry],
    query: &str,
    vocabulary: &[VocabularyEntry],
) -> Result<Vec<(&'a HistoryEntry, TextHit)>> {
    let documents = entries
        .iter()
        .map(|entry| {
            let mut texts = vec![entry.final_text.as_str()];
            if !entry.raw_text.is_empty() && entry.raw_text != entry.final_text {
                texts.push(entry.raw_text.as_str());
            }
            texts
        })
        .collect::<Vec<_>>();
    Ok(search_texts(&documents, query, vocabulary)?
        .into_iter()
        .map(|hit| (entries[hit.index], hit))
        .collect())
}

/// Read only this setting, without loading AppSettings (which migrates and writes).
fn retention(root: &Path) -> Result<HistoryRetention> {
    #[derive(Default, Deserialize)]
    struct Settings {
        #[serde(default)]
        history_retention: HistoryRetention,
    }
    let file = match std::fs::File::open(root.join("settings.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(HistoryRetention::default());
        }
        Err(error) => return Err(error).wrap_err("could not read history retention"),
    };
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(eyre!("settings file exceeds the read limit"));
    }
    let settings: Settings = serde_json::from_slice(&bytes)
        .wrap_err("could not read history retention; settings.json is invalid")?;
    Ok(settings.history_retention)
}

#[derive(Serialize)]
struct StatsOutput {
    version: u32,
    retention: HistoryRetention,
    #[serde(flatten)]
    stats: HistoryStats,
}

/// The retained entries in the range, summed up.
fn stats_output(
    entries: &[HistoryEntry],
    since: Option<&str>,
    until: Option<&str>,
    retention: HistoryRetention,
    now: u64,
    day_of: impl Fn(u64) -> i64,
) -> Result<StatsOutput> {
    let range = TimeRange::parse(since, until, now)?;
    let cutoff = retention.max_age_ms().map(|age| now.saturating_sub(age));
    let stats = crate::history::stats::summarize(
        entries
            .iter()
            .filter(|entry| cutoff.is_none_or(|cutoff| entry.timestamp_ms >= cutoff))
            .filter(|entry| range.contains(entry.timestamp_ms)),
        day_of(now),
        day_of,
    );
    Ok(StatsOutput {
        version: 1,
        retention,
        stats,
    })
}

/// Dictation stats are a Labs feature, refused before History is read
/// while its toggle is off.
fn require_history_stats(on: bool) -> Result<()> {
    if on {
        Ok(())
    } else {
        Err(eyre!(
            "Dictation stats are a Labs feature and are off. Turn them on with `hex labs on history-stats` or in Settings › Labs."
        ))
    }
}

/// `hex history stats`.
fn history_stats(since: Option<&str>, until: Option<&str>, json: bool) -> Result<()> {
    require_history_stats(crate::labs::history_stats())?;
    let root = crate::app_paths::support_dir()?;
    let entries = crate::history::read_snapshot(&root.join("history.json"))
        .wrap_err("could not read history.json")?;
    let output = stats_output(
        &entries,
        since,
        until,
        retention(&root)?,
        crate::desktop::events::now_ms(),
        crate::history::stats::local_day,
    )?;
    let mut out = io::stdout().lock();
    if json {
        return write_json(&mut out, &output);
    }
    write!(out, "{}", stats_text(&output.stats))?;
    Ok(())
}

fn stats_text(stats: &HistoryStats) -> String {
    if stats.dictations == 0 {
        return "No retained dictations.\n".into();
    }
    let mut text = format!("Dictations: {}\nWords: {}\n", stats.dictations, stats.words);
    match stats.words_per_minute {
        Some(wpm) => text.push_str(&format!("Words per minute: {wpm}\n")),
        None => text.push_str("Words per minute: unknown (no recorded audio)\n"),
    }
    text.push_str(&format!(
        "Changed by Output: {} ({}%)\n",
        stats.changed_by_output,
        stats.changed_percent().unwrap_or(0)
    ));
    text.push_str(&format!(
        "Daily streak: {} (active days: {})\n",
        stats.streak_days, stats.active_days
    ));
    if !stats.top_apps.is_empty() {
        let apps: Vec<String> = stats
            .top_apps
            .iter()
            .map(|app| format!("{} {}", app.application, app.dictations))
            .collect();
        text.push_str(&format!("Top apps: {}\n", apps.join(", ")));
    }
    text
}

pub fn history(args: &HistoryArgs) -> Result<()> {
    if let Some(HistoryAction::Stats { since, until, json }) = &args.action {
        return history_stats(since.as_deref(), until.as_deref(), *json);
    }
    let root = crate::app_paths::support_dir()?;
    let entries = crate::history::read_snapshot(&root.join("history.json"))
        .wrap_err("could not read history.json")?;
    let output = history_output(
        &entries,
        args,
        retention(&root)?,
        crate::desktop::events::now_ms(),
        &crate::dictation::vocabulary::read_saved(&root),
    )?;
    let mut out = io::stdout().lock();
    if args.json {
        return write_json(&mut out, &output);
    }
    for row in &output.entries {
        write!(
            out,
            "{} · {}",
            crate::history::time::iso_local(row.timestamp_ms),
            row.application.unwrap_or("Unknown app")
        )?;
        if let Some(mode) = row.mode {
            write!(out, " · {mode}")?;
        }
        if let Some(score) = row.score {
            write!(out, " · score {score:.1}")?;
        }
        writeln!(out, "\n{}", row.final_text)?;
        if let Some(raw) = row.raw_text {
            writeln!(out, "Raw: {raw}")?;
        }
    }
    if output.entries.is_empty() {
        writeln!(out, "No history matches.")?;
    }
    if output.truncated {
        writeln!(out, "More matches; raise --limit or narrow the time range.")?;
    }
    Ok(())
}

fn write_json(out: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out)?;
    Ok(())
}

#[cfg(test)]
#[path = "transcript_cli_tests.rs"]
mod tests;
