use super::*;
use clap::Parser;
use std::fs;
use std::path::PathBuf;

const NOW: u64 = 1_792_700_000_000;

/// The shared command tree every OS's `mumble` flattens.
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<ControlCli>,
}

use crate::control::cli::ControlCli;
const FIXTURE: &str = include_str!("../../tests/fixtures/history.json");

fn args(extra: &[&str]) -> HistoryArgs {
    let cli = Cli::try_parse_from(
        ["mumble", "history"]
            .into_iter()
            .chain(extra.iter().copied()),
    )
    .unwrap();
    match cli.command.unwrap() {
        ControlCli::History(args) => args,
        _ => panic!("expected history"),
    }
}

fn temp() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "mumble-transcript-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn entries() -> Vec<HistoryEntry> {
    serde_json::from_value(
        serde_json::from_str::<serde_json::Value>(FIXTURE).unwrap()["entries"].clone(),
    )
    .unwrap()
}

#[test]
fn history_filters_and_order_apply_before_the_limit() {
    let entries = entries();
    let query = args(&[
        "--since",
        "3d",
        "--app",
        "opencode beta",
        "--grep",
        "PROBLEMS",
        "--limit",
        "1",
    ]);
    let output = history_output(&entries, &query, HistoryRetention::Week, NOW, &[]).unwrap();
    assert_eq!(output.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [2]);
    assert!(!output.truncated);
    let output = history_output(
        &entries,
        &args(&["--oldest-first", "--limit", "1"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    assert_eq!(output.entries[0].id, 2);
    assert!(output.truncated);
    let output = history_output(
        &entries,
        &args(&["--grep", "needle"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    assert_eq!(output.entries[0].id, 4);
    assert!(output.entries[0].raw_text.is_none());
    let output = history_output(
        &entries,
        &args(&["--until", "1792700000000"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    assert_eq!(
        output.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [3, 2]
    );
    assert!(
        history_output(
            &entries,
            &args(&["--since", "now", "--until", "1d"]),
            HistoryRetention::Week,
            NOW,
            &[]
        )
        .is_err()
    );
}

#[test]
fn history_stats_parse_sum_retained_entries_and_need_the_labs_toggle() {
    let query = args(&["stats", "--since", "3d", "--json"]);
    assert!(matches!(
        query.action,
        Some(HistoryAction::Stats { ref since, json: true, .. }) if since.as_deref() == Some("3d")
    ));
    const DAY: u64 = 86_400_000;
    let entry = |id: u64, age_days: u64, raw: &str, final_text: &str| HistoryEntry {
        id,
        timestamp_ms: NOW - age_days * DAY,
        kind: HistoryKind::Dictation,
        raw_text: raw.into(),
        final_text: final_text.into(),
        application: Some("Zed".into()),
        processing: None,
        audio_ms: 6_000,
        inference_ms: 0,
        total_ms: 0,
    };
    let entries = [
        entry(1, 10, "expired by retention", "expired by retention"),
        entry(2, 1, "so um yes", "Yes."),
        entry(3, 0, "ten words", "ten words"),
    ];
    let utc = |ms: u64| (ms / DAY) as i64;
    let output = stats_output(&entries, None, None, HistoryRetention::Week, NOW, utc).unwrap();
    assert_eq!(output.stats.dictations, 2);
    assert_eq!(output.stats.words, 3);
    assert_eq!(output.stats.words_per_minute, Some(15));
    assert_eq!(output.stats.changed_by_output, 1);
    assert_eq!(output.stats.streak_days, 2);
    assert_eq!(output.stats.top_apps[0].dictations, 2);
    let json = serde_json::to_value(&output).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["words"], 3);
    let today = stats_output(
        &entries,
        Some("today"),
        None,
        HistoryRetention::Week,
        NOW,
        utc,
    );
    assert!(today.unwrap().stats.dictations <= 1);
    assert!(stats_text(&HistoryStats::default()).starts_with("No retained"));
    assert!(require_history_stats(false).is_err());
    assert!(require_history_stats(true).is_ok());
}

#[test]
fn history_search_ranks_fuzzy_stemmed_matches_within_retention() {
    let mut entries = entries();
    entries.push(HistoryEntry {
        id: 5,
        timestamp_ms: 1_792_650_000_000,
        final_text: "Board games tonight.".into(),
        ..entries[2].clone()
    });
    let output = history_output(
        &entries,
        &args(&["--search", "problem bord"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    // Entry 2 matches both words; 5 only one; 1 has expired; 3 and 4 miss.
    assert_eq!(
        output.entries.iter().map(|e| e.id).collect::<Vec<_>>(),
        [2, 5]
    );
    assert!(output.entries[0].score.unwrap() > output.entries[1].score.unwrap());
    assert_eq!(
        output.entries[0].snippet.as_deref(),
        Some("The **Problems** **board**.")
    );
    // The raw transcript is searched too, and filters still apply first.
    let output = history_output(
        &entries,
        &args(&["--search", "needle", "--app", "opencode beta"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    assert_eq!(output.entries[0].id, 4);
    assert_eq!(
        output.entries[0].snippet.as_deref(),
        Some("raw only **NEEDLE**")
    );
    assert!(
        history_output(
            &entries,
            &args(&["--search", "\"\""]),
            HistoryRetention::Week,
            NOW,
            &[]
        )
        .is_err()
    );
    assert!(Cli::try_parse_from(["mumble", "history", "--search", "x", "--oldest-first"]).is_err());
}

#[test]
fn history_search_counts_raw_and_final_text_as_one_document() {
    let template = entries()[2].clone();
    let entry = |id: u64, final_text: &str, raw_text: &str| HistoryEntry {
        id,
        timestamp_ms: NOW - id * 1_000,
        final_text: final_text.into(),
        raw_text: raw_text.into(),
        ..template.clone()
    };
    // Two entries say `dax`, each in both texts: still fewer than three
    // documents, so the sound-alike `Dacks` is found.
    let entries = [
        entry(1, "Ask Dax about the release.", "ask dax about the release"),
        entry(2, "Dax shipped the fix.", "dax shipped the fix"),
        entry(3, "Dacks reviewed it.", "Dacks reviewed it."),
    ];
    let output = history_output(
        &entries,
        &args(&["--search", "Dax"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    let mut ids = output.entries.iter().map(|e| e.id).collect::<Vec<_>>();
    assert_eq!(ids.pop(), Some(3), "the fuzzy match ranks last");
    ids.sort_unstable();
    assert_eq!(ids, [1, 2]);
}

#[test]
fn history_json_contract_snapshot() {
    let entries = entries();
    let output = history_output(
        &entries,
        &args(&["--grep", "problems", "--raw"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        serde_json::json!({
            "version":1,"retention":"week","truncated":false,"entries":[{
                "id":2,"timestamp_ms":1792540000000_u64,"application":"OpenCode Beta","mode":"Engineering",
                "kind":"dictation","final_text":"The Problems board.","raw_text":"the problems board"
            }]
        })
    );
    let mut entries = entries;
    entries[3].application = None;
    let output = history_output(
        &entries,
        &args(&["--limit", "1"]),
        HistoryRetention::Week,
        NOW,
        &[],
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        serde_json::json!({
            "version":1,"retention":"week","truncated":true,"entries":[{
                "id":4,"timestamp_ms":1792700000000_u64,"application":null,"mode":null,
                "kind":"send","final_text":"Recent result"
            }]
        })
    );
}

#[test]
fn retention_defaults_fails_closed_and_off_preserves_existing_entries() {
    let root = temp();
    assert_eq!(retention(&root).unwrap(), HistoryRetention::Week);
    fs::write(root.join("settings.json"), "{}").unwrap();
    assert_eq!(retention(&root).unwrap(), HistoryRetention::Week);
    fs::write(root.join("settings.json"), r#"{"history_retention":"day"}"#).unwrap();
    let entries = entries();
    let output = history_output(&entries, &args(&[]), retention(&root).unwrap(), NOW, &[]).unwrap();
    assert_eq!(output.entries.iter().map(|e| e.id).collect::<Vec<_>>(), [4]);
    let output = history_output(&entries, &args(&[]), HistoryRetention::Off, NOW, &[]).unwrap();
    assert_eq!(output.entries.len(), 4);
    fs::write(root.join("settings.json"), "{broken").unwrap();
    assert!(retention(&root).is_err());
    assert_eq!(
        fs::read_to_string(root.join("settings.json")).unwrap(),
        "{broken"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn atomic_replace_read_never_observes_partial_history() {
    let root = temp();
    let path = root.join("history.json");
    fs::write(&path, FIXTURE).unwrap();
    let writer_path = path.clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writer_barrier = barrier.clone();
    let writer = std::thread::spawn(move || {
        writer_barrier.wait();
        for index in 0..100 {
            let content = if index % 2 == 0 {
                FIXTURE.to_owned()
            } else {
                FIXTURE.replace("Recent result", "replacement")
            };
            fs::write(writer_path.with_extension("tmp"), content).unwrap();
            fs::rename(writer_path.with_extension("tmp"), &writer_path).unwrap();
        }
    });
    barrier.wait();
    for _ in 0..100 {
        let entries = crate::history::read_snapshot(&path).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(matches!(
            entries[3].final_text.as_str(),
            "Recent result" | "replacement"
        ));
    }
    writer.join().unwrap();
    fs::write(&path, "{broken").unwrap();
    assert!(crate::history::read_snapshot(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");
    assert!(!path.with_extension("json.corrupt").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parses_release_commands_and_rejects_invalid_limits_and_times() {
    assert!(Cli::try_parse_from(["mumble", "history"]).is_ok());
    for command in [
        vec!["mumble", "history", "--limit", "0"],
        vec!["mumble", "history", "--limit", "10001"],
    ] {
        assert!(Cli::try_parse_from(command).is_err());
    }
    for invalid in [
        "2026-02-30",
        "2026-00-01",
        "2026-10-20T23:00:99Z",
        "2026-10-20T23:00+99:00",
        "18446744073709551615d",
    ] {
        assert!(
            crate::history::time::parse_time(invalid, NOW, false).is_err(),
            "{invalid}"
        );
    }
    let start = crate::history::time::parse_time("2026-10-20", NOW, false).unwrap();
    let end = crate::history::time::parse_time("2026-10-20", NOW, true).unwrap();
    let range = TimeRange::parse(Some("2026-10-20"), Some("2026-10-20"), NOW).unwrap();
    assert!(range.contains(start));
    assert!(range.contains(end - 1));
    assert!(!range.contains(end));
    let second = crate::history::time::parse_time("2026-10-20T12:00:00Z", NOW, false).unwrap();
    assert_eq!(
        crate::history::time::parse_time("2026-10-20T12:00:00.123456Z", NOW, false).unwrap(),
        second + 123
    );
}
