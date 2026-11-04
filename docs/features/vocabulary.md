# Vocabulary

One shared list of names Hex should know (people, projects, products), each
with optional "sounds like" spellings a transcript may show instead. An empty
list (the default) changes nothing. The window edits the list on macOS,
Linux, and Windows; the `hex vocabulary` commands are macOS only for now.

```ts
General › Vocabulary   // vocabulary.settings; after Language
├── Row value -> "None", "1 term", "N terms" -> opens the Vocabulary sheet
├── Sheet description -> what the list does, then "Terms with spaces match
│   only when said in full" (a term is matched whole; "Dax Rad" doesn't help
│   "Dax" said alone, so a name also said alone gets its own row)
├── Sheet rows -> Term | Sounds like (comma-separated) | ×; scroll past 8
├── Empty list -> Opens with one blank row focused; opening never animates rows
├── Add term -> New empty row, focused; the list glides to its end and the row
│   rises 8 pt as it fades in (SETTLE, bounce 0; Reduce Motion: fade only);
│   saved once the term has text
├── Edits -> Debounced drafts, like Modes' corrections -> settings.json `vocabulary`
├── × -> Removes that row at any click speed or focus; rows keep an identity,
│   so a late edit, focus change, or second × for a removed row touches nothing
├── Done, Escape, or a press outside -> Saves, drops blank rows, closes
└── Past the limits -> Keep the term (cut to 64 characters) and the spellings
    that fit; inline danger note    // never drops the term

Cloud keyterms                           // vocabulary.keyterms; transcription::cloud
├── The transcriber (Claude) sends the list as keyterms with every dictation,
│   stream and whole clip, read at the start of each one; it always applies,
│   with no toggle
├── Terms in list order up to 1,024 characters total (the service's cap);
│   printable ASCII only, commas become spaces, deduplicated case-sensitively,
│   with no term-count cap; an empty list sends none
├── Codex takes no keyterms; the list still shapes rewriting and search
└── "Sounds like" spellings are not sent as keyterms; rewriting and search use them

Recognition hints, folded in                // vocabulary.hints
└── Loading settings.json -> `recognition_hints` saved by an older build (the
    selected model's and the recent models', newest first), split on commas,
    semicolons, and line breaks, become terms; names already listed (any case)
    are skipped, and the rewritten file drops the field

hex vocabulary                           // vocabulary.cli
├── list [--json]
├── add <term> [--sounds-like a,b] -> New term, or new spellings merged into it
│   └── Already there, nothing new -> changed: false
├── remove <term> -> Any case
└── Running Hex saves and reloads settings; an open window adopts the change

A mode's own vocabulary                  // vocabulary.modes; DictationMode::vocabulary
├── settings.json `dictation_processing.modes[].vocabulary`, same entries; no
│   window editor yet (Output lists it as the mode's first step; Edit with
│   agent's prompt explains it)
├── Its terms come first, then the shared list's, each term once (any case)
└── OpenCode rewriting -> every OS, the mode chosen for the dictation

OpenCode rewriting (Modes)               // vocabulary.prompt
└── Prompt gains "Vocabulary (spell these exactly when the speaker means them):
    term (may be transcribed as a, b); …" before the foreground application

Search                                   // vocabulary.search
├── hex history --search
├── Query word = a one-word term, fuzzy on
│   ├── Its "sounds like" spellings, or the term as two words -> 0.5 (like a prefix)
│   └── A word that sounds like it (≥2 consonant sounds, ≤ half the letters changed,
│       not a stopword or common word) -> 0.3
└── Aliases never count as exact; exact hits always rank first
```

The list lives in `settings.json` as `vocabulary`, each entry a plain term
(`"Rune"`) or `{term, sounds_like}`; both load, and Hex writes a term without
spellings as a plain string. `hex vocabulary list --json` keeps the object form.
It is written only when non-empty, so older files load unchanged and are not
rewritten. Terms are unique ignoring case; at most 500 terms, 64 characters
each, and 16 spellings per term. The read-only `hex history` reads the field
straight from the file and never loads or migrates settings.

Short names with few consonant sounds (Rune, Dax) would sound like too many
words, so search finds their mis-transcriptions only through "sounds like"
spellings.

Sources: [dictation/vocabulary.rs](../../src/dictation/vocabulary.rs),
[rune_ui/vocabulary.rs](../../src/rune_ui/vocabulary.rs),
[history/search.rs](../../src/history/search.rs) (`vocabulary_aliases`,
`sounds_alike`), [dictation/processor.rs](../../src/dictation/processor.rs)
(`vocabulary_lines`), [transcription/cloud.rs](../../src/transcription/cloud.rs),
and [control.rs](../../src/control.rs) (`edit_vocabulary`).

Checks: `dictation::vocabulary::tests` (dedupe and merge, clamping,
case-insensitive removal, comma splitting, reading the saved list without
settings, folding hints);
`loading_folds_saved_recognition_hints_into_the_vocabulary_once` in
[app_settings.rs](../../src/app_settings.rs);
`vocabulary_round_trips_and_stays_out_of_files_without_one` in
[app_settings.rs](../../src/app_settings.rs);
`a_vocabulary_term_finds_its_mis_transcriptions_below_exact_hits`,
`sounds_like_spellings_and_split_words_alias_a_term`,
`vocabulary_never_pulls_in_common_words`, and
`sound_alikes_need_two_consonant_sounds_and_few_changes` in
[history/search.rs](../../src/history/search.rs);
`prompt_names_vocabulary_terms_and_their_mistranscriptions`;
`vocabulary_edits_report_whether_they_changed_the_list` and the request round
trip in control.rs; `vocabulary_lists_terms_with_their_spellings` in
[control/cli.rs](../../src/control/cli.rs);
`rows_project_to_cleaned_merged_entries`,
`rows_past_the_limits_keep_their_term_and_what_fits`,
`the_sheet_starts_empty_lists_with_a_row_and_saves_on_close`,
`removing_a_focused_row_leaves_the_others_and_late_edits_touch_nothing`,
`removing_the_last_row_then_editing_it_is_harmless`,
`rapid_removals_from_one_frame_remove_only_their_own_rows`, and
`a_reload_retires_the_old_rows` in
[rune_ui/vocabulary.rs](../../src/rune_ui/vocabulary.rs). Scenes:
`mac-settings-vocabulary`, `mac-settings-vocabulary-empty`.
