# Read retained transcripts

```ts
Agent or terminal                                // transcripts.cli; macOS release
└── mumble history -> Timestamp, app, recorded mode, final text
    ├── --since / --until / --app / --grep -> Filter, then limit
    ├── --search <query> -> Rank filtered entries with a stemmed, fuzzy BM25;
    │   exact stems first (raw + final text count as one document)
    ├── --raw -> Include the stored pre-processing transcript
    ├── --oldest-first -> Reverse timestamp order
    ├── --json -> Version 1 response, with truncation flag
    └── stats [--since/--until] [--json] -> Labs › Dictation stats (labs.history_stats):
        dictations, words, words per minute, changed by Output, daily streak,
        top apps over every retained entry; toggle off -> fails naming
        `mumble labs on history-stats` before History is read
```

Commands work beside the running app and independently of feature opt-ins,
except `mumble history stats`, which needs its Labs toggle.
`control/transcript_cli` dispatches before logging, instance locking, local
API, GUI, permissions, and Mumble Microphone. Nothing is created or repaired.

History uses one open-file snapshot and the saved retention, so concurrent atomic
replacement cannot expose a partial write and expiry is enforced before disk
cleanup. Off retains existing entries. Missing settings default to a week;
malformed history/settings fail with an error. The same owner-only store now
holds at most 10,000 entries and 16 MiB of raw + final text, without an archive.
Already-evicted entries remain unavailable. UI search still returns at most 200;
CLI limits apply after filtering and allow up to 10,000.

Sources: [control/transcript_cli.rs](../../src/control/transcript_cli.rs),
[history.rs](../../src/history.rs), History's own search in
[history/search.rs](../../src/history/search.rs), `--since`/`--until` parsing
in [history/time.rs](../../src/history/time.rs), and early dispatch in
[main.rs](../../src/main.rs).
[Command and JSON contracts](../transcript-cli.md) document flags and schema.

Checks in [control/transcript_cli_tests.rs](../../src/control/transcript_cli_tests.rs) cover
filters/order, ranked `--search` within retention, one document per entry
(`history_search_counts_raw_and_final_text_as_one_document`), retention and
malformed settings, JSON snapshots, atomic-replace reads, and parsing. Existing
history checks cover bounded storage, owner-only files, and expiry. These
fixture checks do not establish a signed installed-app invocation; native
install evidence must be recorded separately.

`scripts/test-transcript-cli.py <release-binary>` exercises the real command
parser against copied fixtures while holding the listener lock. It checks that
the commands create no logs or API discovery file and leave all input bytes and
modification times unchanged, with experimental microphone mode requested.

Observed October 22, 2026, on Pro before and after the Rune integration, when
these commands also read meetings: all 35 targeted history, meeting-query, and
CLI tests passed; formatting and all-target/all-feature Clippy with
`-D warnings` passed. The optimized release binary passed the
occupied-lock/no-write smoke. Claude Opus 5.5 reviewed the implementation and
integration independently and found no blockers. Not rerun since meetings were
removed.
