# `hex` control CLI

The control commands drive the one running Hex: start or stop a dictation,
change any setting, switch models, open the window, quit, or read its
status. They are meant for agents and scripts as much as people, so every
command takes `--json`, is idempotent, and exits with a stable code.

One command tree (`control::cli`) is compiled on macOS, Linux, and Windows,
and one versioned protocol carries it on every OS through the running Hex's
local API: the owner-only discovery file (`local-api.json`) and the
bearer-authenticated `POST /control` (`local_api`).

On Linux and Windows one shared host (`control::host`) carries out the
commands through each OS's thin adapter (`linux/app`, `windows/rune`); macOS's
host is `macos_rune::control`. A command the running Hex can't do on its OS
answers `unsupported` at runtime (exit 1) instead of being missing from the
CLI; `hex status --json` lists the OS's `capabilities`. Today that is
playground stories (`window story`) outside macOS and the local `model`
tools on macOS.

The read-only `hex history` command reads files rather than the running Hex;
see `docs/transcript-cli.md`.

## Commands

```ts
hex
├── status                         what Hex is doing (see Status below)
├── start                          open Hex if it isn't running (macOS Hex.app, else `hex app --hidden`); wait until it answers
├── quit [--force]                 refused (`busy`) while a dictation records or transcribes, or unconfirmed
├── restart [--force]              quit and reopen (opens Hex when it isn't running)
├── dictation start|stop|toggle
│   ├── stop --print               print the transcript instead of pasting it (below)
│   └── start|toggle --raw         paste the transcript as heard, skipping the Output steps
├── paste-last                     paste the last dictation (Option-Shift-V); refused while recording
├── settings
│   ├── list                       every setting: key, value, kind, choices, writable, about
│   ├── get <key|prefix>           one setting, or every key under a prefix (`labs`)
│   └── set <key> <value>
├── model
│   ├── list                       every model, its languages and size, installed or not
│   ├── use <id> [--language L]    installed models only; never downloads or repairs
│   ├── install [id] [--language L] download, then switch to it (default: the selected model)
│   ├── cancel                     stop a download or load in progress
│   ├── import <path> [--name N]   Labs: check and copy in a GGUF; prints its custom-… id
│   ├── rename <id> <name>         Labs: an import's picker name
│   ├── repair <id>                Labs: rehash Hex's copy; ready again if it still matches
│   ├── remove <id>                Labs: Hex's copy of an import (not the selected model)
│   └── status|devices|check|transcribe <wav>  Linux and Windows, run in this process
├── commands
│   ├── init                       create ~/.config/hex (local, no running Hex needed)
│   └── check                      validate ~/.config/hex/hex.config.ts transformations
├── window
│   ├── show [settings|models|modes|history]
│   ├── hide                       close the window; Hex stays in the menu bar
│   └── story <story> [--variant V] [--play]  the playground (Labs): open a story
├── onboarding reset (alias restart)  replay the first run now; every OS (below)
├── history clear --yes            delete every retained dictation
├── vocabulary                     the shared vocabulary (features/vocabulary.md)
│   ├── list [--json]              every term and its "sounds like" spellings
│   ├── add <term> [--sounds-like a,b]  add a term, or new spellings to an existing one
│   └── remove <term>              any case
├── labs [on|off <feature>]       list Labs features or turn one on/off; edits settings.json (features/labs.md)
│   └── link <feature>             macOS: print hex://labs/<feature>, the link that opens Hex's Labs page at that row
├── check-for-updates              check for an update now
├── launch-at-login [on|off]       report or change Launch at Login (macOS: installed Hex.app only; Windows Run key; Linux XDG autostart)
└── logs [--process] [-n N] [-f]   tail live.ndjson (or process.log); no running Hex needed
```

