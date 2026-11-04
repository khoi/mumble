# Read transcripts from the command line

The macOS release binary can read retained dictations while Mumble runs. Use
`/Applications/Mumble.app/Contents/MacOS/mumble` if `mumble` is not on your PATH.

```sh
mumble history --since 3d --app 'OpenCode Beta' --grep problems --json
mumble history --since 2026-10-20 --until 2026-10-21 --raw --oldest-first --limit 500
mumble history --since 7d --search 'problems board' --json
mumble history stats --since 7d --json   # Labs › Dictation stats
```

## History filters apply before the limit

`history` returns successful retained results, including Send and Voice Action.
Each result has a timestamp, application, recorded processing mode if present,
and final text. `--raw` adds the stored pre-processing transcript. Older entries
may have no app or mode; Mumble does not infer missing metadata.

- `--since` is inclusive and `--until` is exclusive. A date alone means local
  midnight; `--until 2026-10-20` includes all of October 20.
- Times accept `3d`, `36h`, `90m`, `2w`, `today`, `yesterday`, `now`, dates,
  local date-times, RFC 3339, or Unix milliseconds (10-digit values are seconds).
  Relative times count backwards from the query time.
- `--app` matches the entire application name, ignoring case.
- `--grep` matches a case-insensitive literal substring in raw or final text,
  including raw text when `--raw` is absent. It is not a regular expression.
- `--search QUERY` ranks the remaining entries with History's own stemmed,
  fuzzy BM25 over final and raw text (see [Search ranking](#search-ranking)), so `bord` finds `board` and
  `scheduling` finds `schedule`. Quoted phrases are required, exact (stemmed)
  matches. Unmatched entries are dropped. It combines with the other filters and
  cannot be used with `--oldest-first`. Statistics cover only the filtered,
  unexpired entries, so scores compare within one response, not across queries.
- Results sort by timestamp and then ID, newest first; with `--search`, by score
  and then newest first. `--oldest-first` reverses the time order. `--limit`
  defaults to 50 and accepts 1–10,000.
- The saved retention window filters expired entries even before the app's next
  cleanup. Missing settings use seven days. Invalid settings fail rather than
  silently widening the retention window. Off preserves existing entries and
  prevents new ones, matching the History pane.

## History stats sum up what is retained

`history stats` (Labs › Dictation stats, `mumble labs on history-stats`, off by
default) prints a summary of the retained entries: dictations, words (the
inserted text split on whitespace), words per minute (words over recorded
audio minutes, counting only entries with audio; unknown without any), how
many entries Output changed (inserted text differs from the transcript, ignoring
surrounding whitespace), the daily streak (consecutive local days with an entry,
ending today or, with none yet today, yesterday), active days, and the five
most used applications. With the toggle off it fails, naming the command that
turns it on, before History is read. `--since` and `--until` narrow it with the
same time syntax; the retention window always applies. It counts only what
History still holds: nothing is kept after retention removes an entry, and
which replacements fired is not tracked.

## Search ranking

Exact stems clearly outrank fuzzy matches. Entries matching every unquoted
term exactly come first, whatever their scores. Within each tier a prefix
counts half as much as an exact stem, one or two edits 0.35, and a Soundex
sound-alike 0.2, also for the share of terms an entry covers. Spelling
fallbacks (edits and sound-alikes) are tried only for a term found exactly in
fewer than three entries, and never reach a word in more than 0.5% of the
entries searched (and at least five), except that a term with no exact match
still reaches a common word one edit away (never two). So `blob` does not pull
in `blog`, `Bob`, or `believe` once three entries say `blob`, and `media` does
not pull in a word as common as `made`. When several words in an entry could
stand for one term, the exact stem wins, then the higher score, the more
trusted match, and the alphabetically first word, so results never vary between
runs. An entry's raw and final text count as one document (so a word in both
counts once), and its better text is the hit.

With the `vocabulary` experiment on, `--search` also reads the saved
[vocabulary](features/vocabulary.md) from `settings.json` without loading or
migrating it. A query word that is a one-word vocabulary term also finds its
"sounds like" spellings and the term split into two adjacent words (0.5, like a
prefix), and words that sound like it (0.3) unless they are common. Neither
ever counts as an exact match.

## JSON version 1

Every JSON response has integer `version: 1`. Consumers should ignore unknown
fields; incompatible field changes require a new version. All `*_ms` fields are
integer milliseconds. Absolute timestamps use Unix time. Nullable fields are
present as `null`, except `raw_text`, which is omitted without `--raw`, and
`score` and `snippet`, which are omitted without `--search`.
JSON goes to stdout; errors go to stderr with a nonzero exit status. Empty
results are successful responses. `truncated` reports the query's output limit,
not data already expired or evicted from the store.

```ts
type History = {
  version: 1
  retention: "off" | "day" | "week" | "month" | "forever"
  truncated: boolean
  entries: {
    id: number
    timestamp_ms: number
    application: string | null
    mode: string | null
    kind: "dictation" | "send" | "voice_action" | "printed" | "raw"
    final_text: string
    raw_text?: string // present only with --raw
    score?: number // present only with --search; higher is more relevant
    snippet?: string // present only with --search; matches wrapped in **
  }[]
}

type HistoryStats = { // mumble history stats --json
  version: 1
  retention: "off" | "day" | "week" | "month" | "forever"
  dictations: number
  words: number
  audio_ms: number
  words_per_minute: number | null // null without recorded audio
  changed_by_output: number
  streak_days: number
  active_days: number
  top_apps: { application: string; dictations: number }[] // at most 5, most first
}
```

## Reads never start or change the app

These commands dispatch before logging, instance locking,
local API startup, permissions, GUI, and Mumble Microphone. They open no microphone,
load no model, make no network request, and create no files or directories.
History reads one file descriptor, so an atomic replacement yields a complete
old or new snapshot. Malformed history produces an error without recovery writes.

Data comes from `~/Library/Application Support/mumble`. `MUMBLE_APPLICATION_SUPPORT_DIR` overrides the
root for fixtures or a mirrored copy. These CLI commands currently target macOS;
Linux and Windows retain their existing command surfaces.

## Retention stays bounded in one store

The history cap is now 10,000 entries or 16 MiB of raw plus final text, whichever
is reached first, up from 2,000 / 4 MiB. Per-entry limits, owner-only files,
atomic replacement, expiry, explicit deletion, and Clear all remain shared with
the History pane. Older entries already evicted cannot be recovered by this change.

Keeping one store avoids a second set of expiry and deletion rules. A monthly
archive would need atomic rollover, ID deduplication, expiry while idle, and
deletion across every file. The larger cap is a bounded increment using the
existing store; it still rewrites the JSON file on each successful result. The
cap is not a guarantee to retain every result within a chosen window, and
Forever is also hard-capped. No separate archive is created.
