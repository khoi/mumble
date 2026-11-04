---
name: mumble-personal-commands
description: Configure Mumble personal deterministic transcript transformations in `~/.config/mumble/mumble.config.ts`. Use when the user wants to add or change custom text transformations for the Mumble voice app.
license: MIT
---

# Mumble Personal Transformations

This skill targets current Mumble (dev builds now, and the next stable release).
Hex 2.3.1 uses an older runtime; its matching skill installs with
`npx skills add anomalyco/hex#app-v2.3.1 --skill hex-personal-commands`.

Mumble runs `~/.config/mumble/mumble.config.ts` itself, in an embedded JavaScript
engine. There is no package to install and no build step.

1. Check for `~/.config/mumble/mumble.config.ts` (or `mumble.config.js`). If it is
   missing, ask the user to open Mumble's Modes pane and choose Create Config,
   or run `mumble commands init` if the `mumble` CLI is available.
2. Read `~/.config/mumble/AGENTS.md` for the current API and limits, and
   `mumble.d.ts` for exact types. Mumble owns both and rewrites them; do not edit
   them.
3. Edit only the config and files it imports. Scripts are plain TypeScript or
   JavaScript: they can import `@mumble/commands` and files in the folder, not
   npm packages, Effect, or Node APIs, and they have no filesystem, network,
   or timer access. Enums, namespaces, and constructor parameter properties are
   rejected.
4. Use `defineMumbleConfig` from `@mumble/commands` and declare pure `transformations`
   entries (`{ id, name, description?, transform(text, context) }`).
5. Transformations must be selected in the mode's Transformations section.
   They run after mode corrections and optional AI rewriting.
6. Run `mumble commands check` (macOS) after editing. It loads the config as Mumble
   does and prints what it registers or the error with file, line, and column.
   Optionally type-check with `npx -y -p typescript tsc` in `~/.config/mumble`. A
   running Mumble reloads the config on save and shows load errors in Modes'
   Transformations section.
   If `mumble` reports `commands check` as an unrecognized subcommand, stop: the
   user is on an older Hex (such as 2.3.1). Tell them to install the matching
   skill with `npx skills add anomalyco/hex#app-v2.3.1 --skill hex-personal-commands`,
   or to switch to the dev build (https://github.com/supabitapp/mumble#downloads).

Completion criterion: the requested behavior is represented in the config,
`mumble commands check` succeeds (or, without the CLI, the reload succeeded in
Mumble), and any verification gap is reported.