`dictation start` locks the recording, as a double tap would: `dictation stop`
or the next shortcut press finishes it, and `dictation cancel` discards it.
A CLI start has no press to judge, so its recording counts as
intentional at once: the start tone plays once the microphone is open, and the
recording behavior (mute or pause media, idle-sleep prevention) and the live
stream start as they do after a held shortcut. A `stop` replies `done` only
when the clip went to transcription; a clip with no audio yet (the microphone
was still opening) or a silent one is dropped with `discarded`, and nothing
shows as transcribing.

`dictation stop --print` stops the recording at once, exactly as `stop`
does, so a following `start` isn't blocked; only this command then waits, up
to 60 s, for the transcript and writes it to stdout instead of pasting it. The
text gets the same processing as a paste (global replacements, the mode's
corrections, OpenCode rewriting, and transformations). History keeps it as
`printed`, and Paste Last skips it. No speech (nothing recorded yet, a silent
clip, or an empty transcript) exits 4 with code `no-speech` and prints
nothing; with nothing recording it refuses with `not-recording`. A failure,
or the 60 s timeout prints nothing, exits 1, and never
falls back to pasting. `--print` with any other action is a usage error
(exit 2). The command is the same on Linux and Windows, where `hex dictation`
and `hex paste-last` reach the listener's own queue as its shortcut would.

`dictation start --raw` and `toggle --raw` start a dictation that pastes the
transcript as heard, skipping the Output pane's steps, as the Dictate without
processing shortcut (`raw_dictation_hotkey`) does. History keeps it as `raw`,
and Paste Last pastes it. `--raw` with `stop` is a usage error
(exit 2); `stop --print` of a raw dictation prints the transcript as heard.

