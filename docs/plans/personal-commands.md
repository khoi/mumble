# Personal Commands

**Status:** Personal commands, typed capture descriptors, dictation control
phrases, and text transformations are implemented on macOS; Windows and Linux
use the transformations. This document records the shipped contract. Mumble runs
configs in embedded QuickJS ([quickjs.md](quickjs.md)).

## What Users Can Configure

Personal commands live in `~/.config/mumble/mumble.config.ts` (or `mumble.config.js`).
The config defines
ordinary commands, capture descriptors, named text transformations, and optional
dictation control phrases. Rust validates and matches the grammar and retains
wake, sleep, cancellation, and dictation lifecycle behavior.

Spoken commands and dictation control phrases require the persisted Commands
opt-in, which defaults off. Creating a config does not enable it. Selected mode
transformations can run with Commands off; hotkey dictation does not require
voice commands.

```ts
import { defineMumbleConfig, openUrl } from "@mumble/commands"

export default defineMumbleConfig({
  commands: {
    "open-training": {
      phrases: ["open training"],
      group: "Websites",
      description: "Open the training site",
      action: openUrl("https://example.com/training"),
    },
  },
})
```

Command keys are stable IDs. A command may be global or match one exact
application or browser host. Browser-host context currently comes from Brave
Browser. More-specific commands may specialize ordinary global commands;
commands that can coexist at equal specificity may not share a phrase.

Available actions are `openUrl`, `openApplication`, `openPath`, `press`, and
`typeText`. A handler returns an action, a list of actions, or nothing, and may
also queue actions through its `mumble` argument; Mumble validates them all and runs
them in order after the handler returns. Handlers are plain TypeScript or
JavaScript and may import other files in the config folder, but not npm
packages, Effect, or Node APIs.

Commands may use a trailing `{name}` placeholder for normalized text, or a
`captures` schema built with `digit`, `letter`, `choice`, `union`, and trailing
`text`. Schema-bearing commands bind every declared capture exactly once in
every phrase alias and require a `run` handler. Rust matches these commands
only on completed command transcripts, not partial updates.

`openUrl` accepts absolute web URLs and app deep links, for example
`openUrl("slack://channel?team=T_EXAMPLE&id=C_EXAMPLE")`. The URL is passed
unchanged to macOS, which requires an installed handler for its scheme. Use
`openApplication("Slack")` to launch an app by name or path, not to navigate a
deep link. The API spelling is `openUrl`, not `openURL`. File URLs and inline
script/data URLs remain unsupported; use `openPath` for filesystem paths.

## Dictation Transformations

The config may register named string transformations. Select them in a mode's
Transformations section to run them in displayed order after that mode's
corrections and optional OpenCode rewrite. This pipeline applies to Paste and
Send, not Voice Action. If the transformation stage fails, Mumble
keeps the text from before that stage, not partially transformed output.

```ts
import { defineMumbleConfig } from "@mumble/commands"

export default defineMumbleConfig({
  transformations: {
    "trim-whitespace": {
      name: "Trim whitespace",
      description: "Remove leading and trailing whitespace",
      transform: (text) => text.trim(),
    },
  },
  commands: {},
})
```

Do not reuse the built-in transformation IDs `lowercase`, `spongebob-case`, or
`no-trailing-punctuation`; those IDs execute Rust's built-ins rather than a
registered script function.

Each dictation selects one complete mode. The required Global mode applies
unless a more specific application or browser-host mode matches. A mode owns
its corrections, optional AI rewrite, and selected custom transformations;
contextual modes do not inherit hidden processing from Global.

## Trust Boundary

The config runs inside Mumble's process in QuickJS with no filesystem, network,
environment, process, or timer API, and imports only `@mumble/commands` and files
in the config folder. What it can do is limited to the actions it returns,
which run with the user's permissions, so review agent changes before enabling
commands. Each call has a one-second limit and the engine a 64 MiB limit.

Invalid config never replaces the last valid registry. Rust validates schema,
stable IDs, protected phrases, context predicates, and overlaps before
activation. Dictation remains available when the config fails.

## Dictation Control Phrases

The optional `dictation` section replaces the native voice-dictation phrases,
not the native capture lifecycle:

```ts
import { defineMumbleConfig } from "@mumble/commands"

export default defineMumbleConfig({
  dictation: {
    start: ["begin note"],
    stop: ["finish note"],
    send: ["send note"],
    cancel: ["discard note"],
  },
  commands: {},
})
```

`start` matches an utterance prefix while Mumble listens. `stop`, `send`, and
`cancel` match suffixes during active voice dictation. Each list must contain at
least one phrase. Omitting the section uses the native protocol.

## Workspace

Choose Create Config in the Commands pane, Set up in Modes' Transformations, or
run:

```sh
mumble commands init
```

```text
~/.config/mumble/
├── mumble.config.ts   the user's; created only when no config exists
├── mumble.d.ts        types for @mumble/commands; Mumble rewrites it when it changes
├── AGENTS.md       agent notes; Mumble rewrites it when it changes
└── tsconfig.json   created only when missing, for editors and `tsc`
```

Nothing is installed. Folders from the Bun era keep working once their config
avoids Effect and npm packages; their `package.json`, `bun.lock`,
`node_modules`, `.mumble-sdk`, and `.agents` can be deleted.

Mumble reloads the config about a third of a second after any `.ts`, `.js`,
`.mjs`, or `.json` file in the folder changes (not `.d.ts`, dot folders,
`node_modules`, or symlink targets), activates valid changes, and reports the
last load error with its file, line, and column in Modes' Transformations
section. `mumble commands check` (macOS) loads the config as Mumble would and prints
what it registers or why it fails. Type checking is optional:
`npx -y -p typescript tsc` in the folder.

## Runtime Boundary

```text
mumble.config.ts -> type erasure -> QuickJS on one worker thread -> registration
  -> native Rust resolver -> handler call -> actions -> native executor
```

The script engine never runs in the partial-transcript recognition path.
Request queues, actions per call, and status files are bounded. A failed load
preserves native commands and the last valid runtime snapshot.

## Remaining Work

- Add listing, reload, and log inspection to the CLI when the Commands pane
  is insufficient.