`dictation cancel` discards the recording in any shortcut mode: nothing is
transcribed, pasted, or saved to History, and dictations already transcribing
finish. With nothing recording it answers `unchanged`. In Toggle mode the
cancel shortcut (`cancel_hotkey`, Escape by default) does the same; on Wayland,
bind `hex dictation cancel` to a key in the desktop instead (#786, #1022).

`onboarding reset` (or `onboarding restart`) opens the window on the
first-run onboarding at once, as on a new install, for trying it again, on
macOS, Linux, and Windows. It removes only the onboarding markers (`onboarding-complete`, `onboarding-step`,
`onboarding-relaunch`) from the data folder. Settings, history, and
permissions are untouched, so the run finds them ready and the permission step
is skipped. Labs' Run setup again row does the same.

`paste-last` needs a listening Hex (`not-ready` during setup) and is refused
with `busy` while anything records or a shortcut gesture is in progress. The
menu bar's Paste Last Dictation does nothing in that case.

Transcription is cloud-only: the service and language are the
`transcription.provider` (`claude` or `codex`) and `transcription.language`
settings, changed with `hex settings set`. There is no `hex model` command and
no local model to download.

`vocabulary` edits the [shared vocabulary](features/vocabulary.md) through the
running Hex, which saves settings and reloads them as a window edit would.
Terms match in any case; `add` of a term already listed only merges new
spellings, and replies `changed: false` when there are none.

`quit` and `restart` without `--force` are refused with `busy` when the
listener doesn't answer whether it's recording; no listener, or one waiting for
setup, counts as idle. On Linux a Hex its systemd unit runs restarts through
the unit (after writing this desktop's `session.env`); elsewhere `restart`
waits for the old Hex to drain and exit, then runs `hex start`.

On Linux, an older Hex an update left running (from before `hex` commands
moved to the local API) answers only on its `service.sock`. Every control
command and `hex start` then answer `outdated` (exit 1) instead of
`not-running`, and `hex restart` restarts its systemd unit into the installed
build and waits for it to answer (#915); one not run by the unit has to be
quit from its window or tray.

`start` and `restart` keep the `HEX_*` environment (`HEX_RETAIN_DICTATION_AUDIO`,
`HEX_ENABLE_LLM`, and so on): macOS opens apps without the caller's
environment, so Hex passes each `HEX_*` variable to `open --env`. `start`
takes them from the shell that runs it, `restart` from the running Hex. The
values are never logged.

## Exit codes

```ts
0  the command ran, or there was nothing to change (`changed: false`)
1  Hex refused or failed it (an error reply), or the transport failed
2  usage error (clap)
3  Hex isn't running (code `not-running`); control commands never launch it
4  `dictation stop --print` heard no speech (code `no-speech`); nothing printed
```

## Transport

The CLI reads the owner-only discovery file
`~/Library/Application Support/hex/local-api.json` (`HEX_APPLICATION_SUPPORT_DIR`
overrides the folder), checks that its pid is alive, and sends one
`POST /control` to `127.0.0.1:<port>` with `Authorization: Bearer <token>`.
The route is always on in the running app on every OS; on Linux and Windows
the discovery file lives in `~/.local/share/hex` and `%APPDATA%\hex`.

- Request bodies are JSON up to 64 KiB; replies are at most 1 MiB.
- The app answers within 10 s (`504 control-timeout` otherwise); the CLI waits
  15 s. `dictation stop --print` gets 65 s, and the CLI waits 70 s.
- Every answer the app gives is `200` with a `ControlResponse`, refusals
  included. Transport-level failures use HTTP status codes:
  `400 invalid-control-command` (malformed or unknown command),
  `404 control-unavailable` (no control host), `413 control-too-large`,
  `503 control-busy` (too many in flight) or `control-unavailable`, and
  `504 control-timeout`. Bad or missing tokens get `401` like every other
  route.

## Wire format (version 7)

Requests are flat objects tagged by `command` (kebab-case):

```json
{"version": 7, "command": "settings-set", "key": "sound_effects", "value": "off"}
{"version": 7, "command": "dictation", "action": "start"}
{"version": 7, "command": "dictation", "action": "stop", "print": true}
```

```ts
status | quit {force} | restart {force}
dictation {action, print?}                       action: start | stop | toggle | cancel;
                                                 print (stop only) answers `transcript`
paste-last
settings-get {key?} | settings-set {key, value}
window-show {pane?} | window-hide | window-story {story, variant?, play}
onboarding-reset
history-clear | check-for-updates | launch-at-login {enabled?}
vocabulary-list | vocabulary-add {term, sounds_like} | vocabulary-remove {term}
```

Optional fields may be left out. `hex start` and `hex logs` are local and have
no wire form. `history-clear` has no confirmation field: the CLI asks for
`--yes` before it sends it.

Replies are tagged by `status`:

```json
{"version": 7, "status": "done", "message": "Quitting Hex", "changed": true}
{"version": 7, "status": "error", "code": "feature-off", "message": "the playground is in Labs; …"}
{"version": 7, "status": "settings", "settings": [{"key": "sound_effects", "value": true,
  "kind": "bool", "writable": true, "about": "Play recording and error tones"}]}
{"version": 7, "status": "vocabulary", "entries": [{"term": "OpenCode",
  "sounds_like": ["open code"]}, {"term": "Rune"}]}
{"version": 7, "status": "transcript", "text": "Ship it."}
```

`done.changed` is `false` when the command found things already so ("Already
recording", "The window isn't open"). Setting `kind` is one of `bool`,
`number`, `integer`, `choice` (with `choices`), `text`, `shortcut`, `path`, or
`json`; `value` is `null` for an unset optional setting.

### Status

```json
{
  "version": 7, "status": "status",
  "app_version": "2.3.1", "pid": 31708, "memory_footprint_bytes": 412000000,
  "listener": {"running": true, "status": "Listening"},
  "capture": {"state": "idle", "recording": null, "pending_jobs": 0},
  "transcription": {"provider": "claude", "language": "en"},
  "features": {"release_microphone_while_idle": false, "history_retention": "week"},
  "permissions": {"ready": true, "microphone": "ready", "accessibility": "ready"},
  "window": {"open": false, "pane": null},
  "update": "current",
  "labs": ["live-text"]
}
```

```ts
listener.status     Enabled | Waiting for setup | No microphone | Stopped (running false)
listener.shortcuts  Wayland portal only: [{"name": "Dictation", "trigger": "Super+D"}], the keys the desktop
                    granted (changed in its settings or not); trigger null when it refused one
capture.state       idle | recording | transcribing | unknown (the listener didn't answer or stopped)
capture.recording   dictation | service | null
permissions.*       ready | needs-request | needs-settings | needs-restart
window.pane         settings | modes | history | …
update              current | checking | available | unavailable
```

`memory_footprint_bytes` is the physical footprint Activity Monitor shows.
`labs` lists the [Labs](features/labs.md) features turned on; older Hex omits it.

### Error codes

```ts
not-running            Hex isn't running (CLI only; exit 3)
outdated               Linux: an older Hex an update left running; run `hex restart` (CLI only)
transport              the request couldn't reach Hex or its reply was unreadable (CLI only)
version-mismatch       this hex speaks another control version; use the running app's hex
unknown-setting        no such key (the message suggests a near match)
read-only              the key can't be set here (the message says where)
invalid-value          the value doesn't parse or is out of range
confirmation-required  clears history; pass --yes
shortcut-in-use        another Hex shortcut already uses that binding
feature-off            the command needs an opt-in that is off (the playground)
not-ready              Hex is waiting for setup (permissions)
busy                   something is in progress (recording, transcribing, a full queue),
                       or the listener couldn't confirm it's idle
discarded              `stop` dropped the clip: no audio yet, or silent; nothing transcribes
no-speech              `dictation stop --print` heard no speech; nothing printed (exit 4)
not-recording          `dictation stop --print` with nothing recording
unavailable            a part of Hex the command needs isn't available
unsupported            this build or OS can't do it (e.g. restart from a development binary)
failed                 Hex tried and failed (the message has the cause)
internal               reading or saving settings failed
```

## Settings

Keys are dotted paths into `settings.json`. `hex settings list` prints every
key with its kind and description; `hex settings list --json` adds `choices`.
Changes go through the same rules as the window and are saved by the app,
which then reloads them live; an open window adopts the change unless it has
an unsaved edit.

Transcription is cloud-only: `transcription.provider` takes `claude` or
`codex`, and `transcription.language` takes a language code (`en`, `de`, `fr`,
…) or `auto`. Both are writable here; a change applies to the next dictation.

- Booleans take `on`/`off`, `true`/`false`, `yes`/`no`, or `1`/`0`.
- Optional settings take `none` to clear them.
- `microphone` takes a device name exactly as Settings lists it, or
  `system-default` (or `none`) to follow the OS default input.
- `avoid_bluetooth_microphone` (default on) makes System default record from
  the built-in microphone, else another wired or USB input, when the OS
  default is a Bluetooth headset, so its audio stays out of call quality (on
  Linux, Hex moves its PipeWire stream off a Bluetooth default source after
  opening it, so the headset may still switch to hands-free mode; see
  [Bluetooth headsets](linux.md#bluetooth-headsets)). A saved microphone and
  `--device` are always used as named.
- Shortcuts take text such as `option`, `right-command`, `option+shift+v`,
  `control+space`, `fn`, `f5`, or `left-option+right-option` (both Options
  held together). `left-` and `right-` apply to shortcuts without a key. On macOS, Windows and X11, `mouse4` and `option+mouse4`
  also work (Mouse 1/2/3 are left/right/middle). Key names resolve on the active keyboard
  layout, the OS's rules for Settings' recorder apply, and a binding another
  Hex shortcut uses is refused with `shortcut-in-use`. `double_tap_only`
  needs a key chord, as in Settings.
- On Linux with `labs.standalone_special_keys` on (`hex labs on
  special-keys`, off by default), a shortcut may use `menu`, `pause`, or
  `scrolllock`, alone or with modifiers, for example
  `hex settings set dictation_hotkey menu` after remapping Caps Lock to Menu.
  Off, they are refused with `invalid-value`, and turning the toggle off puts
  a shortcut on one back to its default. Elsewhere one alone still needs a
  modifier.
- `second_dictation_hotkey` (macOS, unbound by default; `none` clears it) is
  a second shortcut that dictates exactly as `dictation_hotkey` does, such as
  a key on an external keyboard. It acts only while
  `labs.second_dictation_shortcut` is on (`hex labs on
  second-dictation-shortcut`) and shares no key with another shortcut.
- `json` settings (modes) take the JSON value the file stores.
- `ui_language` takes `en`, `pl`, `zh` (Simplified Chinese), `zh-Hant`
  (Traditional Chinese), `ja`, `de`, `es`, `uk`, `pt`, `fr`, or `none`
  (follow the system language), the same languages Settings offers. Following
  the system, a Chinese locale with the Hant script or the TW, HK, or MO
  region shows Traditional Chinese; other Chinese locales show Simplified.
- `updates.channel` takes `stable`, `dev`, or `none` (follow the build: the
  channel it was published on), and checks for updates at once. Choosing
  another channel than the build's offers that channel's current release
  ("Switch to Stable 2.3.1" in the window); Hex installs it only once you
  confirm there. Versions are never compared.
- `transcription.provider` (`claude` or `codex`) and `transcription.language`
  (a language code or `auto`) choose the cloud transcription service.

`recording_audio_behavior` also accepts `duck` (Lower audio): macOS and Linux
lower the default output, Windows lowers other apps' audio sessions.
`recording_duck_volume` sets the fraction of the original output volume (0–1,
default 0.2), applied on the next intentional capture when ducking is selected.

Writable keys: `transcription.provider`, `transcription.language`,
`release_microphone_while_idle`,
`sound_effects`, `sound_effect_volume`, `sound_theme`, `microphone`,
`avoid_bluetooth_microphone`, `recording_audio_behavior`, `recording_duck_volume`, `double_tap_lock`, `double_tap_only`,
`shortcut_mode`, `dictation_hotkey`, `paste_last_hotkey`, `raw_dictation_hotkey`,
`second_dictation_hotkey`, `labs.second_dictation_shortcut`, `labs.standalone_special_keys`, `show_dock_icon`,
`dictation_processing.default_mode`, `dictation_processing.modes`,
`history_retention`, `experimental.hex_microphone`, `experimental.hex_microphone_mode`,
`experimental.hex_microphone_comfort_noise`,
`experimental.hex_microphone_strength`, `enable_on_launch`,
`paste_with_shift`, `ui_language`, `indicator_position`, `updates.channel`.

A test (`control::settings::tests::every_persisted_setting_has_a_cli_key`)
fails when a new persisted setting has no key here.

## Compatibility

`CONTROL_VERSION` is 7. A change that breaks a request or reply shape
increments it; adding a command, an optional request field, or a reply field
does not. A `hex` from a different version gets `version-mismatch`; use the
`hex` inside the running Hex.app, or restart Hex after updating. Version 2
removed `permissions.input_monitoring` from the status reply; version 3 removed
`voice-action` and `features.voice_action`; version 4 removed `commands-retry`,
`settings-set.confirm`, `window-show` `commands`, `capture.recording` `voice`,
`features.commands`, and `features.command_model`; version 5 removed
`meeting-record`, `meeting-stop`, `meeting-copy-last`, `window-show`
`meetings`, the status reply's `meeting` and `features.meetings`, and the
`meeting-active` error; version 6 removed `scrawl-capture`,
`scrawl-capture-list`, `scrawl-capture-show`, `scrawl-capture-send`,
`scrawl-capture-undo`, `scrawl-capture-route`, their replies
(`scrawl-capture`, `scrawl-captures`, `scrawl-capture-detail`, and
`scrawl-capture-route`), and the `not-found` error; version 7 removed
`model-list`, `model-use`, `model-install`, `model-cancel`, the `models`
reply, the status reply's `model` (now `transcription: {provider, language}`),
`permissions.transcription_model`, and the `unknown-model`, `model-unavailable`,
and `not-installed` errors (transcription is cloud-only).
