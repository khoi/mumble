# Mumble Feature Map

What can the user do, how do they reach it, and what would prove it works?
This maps existing behavior. [ROADMAP.md](../../ROADMAP.md) owns future work;
[AGENTS.md](../../AGENTS.md) owns implementation invariants.

Initial source baseline: September 1, 2026, `9e9da53aa9ed`. Public macOS release:
[2.3.1](../releases/2.3.1.md), with custom-domain downloads, a GitHub DMG mirror,
and model-download recovery guidance. The initial map was source-only. Listed checks are
locators unless an executed result is explicitly recorded, as in the
[keyboard-layout verification](recovery.md#keyboard-layout-resolution).

## Product Map

```ts
Mumble                                      // macOS release
├── Get ready                            // install, permissions, sign in to Claude or Codex
├── Dictate                              // hold OR double-tap lock
├── Use the result                       // paste, paste-last, History
├── Adjust behavior                      // shortcut, microphone, recording preferences
├── Process text                         // Output tab + context-selected Modes
├── Opt in                               // Labs: off by default
├── Recover                              // prove the next attempt works
└── Maintain the app                     // menu bar, login, updates, quit
```

[Dictation](dictation.md) and [Recovery](recovery.md) have detailed maps.
Agents can [read retained transcripts](transcript-cli.md) and
[control the running app](control-cli.md) with the release CLI.
[Labs](labs.md) lists every unfinished feature as a row, each off by default.
Its Restore shortcut default toggle (off by default) puts a Restore default
button beside Paste last dictation's shortcut (#613).
General › Advanced › Interface language offers Português (Brasil), and Mumble
uses it for any Portuguese system language (#867); it also offers Français,
used for any French system language (#892). Neither has had a native
speaker's review yet. It offers 简体中文 (`zh`) and 繁體中文 (`zh-Hant`, #944);
Traditional Chinese is drafted from the Simplified strings by a character and
Taiwan-usage phrase converter (`desktop/i18n/traditional_chinese.rs`), not yet
reviewed by a native speaker.
The [vocabulary](vocabulary.md) lists names you want spelled correctly; Mumble
sends them to the transcription service as keyterms.
The other capabilities stay here until they need their own verification recipe.

## Get Ready

Install the signed macOS app with `brew install --cask anomalyco/tap/mumble` or the
[manual DMG](../../README.md#downloads). The cask uses a versioned, checksum-pinned
DMG, requires Apple silicon/macOS 15+, and leaves updates to Mumble's own updater. It does
not require OpenCode, launch the app automatically, migrate the legacy
Swift app's data, or forcibly replace an existing app destination. Homebrew uninstall
leaves runtime settings, models, and retained data intact.

```ts
Download Mumble                              // setup.download-recovery
├── anoma.ly/mumble names the visitor's platform  // setup.download-button
│   ├── Mac -> DMG | Intel Mac -> disabled, "needs Apple silicon"
│   ├── Windows x64 / ARM64 (UA-CH) -> mumble-setup-<version>-windows-<arch>.exe + SmartScreen note
│   ├── Linux -> copyable `curl … install-linux.sh | sh` + Arch/glibc requirement
│   ├── Phone -> Send yourself the link
│   └── Other platforms -> every build, Homebrew, the GitHub release
├── Website, README, installers, apps, feeds, appcast -> anoma.ly/mumble/files -> R2 bytes (Worker mumble-production-site)
├── Releases up to 2.3.1 -> downloads.hex.kitlangton.dev (Kit's frozen bucket) -> the same bytes
├── GitHub v<version> (app-v through 2.3.1) -> identical DMG, Setup.exe, and Linux bytes
└── Older app cannot reach its update feed -> manually install the DMG

Install on Windows                        // setup.windows-installer
├── Setup.exe, per user, no admin -> managed versions\<version>\mumble.exe
│   ├── another version installed (e.g. self-updated) -> install this one over it
│   └── finish -> Open Mumble
└── Settings › Apps -> uninstall every version, shortcut, Launch at login; keep data

Install on Linux                          // setup.linux-installer
└── curl … | sh -> signed feed -> verified artifact -> launcher + mumble.service
    └── cannot start -> name the glibc floor or the missing Arch packages

Sign in to transcribe                     // setup.sign-in
└── Dictation -> Claude or Codex, through the sign-in their own app holds
    └── Signed out or expired -> the dictation fails with that reason -> sign in
```

The website shows the pinned macOS version and platform requirements beside its
download links. App mirrors use explicit `v…` release tags (`app-v…` through 2.3.1) rather than GitHub's
repository-wide latest release. Builds on the release
service update from `https://anoma.ly/release` (docs/releasing.md); the 2.3.1
feeds, appcast, and the legacy `r2.dev` endpoint stay frozen for installed
clients. Hosting does not guarantee access through every corporate policy.

The site's hero is an animated, optional show: an idle head and an ⌥ keycap until
the visitor presses it, then a voiced demo of messages, OpenCode prompts and fixes
(WebGPU, with a Canvas 2D fallback). Audio loads only after that press; the website
has no microphone button or microphone access. Reduced motion keeps a still until
the press and then plays only the calm stage. See [site authoring](../../site/README.md).
Headless Chromium renders of the WebGPU, Canvas 2D, reduced-motion and phone paths
showed no console errors; a deterministic 60 fps capture was reviewed frame by
frame. These checks do not exercise real speakers or a physical phone.

The Windows installer is silently installed, reinstalled, installed over
another version, and uninstalled on x64 and ARM64 runners by
[test-windows-installer.ps1](../../scripts/test-windows-installer.ps1); no
interactive install, SmartScreen prompt, or first launch on a real PC has been
observed. The Linux library diagnosis has no automated check. The site's
platform states were captured once in headless Chromium with `?platform=`.

Sources: [website](../../site/src/Download.tsx), [installer](../../packaging/windows/mumble.iss),
[release publisher](../../.github/release/release-service.sh),
[mirror publisher](../../scripts/publish-app-mirror.sh),
[macOS release publisher](../../scripts/release-app.sh),
[Linux installer](../../.github/release/install-linux.sh), and
[updater](../../src/updater.rs).
Executed hosting checks and deployment details are recorded in
[Download Hosting](../downloads.md). In-app feed defaults and recovery copy ship
with 2.1.17; the initial 2.1.16 mirror preserves that release's original bytes.

```ts
Launch Mumble, setup incomplete              // setup.tour; every build and OS, full window
├── One onboarding: the first-run story (rune_ui/onboarding_kit/live.rs) over
│   the tour state machine (desktop::onboarding::Flow); no flag, no other tour
├── Skip (quiet, top-right, only on the demo and the practice)    // setup.skip
│   ├── Passes over that part to the next step setup still needs: permissions,
│   │   then Almost ready, else Mumble is ready
│   ├── Never closes onboarding or records it done
│   └── No Skip on permissions or the paste
│       permission. Check: `onboarding_skip` (scripts/check-onboarding.sh)
├── GNOME/KDE Wayland (pastes through the RemoteDesktop portal, no wtype)
│   and no saved grant -> before Your turn, "Let Mumble paste for you"     // setup.paste-access
│   ├── Says why: Mumble only sends the paste keystroke, never sees the screen;
│   │   the desktop asks once and may call it remote desktop (#932)
│   ├── Allow -> opens the desktop's dialog; the grant's restore token is saved,
│   │   so the first paste needs no dialog
│   ├── Refused -> Try again, or Continue (the desktop asks at the first paste)
│   ├── X11, wlroots with wtype, macOS, Windows -> no page
│   └── Skipped or refused -> Settings › General shows it under "Permissions
│       needed" until allowed. Check: golden `onboarding-paste`; the real
│       portal dialog is not exercised
├── 2.3.1 or Swift Hex -> Sparkle offers the new Mumble -> Install Update   // setup.legacy-update; macOS
│   ├── Replaces the old app in place (its file name kept) and opens the new one
│   ├── Accepted only because app/Info.plist keeps the old SUPublicEDKey;
│   │   scripts/validate-app.sh requires it. Feed items: docs/plans/swift-app-handoff.md
│   └── No automated check; a clean-Mac install from each old app is the proof
├── 2.3.1 Linux or Windows updates into this build     // setup.legacy-adopt
│   ├── It lands in <data>/voice-control/versions/<version>; its first app or
│   │   `service` launch from there moves it into the managed layout, then
│   │   relaunches from it (Linux: exec, so the old unit keeps its process)
│   ├── Linux -> 2.3.1's marked unit, ~/.local/bin/mumble link, and mumble.desktop
│   │   entries move aside -> the built-in .github/release/install-linux.sh
│   │   installs this executable -> session.env copied; failure puts them back
│   ├── Windows -> %APPDATA%\mumble\versions, receipt, Mumble.lnk recreated,
│   │   Launch at login repointed, 2.3.1's Apps & Features entry removed
│   ├── Failure -> logged, runs in place; the next launch tries again
│   ├── Settings, history, and models in voice-control are not carried over
│   └── Check: `a_legacy_linux_install_moves_into_the_installers_layout`,
│       `a_failed_move_puts_the_legacy_files_back`; Windows and a real 2.3.1
│       feed update are unverified
├── Upgrading from 2.3.1 or earlier (same `mumble` folder)     // setup.upgrade; macOS
│   ├── `anomaly.mumble` is a new identity: macOS kept none of the old grants
│   ├── Settings without `anomaly-mumble-identity` in `migrations` -> checked once at launch
│   │   ├── Permissions not ready -> the old completion
│   │   │   no longer counts -> onboarding opens from the start
│   │   └── Everything ready -> no onboarding; nothing but the ID changes
│   ├── Saved settings are kept; the ID is recorded either way
│   └── Setup ready, listener starting -> a running 2.3.1 or Swift Hex   // setup.quit-legacy
│       (retired bundle IDs) is asked to quit, so both never answer Option;
│       it stays installed. No check; `macos_rune::quit_legacy_mumble`
├── The demo: Without Mumble (typed, with typos) -> With Mumble (spoken) -> Continue
├── Quit part-way -> next launch resumes at the saved step  // setup.resume
│   └── Permissions are read again at launch: a setup they now finish (macOS's
│       Quit & Reopen after a grant) is passed; past setup, it waits until they do
├── Permissions                           // setup.checklist; skipped when granted
│   ├── macOS: "Let Mumble hear you and type for you", Microphone and Accessibility
│   │   ├── No Input Monitoring row: Accessibility also lets the shortcut taps listen
│   │   ├── Microphone -> macOS prompt; refused -> "Open System Settings" + guide
│   │   ├── Accessibility -> Privacy pane + drag guide
│   │   │   ├── Retired Mumble bundle IDs (`com.kitlangton.hex2`, `com.kitlangton.Hex`,
│   │   │   │   `com.kitlangton.Hex.debug`) cleared once in the background (`tccutil reset`,
│   │   │   │   ignoring `OSStatus -10814` when no bundle is on disk)
│   │   │   ├── Returned from System Settings still untrusted (2 s, or 6 s side-by-side) ->
│   │   │   │   "An older Mumble entry can keep this off" + "Still grey? Fix it"
│   │   │   │   -> resets the running bundle ID + retired IDs, prompts, and reopens the guide
│   │   │   └── macOS 27 names the list "Device Control and Data Access"; the row follows
│   │   ├── On in System Settings, unusable until relaunch -> "Needs a restart"
│   │   │   └── Every permission on -> "Restart Mumble" -> relaunch at setup  // setup.restart
│   │   └── Last grant -> 0.7 s beat -> the story moves on
│   ├── Windows: only while the microphone privacy switch is off: "Let Mumble hear
│   │   you", one Microphone row -> Open Settings (ms-settings:privacy-microphone)
│   └── Linux: no page
├── Almost ready -> Your turn
├── Your turn: the practice game          // setup.practice; every OS
│   ├── "Clear your messages": hold the saved shortcut and answer the lit card;
│   │   Mumble's own dictation (macOS listener, Linux and Windows event log) holds
│   │   the keys, flies the live words, and lightning burns the answered card
│   ├── A short, silent, or failed try -> a hint under the field
│   └── Done at any time -> Mumble is ready  // practice is never required
├── Listener cannot start after setup     // setup.relaunch; event tap refused
│   ├── First time -> Mumble records a marker, reopens itself, resumes at the practice
│   └── Within 2 min of that relaunch -> "Mumble's shortcut didn't start." + Reopen Mumble
└── Mumble is ready -> where Mumble lives (menu bar, system tray, or app launcher)
    ├── Try again -> back to the practice game
    └── Start using Mumble -> records completion once setup is ready; closes onboarding
Run setup again (Labs) or `mumble onboarding reset` -> the same story from the start,
on every build; settings and permissions stay, so their pages pass
```

Permissions revoked later
  -> General > Permissions needed: Allow rows like onboarding's
     ├── Returned from System Settings still untrusted -> "Still grey? Fix it"
     └── On, waiting for a relaunch -> "Restart Mumble"
```

Observed on macOS 26.6.2 and macOS 27.0 VMs (2026-10-31, SIP enabled): unprivileged
`/usr/bin/tccutil reset` from inside `anomaly.mumble` clears `Accessibility`,
`ScreenCapture`, and `AudioCapture` entries for retired Mumble bundles on disk with
no password prompt (`rc=64` / `OSStatus -10814` when absent from disk), and
resetting `anomaly.mumble` followed by `AXIsProcessTrustedWithOptions` /
`CGRequestScreenCaptureAccess` replaces a stale `CDHash` entry that stayed grey
or untrusted in System Settings. Checks:
`tccutil_exit_codes_distinguish_cleared_absent_and_failed_bundles`,
`stale_tracker_waits_for_return_from_system_settings_and_ignores_touch_id_focus`,
`stale_accessibility_row_resets_permission_and_returns_to_waiting` in
[desktop/onboarding.rs](../../src/desktop/onboarding.rs), and
`stale_permissions_offer_fix_it_and_reset_entries_across_panes` in
[rune_ui/tests/panes.rs](../../src/rune_ui/tests/panes.rs). Scenes:
`mac-onboarding-stale`, `mac-settings-permissions-stale`.

Apple silicon, macOS 15+. Transcription is cloud-only: every dictation goes to
Claude or Codex through the sign-in those apps already hold. There is no local
speech model to choose, download, or run.

```ts
General > Dictation > Transcribe with, then Language below it   // setup.transcription; every desktop root
├── Transcribe with -> Claude or Codex (transcription.provider; Claude default)
│   └── Uses the app's own sign-in (Claude Code's token, Keychain, or
│       .credentials.json; Codex's auth.json); Mumble never refreshes or writes it
├── Language -> a language code (transcription.language; "en" default), or Auto
└── Vocabulary (vocabulary.md) rides along as keyterms (Claude)
```

Sign in to Claude Code or Codex in their own app before dictating; an expired
or missing sign-in reads as signed out, and the dictation fails with that
reason (there is no local fallback). The selection is two `mumble settings` keys,
`transcription.provider` and `transcription.language`, applied to the next
dictation.

Sources: [transcription/cloud.rs](../../src/transcription/cloud.rs)
(`CloudProvider`, `dictation`),
[cloud/claude.rs](../../src/transcription/cloud/claude.rs),
[cloud/codex.rs](../../src/transcription/cloud/codex.rs),
[cloud/stream.rs](../../src/transcription/cloud/stream.rs), and the
Transcribe with and Language rows in
[rune_ui/general.rs](../../src/rune_ui/general.rs).

## Dictate

```ts
Saved shortcut                           // dictation; Option by default; Windows and X11 accept mouse buttons, macOS with Labs > Mouse shortcuts
├── Hold -> Speak -> Release
├── Tap, tap -> Speak hands-free -> Press again   // Shortcut mode: Hold or double-tap
├── Tap mode (Toggle) -> Cancel shortcut, Escape by default, while recording
│   -> Discarded: nothing pasted or saved; transcribing jobs finish (#1022)
├── Hold modes -> no cancel: Escape reaches the app, every dictation pastes (#786)
└── macOS recorder: a key alone needs a modifier, except F1–F20 and Home, End,
    Page Up, Page Down, Help (#1138; keys_without_modifiers_need_one_except_function_keys)

General > Shortcut mode -> Hold or double-tap (default), Hold, Tap, Double-tap
  -> Modal: each choice with its keys and a looping key/recording timeline
  -> Double-tap needs a key binding; with a lone modifier it says so

Labs > Second dictation shortcut (macOS, off)   // labs.md; issue 627
  -> General shows "Second dictation shortcut" under Shortcut (unbound, clearable)
  -> it dictates like the shortcut: Hold/Toggle, double-tap lock, release
  -> the shortcut that starts a capture finishes it; the other stands down meanwhile
```

The [detailed map](dictation.md) covers timing, successive captures, output
commitment, named checks, and native-proof gaps.
The macOS CLI entry is `mumble listen`, with an optional authoritative `--device`.
Do not start it beside an existing listener.

## Use The Result

```ts
Completed output                         // output
├── Paste -> Current focus, in submission order
│   ├── Previous clipboard restored 500 ms later, unless something newer replaced it
│   ├── Windows -> the transcript is marked out of clipboard history (Win+V) and cloud sync (#851)
│   ├── Labs > Keep dictation on clipboard (macOS, off) -> the dictation stays on the clipboard (labs.md)
│   └── Labs > Copy dictation only (macOS, off) -> clipboard only: no paste, typing, or restore (labs.md)
├── Paste Last Dictation                  // menu or Option-Shift-V / saved binding
│   └── Last completed dictation
└── History
    ├── Inspect / search / copy (row hover or the card) / delete
    ├── Retention ("Keep: 7 days" menu) -> Seven days by default, always hard-capped
    └── Off -> Stop new entries; preserve existing entries
```

Checks in [paste.rs](../../src/paste.rs), [dictation_worker.rs](../../src/dictation_worker.rs), and
[history.rs](../../src/history.rs) cover output and retention boundaries.
They do not prove that an arbitrary target consumed its clipboard contents.
Send is not an atomic target-app transaction. Chromium 155 and 156 get the text
typed instead of pasted for a [Chrome paste bug](dictation.md#ongoing-jobs-and-output).

**Easy to misread:** History Off does not stop diagnostic logging. Diagnostic
logs are separate and currently unbounded on disk.


The shared `EventReader` streams a file-length snapshot in 16 KiB chunks rather
than copying the entire log into memory. It retains up to 1,024 event rows and
the latest context, even when that context predates the retained rows.
Diagnostic records over 1 MiB are skipped through their terminating newline;
the original log and dictation output remain intact. Partial UTF-8 records carry
over to the next refresh, and truncation or file replacement resets the reader.

Checks: the `reader_*` regressions in [desktop/events.rs](../../src/desktop/events.rs). These
exercise synthetic files, not an installed app. The GPUI window's
on-demand-loading regression went with GPUI; when the Rune window reads the
log is not covered by a test. A first Commands/Activity refresh still scans the
full log snapshot; disk rotation and background parsing remain separate work.

Observed September 26, 2026: the reader and pane-loading regressions passed.
An optimized isolated harness using the production reader and a synthetic
64 MiB log reduced peak process RSS from about 204 MB to 2.4 MB; refresh took
254 ms before and 222 ms after. This is reader evidence, not a native Settings
latency measurement.

## Adjust Behavior

```ts
Sidebar                                  // settings.nav
├── General, Output, History, then each feature that is on: Voice Filter
│   (a Labs toggle), Playground (Labs, an item)
└── Labs -> one flat page of toggles, each off by default; mumble://labs/<feature> opens it at a row (labs.md)
    // Command-1…9 follow the panes' order (labs::order)

General                                  // settings
├── Basics card -> Shortcut, Transcribe with, Language, Microphone, Launch at login
│   ├── Each row leads with an icon; choices are inline dropdowns, not segments
│   ├── Transcribe with -> Claude or Codex; Language -> a dropdown of codes, Auto first
│   └── A row that hides or shows (Advanced, a microphone error)
│       -> Folds under a clip as the rows below slide over it; no fade in place
├── Microphone -> Value names the input in use, with live level bars // settings.microphone
│   ├── "System default · Kit Pods", or a saved device
│   ├── Level bars -> peak of the audio Mumble receives, ~30 fps while General shows them above rest // input_level
│   │   ├── Read from one atomic the audio owner stores per chunk; never opens or blocks the mic
│   │   ├── Quiet room (bars at rest) -> no 30 fps frames; the window's 250 ms tick notices a rise.
│   │   │   An open General window with a warm mic woke 34.5 times a second in a quiet room before
│   │   │   (dev-364, headless sway on apollo, Oct 31) and 4.1 after; macOS was reasoned about only
│   │   ├── No audio for 250 ms (released while idle, failed, not allowed) -> flat, dimmed
│   │   └── Scenes: mac-settings-mic-level (a recorded WAV, `--video`), mac-settings-mic-released
│   │       // check: the_microphone_level_follows_the_voice (Linux)
│   ├── Re-listed on a device or default change while General shows (Windows: also every 3 s)
│   ├── System default -> The OS default input, followed live
│   │   ├── Default changes -> Switch once capture is idle; new stream opens before the old drops
│   │   ├── Default is Mumble Microphone -> Built-in mic, else first real input; row says so
│   │   ├── Stand-ins never include virtual or aggregate inputs (macOS: e.g. Microsoft Teams Audio)
│   │   │   ├── Nothing real left -> "lid is closed" / "no microphone" error, not silence
│   │   │   └── A virtual default, saved device, or `--device` is still used as named
│   │   └── Default is Bluetooth (Advanced switch on) -> Built-in mic, else first wired/USB input
│   │       ├── Row says "KIT PODS is Bluetooth, so Mumble listens to MacBook Pro Microphone"
│   │       ├── Windows: WASAPI `BTHENUM`, or a headset/handset endpoint of unknown bus (`BTHHFENUM`)
│   │       ├── Linux: a `bluez5` default source; Mumble opens ALSA's default, then moves its own
│   │       │   stream to the PCI input, else the first other hardware source (pactl, PipeWire);
│   │       │   the stream is marked so WirePlumber never remembers the move. A Bluetooth
│   │       │   default chosen while Mumble's stream is open is followed until the next open
│   │       ├── No other usable input (or lid closed, Bluetooth only) -> the Bluetooth default
│   │       └── A saved Bluetooth mic or `--device` is always used as named
│   ├── Saved device missing -> System default stands in, value names it
│   │   ├── Row says "SHURE MV7 isn't connected; using your Mac's input"
│   │   ├── Menu keeps "SHURE MV7" checked with a faint "not connected"
│   │   └── Device returns -> Switch back once idle; the setting never changes
│   ├── Menu lists real inputs only                                // never Mumble Microphone
│   └── Open menu covers the controls beneath it; click elsewhere dismisses
├── Advanced ▸ -> one fold per pane, closed by default, even while a setting in it is in use
│   ├── Collapse -> drops a pending release confirmation
│   ├── Dictation -> Shortcut side, keyboard-language mappings (while its Labs toggle is on),
│   │   Use the Mac's microphone instead of Bluetooth (Linux, Windows: "Use another
│   │   microphone instead of Bluetooth")
│   ├── Output -> Paste last shortcut, While dictating, Dictation HUD, Terminal paste (Linux)
│   │   (Dictate without processing sits in the Output pane; see dictation.md)
│   └── Application -> Enable on launch, Show Dock icon, Sound volume, Sounds,
│       Interface language, Update channel (self-updating installs)
│       // Interface language System: macOS's first preferred language (zh-Hans -> 简体中文,
│       // zh-Hant, zh-TW, zh-HK, zh-MO -> 繁體中文), Linux LC_ALL/LC_MESSAGES/LANG
│       // (zh_TW.UTF-8 -> 繁體中文), Windows the user locale; unknown -> English
├── Use the Mac's / another microphone instead of Bluetooth (Advanced, on)
│   └── Off -> System default follows a Bluetooth default too; re-resolves once idle
├── Labs > Share dictation shortcut with other apps (Windows, off)
│   └── On -> the hook passes the shortcut's key-down/up on instead of consuming it
├── Terminal paste (Advanced on Linux, Labs on Windows; off)
│   └── On -> paste sends Ctrl+Shift+V instead of Ctrl+V; applies to the next paste
├── Turn off the mic when not dictating (General, under Microphone; off)   // release_microphone_while_idle
│   ├── One switch, no confirmation; its line says what the state means (issue 61), on every OS
│   ├── Off (default) -> "The mic stays ready so dictation starts instantly; your system shows it in use"
│   │   Open while idle; pre-roll available
│   └── On -> "The mic opens only while you dictate; the first moment can be clipped"
│       Open on press; no pre-roll; startup delay
├── While dictating -> Mute / Pause media / Lower audio / Do nothing
│   ├── Lower audio -> Lower to slider, 0–100% of the previous volume (default 20%)
│   ├── Intentional capture only, not ordinary shortcut chords
│   ├── Pause/resume only players already reported running by macOS
│   ├── Linux restore -> Waits 300 ms; a new capture keeps it suppressed // double-tap lock
│   └── Labs > Also pause the browser (macOS, Pause media only; off by default); its permission row shows here while on
│       ├── Turning it on without System Audio Recording -> ask now, never while dictating
│       ├── On -> Screen & System Audio Recording row: Allow / waiting / Allowed
│       └── Dictation -> browser_media sequence below
├── Sound volume -> immediate feedback; zero suppresses tones
│   └── Default output changes or disappears (macOS) -> next tone reopens on the current default
├── Sounds -> Sheet: hear each set on Mumble's real HUD, pick one
└── Linux / Windows -> listener card and desktop commands below the basics

Pause media + Also pause the browser     // browser_media
├── A browser uses the microphone (a call) -> leave it alone
├── No browser process has its audio output open -> leave it alone (no tap)
├── Listen to those processes' own output (private process tap), 3 windows of 100 ms
│   └── Any window at or below -60 dBFS, or no level -> leave it alone
├── Send the system pause command (MediaRemote 1; never the toggle)
├── Poll 5 windows of 100 ms
│   ├── 2 windows in a row below -90 dBFS -> Mumble paused it
│   └── Otherwise (another tab, a sound that ended later, or the pause went elsewhere) -> no play later
└── Dictation ends and Mumble paused it -> first, before the players: listen 300 ms to every browser process
    ├── Above -60 dBFS (you started it again) -> no play
    ├── No level, a browser runs and every one has closed its output -> still paused: play
    │   (a pause longer than about 10 s closes the output; a tap on it delivers nothing)
    ├── No level otherwise (browser quit, tap failed) -> no play
    └── Otherwise -> send the system play command (MediaRemote 0), then poll 20 windows of 100 ms
        ├── A window above -60 dBFS -> resumed
        ├── No level before the first level (the output is opening again) -> keep polling
        ├── No level after a level -> stop listening
        ├── No browser output open after 10 windows (tab closed: the play reached another app) -> send pause once
        ├── Never a level and a browser opened its output -> stop listening
        └── Silent throughout (tab closed or reloaded: the play reached another app) -> send pause once

Intentional recording                    // recording.environment, idle_sleep
├── Prevent idle sleep
│   ├── macOS -> IOKit no-idle-sleep assertion
│   ├── Linux -> logind `idle:sleep` inhibitor; `idle` only where polkit refuses a
│   │   process outside the seat session (Mumble's user service) to block sleep
│   ├── Windows -> system-required power request (from the press, as its While dictating does)
│   └── Failure -> Warn and continue recording
└── Finish/cancel -> Release it
    // no caffeinate child process or executable-policy dialog
```

Feedback tones keep their output open for five idle minutes (macOS, Windows).
On Linux, where an open output stays listed in the desktop's sound controls,
it closes about a second after the last tone ends
([#720](https://github.com/anomalyco/hex/issues/720)); the existing
`output_opens_for_a_tone_and_releases_after_the_idle_grace` check covers the
grace logic, not a real PipeWire stream list. On macOS the
CoreAudio listeners on the default output, the device list, and an audio service
restart advance an output route generation; the next tone drops an output opened
under an older one and opens the current default at once, so unplugging a USB
or Bluetooth output no longer silences tones until Mumble restarts
([#428](https://github.com/anomalyco/hex/issues/428)). Check:
`a_route_change_reopens_the_output_on_the_current_default` in
[feedback.rs](../../src/feedback.rs), with injected opens; a real unplug has not
been observed by a test.

**Lower audio** (`recording.environment.duck`) is an explicit fourth
choice, off by default, on every OS. The slider appears only while selected and saves on
release. It remembers its value across behavior changes. Intentional captures
ramp the original output channels to that fraction over
one second, preserving balance and silent channels, then restore on finish,
cancel, or orderly shutdown. Changes apply to the next capture. The stop tone
waits for restoration. A manually changed volume after settling is left alone;
a newly selected output device is not lowered. Fixed-volume devices and outputs
whose channel balance cannot be safely restored are left unchanged. Linux
lowers the default sink's channels through `pactl` (one volume through `wpctl`
without it), so Mumble's own tones are lowered too and the stop tone waits, as on
macOS. Windows lowers
every other app's audio session instead, leaving Mumble's tones at full volume;
sessions that start while recording are left alone.

Mumble snapshots the volumes and applies a smooth ramp in
[audio/output_ducking.rs](../../src/audio/output_ducking.rs) over the CoreAudio adapter in
[ducking.rs](../../src/audio/recording_environment/ducking.rs), the sink adapter in
[linux/audio_control.rs](../../src/linux/audio_control.rs), and the session
adapter in [windows/audio_control.rs](../../src/windows/audio_control.rs). The worker runs inside the app and cannot restore volume after a
hard process crash or SIGKILL. Changes during the ramp itself cannot be
distinguished from delayed Bluetooth acknowledgements.
Fake-device tests cover relative volume, balance, silence, interrupted ramps,
manual changes after settling, and partial-write rollback; settings/UI tests
cover defaults, bounds, persistence, conditional visibility and platform gating.
On 2026-10-25, the macOS logic/settings checks and strict Clippy passed.
The seven reference images were regenerated on Ubuntu 24.04 ARM64 with Mesa
lavapipe, using the unmodified Linux-only Rune test module and
`MUMBLE_UPDATE_GOLDENS=1 scripts/check ducking`. A subsequent
`scripts/check rune_ui --nocapture` without the update flag passed all 113 tests,
including the unchanged control goldens; no golden skipped for lack of a GPU
adapter. The five existing images differ from the base branch above the existing
12-level channel tolerance only within the While dictating control; the sidebar,
Quit Mumble, and shortcut layouts remain unchanged. Dark and light ducking previews
were visually inspected. The earlier macOS-generated references were replaced
because these fixtures retain build-OS-dependent layout and defaults. This
verifies headless Linux rendering, not audible playback, physical devices,
crash recovery, or native Windows execution.

The Linux and Windows ports (2026-10-30) were checked by unit tests (pactl and
wpctl volume parsing, the Linux Bluetooth stand-in choice), the Settings test
that Lower audio and its slider appear on every OS, Linux Clippy, and Windows
x86_64 cross-Clippy. On apollo (niri, PipeWire), the ignored
`native_idle_sleep_prevention_acquires_and_releases` took a logind `idle`
inhibitor (polkit refused `idle:sleep` over SSH), and a marked `arecord`
stream moved with `pactl move-source-output` without WirePlumber storing the
target. Not verified: audible Linux or Windows ducking, Windows idle-sleep
and Bluetooth detection on hardware, a real Bluetooth headset on Linux, and
Windows ARM64 (its cross-build stops in `rquickjs-sys` on this host).

Successful settings saves persist and project changes at safe runtime boundaries.
An invalid saved field falls back independently at launch, with its key and
reason in the log; the next save preserves the original first. See
[settings recovery](recovery.md#load-settings-with-invalid-fields).
Immediate settings controls commit a candidate before changing the saved UI
choice or applying dependent native/history effects. A failed shortcut save
stays in capture instead of showing Saved; failed retention saves keep the old
choice and surface the error. Failed debounced text saves retain their drafts
for a subsequent edit or close-time flush; no Rune test injects a save failure
yet.
Persistence, conflict, and ownership checks live in
[app_settings.rs](../../src/app_settings.rs),
[audio/recording_environment.rs](../../src/audio/recording_environment.rs), and
[audio.rs](../../src/audio.rs). Settings previews do not prove physical device
switching or native mute support; muting is best-effort, not universal. The
recording-environment ownership checks cover overlapping acquisition and release,
not a managed-device policy or native power assertion. The pause-script regression
checks that inactive players are absent before AppleScript application resolution;
when no supported player is running, acquisition skips AppleScript entirely.
The script check does not prove subprocess avoidance or exercise Music, Spotify,
or VLC through Automation.
The browser pause runs after the AppleScript players and only when System Audio
Recording is reported granted (an unknown answer skips it, because the tap would
open the macOS dialog mid-dictation). macOS 15.4 and later no longer tell other apps
what is playing, so Mumble hears the browsers through a private process tap
(aggregate `anomaly.mumble.browser-tap.<pid>`) and sends
play only after it saw the browser go silent right after its own pause. The
sequence checks in [audio/browser_media.rs](../../src/audio/browser_media.rs) run on fakes:
`a_confirmed_pause_is_resumed`, `two_sounding_tabs_never_get_play`,
`a_browser_already_playing_gets_no_play`, `a_call_is_never_paused`,
`no_level_sends_nothing`, `a_play_the_browser_never_takes_is_paused_again`,
`a_browser_that_closed_its_output_gets_play`,
`a_closed_browser_that_never_opens_after_play_is_paused_again`; the ignored `native_tap_hears_a_playing_browser`
heard Brave at -25.4 dBFS on macOS 27.0.1 (2026-10-28). Which app receives the
system command is macOS's Now Playing choice; Safari is heard only when its
WebKit process reports Safari's name. An opt-in native
assertion smoke exercises IOKit without starting microphone capture.

**Changed after 2.3.1:** Automatic was a compiled preference list (Universal
Audio Thunderbolt, Studio Display Microphone), then the macOS default with a
Bluetooth default swapped for the built-in microphone (the 2.1.22 fix for
[#106](https://github.com/anomalyco/hex/issues/106) and
[#88](https://github.com/anomalyco/hex/issues/88)). That skipped a user's
AirPods default, and Mumble Microphone then passed the wrong microphone on to
calls. Automatic is now System default: the OS default input, followed live.
A Bluetooth default (CoreAudio transport Bluetooth or Bluetooth LE) is passed
over for the built-in microphone, else the first other real input, so AirPods
keep their music out of call quality while you dictate; it is kept when nothing
else is usable. Only System default is affected: a saved Bluetooth microphone
and `--device` are honoured, and General › Advanced › "Use the Mac's microphone
instead of Bluetooth" (`avoid_bluetooth_microphone`, default on) turns it off.
Mumble Microphone carries whichever input Mumble records from. Linux and Windows
don't detect Bluetooth inputs yet and follow the default. Checks in
[audio.rs](../../src/audio.rs):
`system_default_passes_over_a_bluetooth_default`,
`a_bluetooth_default_is_kept_when_nothing_else_is_usable`,
`a_virtual_input_never_stands_in` (CoreAudio transport Virtual, Aggregate, or
AutoAggregate; [#429](https://github.com/anomalyco/hex/issues/429)),
`inputs_resolve_to_the_selection_then_the_system_default`,
`device_override_wins_and_never_falls_back`,
`a_default_change_switches_only_while_capture_is_idle`, and
`a_missing_selection_uses_the_default_until_it_returns`, with injected device
lists and opens; scene `mac-microphone-bluetooth` shows the row and switch. Not
verified: the CoreAudio listeners against a real default change, the transport
read against real AirPods, or Windows' default-change report on a real PC.

**Fixed after 2.1.20:** through 2.1.20 the microphone menu was painted without
occluding the panel, so hovering or clicking a device also reached the setting
under the pointer ([#94](https://github.com/anomalyco/hex/issues/94)), and it
only closed by choosing or reopening. The menu is now anchored to its button,
occludes the controls beneath it, and closes on any outside mouse-down. In the
Rune window the menu is a rune-kit `Menu` in the overlay layer (scene `menus`);
the GPUI occlusion regression went with GPUI and has no Rune counterpart.

**Easy to misread:** an open microphone is not an active recording.

## Process Text With Modes

```ts
Output (main sidebar, replaces the Modes editor) // rune_ui/output.rs; scenes output*
├── Trust prompt -> Allow a config that uses fetch or programs (only when one asks)
├── Edit with agent -> Copy a prompt naming the steps' keys (the AI rewrite too), `mumble settings`,
│   `mumble vocabulary` for names, and the transforms docs
├── Open file -> settings.json in the default editor
├── Every dictation -> Global's steps, numbered, read-only
├── Try -> Global's corrections and built-ins on a typed sentence
│   ├── Scripts and rewriting run only when dictating
│   └── Labs > Fix results in Output (off, #989) -> Vocabulary (the shared sheet) and,
│       with a sentence typed, Fix this: copies Edit with agent's prompt ending
│       `When I say "<sentence>", Mumble pastes "<result>". It should paste: ` (labs.md)
└── In some apps -> Each mode's apps and its steps (Only: …), opens to list them
    // the old Modes editor stays in the code for its scenes, unreachable

Foreground context -> Select processing
├── Matching website                     // macOS: scriptable browsers, then AXWebArea; Windows: UIA; Linux: AT-SPI
├── Otherwise matching application
└── Otherwise Global

Selected mode                            // replaces Global's rewrite and transformations
  -> Its vocabulary, then the shared one   // features/vocabulary.md (vocabulary.modes)
  -> Global's corrections, then the mode's  // every OS
  -> OpenCode transformation             // optional provider-backed rewrite
  -> Transformations                     // ordered text chain
  -> Paste or Send

Stage fails -> Preserve its input        // failed chain discards partial results
```

**Fixed November 1, 2026 (issue 949):** every OpenCode rewrite's fixed
instructions (`PROTOCOL_PROMPT`) now say to keep every number as said (no
dropped, merged, or reformatted numbers, no added thousands separators) and,
unless the mode's prompt says otherwise, to write a spoken enumeration
("point one … point two …") as a numbered list, one item per line. A
transcript of numbers alone (digits or English number words) skips the
rewrite and pastes after corrections as heard; transformations still run.
Check: `numbers_only_transcripts_skip_the_rewrite` in
[dictation/processor.rs](../../src/dictation/processor.rs). The prompt rules
were not run against a live model.

Built-in text transformations run natively. Custom transformations come from
`~/.config/mumble/mumble.config.ts` and run in Mumble's embedded script engine, each
under its own `timeout` (1 s by default, 30 s when the config uses host APIs,
60 s cap). OpenCode rewriting sends documented
context to the configured provider; speech transcription stays local.

```ts
Transform host APIs                      // personal_commands/{io,fetch,process,trust}.rs
├── fetch(url, { method, headers, body, signal }) -> Response (ureq, platform TLS)
├── node:child_process execFile / exec (+ node:util promisify, input option)
├── setTimeout, AbortController, AbortSignal.timeout
└── First use -> Output pane trust prompt (hosts, programs, or capability)
    ├── Allow -> hash of every loaded file pinned in transform-trust.json
    ├── Any edit -> asks again
    └── Not allowed -> call throws -> chain fails -> text pasted unchanged
Timeout or settle -> requests abandoned, process group / job object killed
```

The API is documented for agents at
[anoma.ly/mumble/docs/transforms](../../site/public/docs/transforms.md), linked
from `llms.txt`, the managed `AGENTS.md`, and Edit with agent's prompt. Checks:
[tests/host_apis.rs](../../src/personal_commands/tests/host_apis.rs) runs
`fetch` against a local server (approval, re-ask on edit, abort, network
error) and `exec`/`execFile` (exit codes, stdin `input`, promisify, `timeout`,
`maxBuffer`, process-tree kill on Linux, fallback through dictation's
processing stage); [trust.rs](../../src/personal_commands/trust.rs) covers the
scan and hash; `the_trust_prompt_lists_what_a_config_reaches_until_it_is_allowed`
and the `output-trust` golden cover the prompt. Windows `cmd` quoting is a
pure-function test only; the job object and macOS paths compile on CI but are
not run against real programs there. No check reaches a remote TLS host.

The built-ins are Lowercase, SpongeBob case, and No trailing punctuation. The
last removes only the final contiguous run of sentence marks (including
full-width marks), preserving internal punctuation, trailing whitespace, and
closing quotes/brackets. Punctuation inside a final closing quote is preserved.
Select it after Lowercase for lowercase messages without the final period.
The `no_trailing_punctuation_*` and `built_in_transformations_chain_in_selected_order`
tests in [personal_commands.rs](../../src/personal_commands.rs) cover text behavior
and native dispatch; `transformations_chain_with_built_ins_and_failures_keep_nothing`
and `runaway_scripts_stop_at_the_time_limit_and_the_config_keeps_working` run
real scripts. None is a live target-app paste.

Selection and stage checks start in
[dictation/processor.rs](../../src/dictation/processor.rs),
[dictation/text_replacements.rs](../../src/dictation/text_replacements.rs), and
[personal_commands.rs](../../src/personal_commands.rs). They do not prove live
provider availability or real application/browser context changes.

Website context per OS: macOS reads no page at all until something uses it: a
mode chosen by website, an OpenCode rewrite (its prompt names the site), or a
script transformation (it receives the address)
(`dictation::config::reads_browser`), so dictation's page reads never prompt
without one (media pause still asks browsers when on). Otherwise
it asks the front tab of Safari, Orion, Chrome,
Brave, Edge, Arc, Vivaldi, Opera, Helium, and Chromium over AppleScript (one
Automation prompt per browser; a refusal keeps the application context) and
reads any other app's focused AXWebArea `AXURL` (Firefox, Zen, web views).
Linux matches the focused window from X11 or the Wayland foreign-toplevel list
to an AT-SPI window by title and process and reads its showing web
document's `DocURL` (Firefox) or `URI` (Chromium, only with
`--force-renderer-accessibility`). `linux_context_reads_the_focused_window`
(ignored, manual) printed `google-chrome-stable` with `https://example.com/path`
under headless Sway and `chrome` with the page under its XWayland, and `mumble`
on niri (Oct 30); the macOS adapters are compiled on CI only and not run.

OpenCode transformation talks only to an already running OpenCode service,
found through its registration file ([desktop/opencode_service.rs](../../src/desktop/opencode_service.rs));
Mumble runs no OpenCode executable and starts nothing. The catalog loads only while
the Modes pane shows. Without a service, "OpenCode isn't running" shows beside
the switch and the setup/retry notice below;
`unavailable_opencode_is_explained_beside_each_switch` covers it headlessly, and
the `desktop/opencode_service` tests cover registration, stale-PID, loopback, chunked,
deadline, and cancellation handling against a local fixture, not a live
provider.

`ordinary_processing_snapshots_modes_before_stage_and_keeps_transformations` in
[dictation_worker.rs](../../src/dictation_worker.rs) checks processing-time profile acquisition,
conditional Processing-stage emission, and retained transformation selection.
It uses controlled profiles, not a live provider or custom transformation host.

`modes_add_select_and_delete`, `mode_corrections_and_names_save_from_their_fields`,
and `mode_transformations_add_reorder_and_remove` in
[rune_ui/tests.rs](../../src/rune_ui/tests.rs) cover the Rune mode editor and
its debounced drafts headlessly. They do not exercise native typing or
foreground mode activation.

Application activations compare the picker's bundle name with the foreground
application's localized name. When Finder shows all filename extensions, the
picker name arrives as `Ghostty.app`; [dictation/context.rs](../../src/dictation/context.rs) strips
that suffix on both sides and [app_settings.rs](../../src/app_settings.rs)
rewrites already persisted activations on load, checked by
`application_matching_ignores_finder_bundle_extensions` and
`loading_strips_finder_bundle_extensions_from_mode_applications`.

**Fixed in 2.1.14:** through 2.1.13, a mode saved with that Finder
preference enabled never activates and every dictation falls back to Global.
Update to repair saved selections automatically. For older builds, the workaround
is to quit Mumble, remove the `.app` suffix from
`dictation_processing.modes[].applications` in `settings.json`, then relaunch.

## Experimental: Mumble Microphone And Voice Filter

```ts
Labs › Voice Filter (experimental.mumble_microphone = true), MUMBLE_EXPERIMENTAL_MIC=1,
or the driver installed; off, the pane is hidden  // fun.mumble-microphone; macOS, Linux
├── Sidebar > Voice Filter  // the only home of these controls
├── Mumble Microphone card
│   ├── Linux -> no driver: a PipeWire virtual source (Audio/Source "Mumble Microphone",
│   │   node mumble_microphone) that Mumble plays into through `pw-cat` while the Voice Filter is on
│   │   ├── Status -> Starting… / Available ("Choose Mumble Microphone as your input…") / Unavailable + reason
│   │   ├── pw-cat missing or exits -> Unavailable with its last error line; retries every 2 s
│   │   ├── Off or Mumble quits -> the source disappears (pw-cat ends with its input); no root, nothing installed
│   │   └── Session priority 1 -> WirePlumber never picks it as the default input Mumble records
│   ├── macOS driver status -> Installed v<ver> / In use by <app> + Uninstall
│   │   ├── Version mismatch -> v<old> installed + Update + Uninstall
│   │   ├── Pre-Anomaly com.kitlangton driver at the same path -> Update replaces it in place
│   │   ├── Not installed -> Install -> macOS admin prompt -> copies bundled MumbleMicrophone.driver + restarts coreaudiod
│   │   └── Inline state -> Installing… / Uninstalling… / Cancelled / Failed with reason
│   ├── Mode while dictating -> Muffled (default), Muted, Quiet, Normal, Chipmunk, Robot, Walkie, Underwater, Far
│   ├── Key sliders -> Live mode tuning + Effect strength + Reset
│   └── Try it -> Example clip OR Record 5s (in-memory only)
│       ├── Dual waveform -> You vs Call hears with highlighted dictation span
│       └── Explicit playback -> Play what call hears | Play your voice
├── Live microphone capture -> Raw audio to Mumble transcription (unmodified)
│   ├── Source -> Mumble's dictation microphone (System default follows the OS default, never Mumble Microphone itself)
│   ├── Lock-free ring -> 48 kHz virtual sink (macOS IOProc; Linux 11 ms blocks into a one-page pipe) -> Raised-cosine fade on intentional hold
│   │   └── Not dictating -> bit-exact pass-through; non-48 kHz inputs resample continuously
│   └── Ring underflows/overflows -> process.log each minute while an app reads it, and on sink stop
└── Offline CLI -> mumble dev voice-filter render <in.wav> --mode <m> --dictation <start>-<end> -o <out.wav>
```

Sources: [audio/voice_filter.rs](../../src/audio/voice_filter.rs),
[audio/mumble_microphone.rs](../../src/audio/mumble_microphone.rs),
[MumbleMicrophoneDriver.c](../../macos/MumbleMicrophoneDriver/MumbleMicrophoneDriver.c),
[build-app.sh](../../scripts/build-app.sh),
and [rune_ui/fun.rs](../../src/rune_ui/fun.rs). Checks:
`default_mode_is_muffled`,
`all_nine_modes_render_finite_and_expected_levels`,
`raised_cosine_fade_has_no_step_discontinuity`,
`ring_buffer_transfers_samples_and_handles_underflow_and_overflow`,
`a_second_stream_on_the_same_microphone_does_not_double_the_feed`,
`a_callback_larger_than_the_latency_cap_passes_through_whole`,
`resampling_is_continuous_across_callbacks`,
`only_the_listeners_current_input_feeds_mumble_microphone`,
`driver_plugin_loopback_streams_at_48khz_under_20ms_latency`,
`driver_plist_version_and_status_mismatch_are_detected`,
`osascript_failure_classifies_user_cancel_and_errors`,
`audio_service_restart_interrupts_stream_and_recovers`,
`voice_filter_saves_the_mode_and_applies_the_tuning_live`,
`try_it_records_and_plays_through_the_host`, and
`voice_filter_needs_labs_or_mumble_microphone`.
Scenes: `mac-voice-filter`, `mac-voice-filter-muted`,
`mac-voice-filter-driver-missing`, `voice-filter`, `voice-filter-unavailable`
(Linux).
Linux observed (apollo, PipeWire 1.6): the source lists in `pactl list sources`
and `pw-record --target mumble_microphone` records the fed audio at 48 kHz; no
automated check covers it, and no call app was tried.
Proof limit: in-process `dlopen` loopback proves the compiled driver's 48 kHz
`WriteMix`/`ReadInput` path at 5.33 ms (256 frames), not an interactive macOS
admin authorization prompt or an active Slack/Zoom call. The pass-through
checks simulate callback timing and sizes; clock drift between the microphone
and the virtual device is not corrected, so a skip or zero-fill click every
minute or two remains possible.

## Recover And Maintain

```ts
Cancellation / interruption / failure    // recovery
  -> Scoped stop, fallback, or error
  -> Retry or fresh gesture
  -> Next capture succeeds               // this is the proof, not a cleared error

Menu bar and Settings                    // maintenance
├── Icon -> 18 pt monochrome Mumble mark template
│   └── Labs > Hide menu bar icon (off, #1064) -> hidden; shortcuts keep working;
│       Finder or Spotlight reopen -> Settings (labs.md)
├── Settings / Paste Last -> Reachable with Dock icon hidden
├── Launch at Login -> macOS owns registration state
├── Check for Updates -> The updater (maintain.updates); opens the window, whose
│   footer then says "Checking for updates…", "Mumble is up to date", or
│   "Couldn't check for updates" (#1120)
└── Quit -> Orderly shutdown
```

```ts
Launch Mumble                               // maintain.startup
├── Setup incomplete / permissions or model missing -> Open setup or Settings
├── Show Dock icon on -> Open Settings
└── Show Dock icon off + usable menu-bar item -> No startup window
    ├── Dictation starts independently of the Settings window
    └── Menu-bar Settings / Finder or Spotlight reopen -> Open or focus the window
        └── On the active Space, never switching back to where it last showed

Menu-bar installation fails -> Show Dock icon and open the window // recovery access

Show Dock icon off                       // maintain.dock-policy
├── Settings window open -> Dock icon stays // hiding it would deactivate Mumble
└── Settings window closes -> Menu-bar only
```

```ts
General > Launch at login                // maintain.login-item
├── Open -> Checking until macOS returns the initial state
├── Refresh / toggle / Open settings -> One persistent background worker
│   └── One active request; latest pending user action precedes the next refresh
├── Toggle -> Switch moves at once; macOS's answer settles it
│   ├── Answers behind a later click -> Switch keeps the last click
│   └── Settled answer differs -> Switch springs back
├── Failed toggle -> Show error and reconcile with actual macOS state when available
└── Close Settings -> Release channels without waiting for an active native call
```

[desktop/login_item.rs](../../src/desktop/login_item.rs) serializes ServiceManagement calls on one
window-owned worker, with bounded request and response channels and an autorelease
pool per operation. Settings reads responses without blocking on its 250 ms tick;
refreshes do not accumulate while macOS is busy. The toggle is unavailable until
the first successful status lookup. A click is held in memory
(`PlatformState::login_item_requested`) only until the worker is idle again, so
the switch need not wait for `SMAppService` (about 120 ms per toggle, measured
October 24, 2026, plus up to one tick). macOS owns registration state; no login
Boolean is persisted. Scenes never start the worker or call macOS.

The worker tests in `desktop/login_item.rs` exercise thread reuse, nonblocking polling,
latest-action coalescing, action priority, and closing during a blocked call.
`launch_at_login_asks_the_login_item_and_needs_approval_first` in
[rune_ui/tests/panes.rs](../../src/rune_ui/tests/panes.rs) checks the Rune row's
requests and the approval state, and the `login_item_*` tests there check the
switch while macOS answers: confirmed, refused, a status answer behind a click,
and rapid clicks. The GPUI regression for error rollback and
external enablement went with GPUI. Native `RequiresApproval` and error transitions remain unverified;
the simulated checks do not establish Developer ID signed distribution behavior.

Observed September 23, 2026, on `d1930b5` after the final simplifications: the
full debug suite passed 486 tests and the release suite passed 485, each with
twelve opt-in tests ignored and all twelve keyboard-layout child scenarios
passing. Formatting, strict debug all-target/all-feature Clippy, and diff
whitespace checks passed.

Computer-use checks on an ad-hoc signed temporary bundle of the optimized build
confirmed shortcut capture/cancellation, Settings/Modes navigation, registration
and unregistration, the final state after three consecutive toggle clicks, and
refresh after removing that bundle's login item in macOS Settings. The temporary
item was left disabled/removed; no new microphone, input, or Accessibility grants
were made. A five-second `sample` profile found `SMAppService` frames on the
`login-item` worker and none on the main thread. This is sampled-stack evidence,
not an input-to-frame latency measurement or a signed-release smoke test.

Closing Settings left the app process alive with zero `login-item` threads;
reopening produced one new worker. An earlier apparent surviving worker came
from computer-use observation reopening the window. Cmd+Q and Quit Mumble each
terminated the process. Local profiles are `/tmp/mumble-login-worker-current.sample.txt`
and `/tmp/mumble-login-cycle-{before,closed-no-observe,reobserved}.sample.txt`.
The tested temporary executable's SHA-256 is
`d1a3be1e219e7371726273937eeef6c7e16434637e4297ae183f9bdaf4d28b04`.
No Developer ID signing identity was available for a new distribution build.

The existing **Show Dock icon** preference controls quiet startup; there is no
additional launch-window setting. This applies to normal and login launches,
does not change login registration, and does not hide an already open window
when the preference changes. Any open Settings window keeps the Dock icon: macOS
deactivates an app asynchronously when it becomes menu-bar only, which dropped
the window behind other applications ([#103](https://github.com/anomalyco/hex/issues/103)).
Turning the preference off therefore takes effect when Settings closes.

Closing the window only orders it out, and AppKit keeps an ordered-out window
on the Space it last showed on, so reopening it from the menu bar on another
Space switched back to that one. `macos_rune` gives the window
`NSWindowCollectionBehaviorMoveToActiveSpace`, so it opens on the active
Space instead. No automated check covers Spaces; it needs a native check with
two desktops.

Checks in [macos_rune/tests.rs](../../src/macos_rune/tests.rs):
`dockless_startup_stays_quiet_only_when_setup_and_menu_bar_are_ready` and
`dock_visible_startup_always_opens_the_app` cover the startup decision. The
Dock/Finder reopen Apple event and the status item's `OpenSettings` bypass it.
These checks do not establish signed-app login behavior or native Finder/Spotlight
reopening; those still need an installed-app smoke test. This addresses the
menu-bar-only case in [#64](https://github.com/anomalyco/hex/issues/64), not an
independent window preference for users keeping the Dock icon visible.

**Executed September 3, 2026:** 441 Rust tests passed (nine opt-in tests ignored),
along with all twelve keyboard-layout child scenarios, formatting, and Clippy.
The release Settings preview built and launched, but window screenshot capture
failed; no visual or installed-app startup verification is claimed.

```ts
Mumble checks the release service            // maintain.updates
├── 10 s after launch, hourly (Dev channel: every 10 min and on window focus), and Check for updates -> anoma.ly/release/mumble/desktop/<channel>.json
│   ├── User-Agent: mumble/<build channel>/<version> (mumble/dev/0.0.0-dev-171, mumble/stable/2.3.1); nothing else about the install
│   ├── Failed check -> backs off, never past hourly
│   └── <channel> -> updates.channel (General › Application › Update channel: Stable | Dev,
│       or `mumble settings set updates.channel`), else the channel the build was published on
│       (MUMBLE_BUILD_CHANNEL; older builds: dev for a dev version, else the receipt's);
│       package managers always stable
│       └── Choosing a channel -> checks now; a staged update stays staged
├── Versions never compared: channel's current release != running build -> take it
│   ├── Running build -> Linux/Windows: executable SHA-256 = target's; macOS: same version
│   ├── Another channel than the build's -> sidebar, tray, menu bar: Switch to Stable 2.3.1
│   │   └── Only after the user confirms -> download -> stage (never switches by itself)
│   ├── required -> install without waiting for the rollout
│   └── Rollout -> device-local bucket in <support>/update-bucket, never sent
├── Direct install -> download (resumable) -> size + SHA-256 -> code signature -> stage
│   ├── macOS -> unpack beside Mumble.app -> Developer ID 5NZ4Q7NXJ4, anomaly.mumble, notarized
│   ├── Windows -> Authenticode valid, signer AUTHENTICODE_SIGNER -> new version directory
│   ├── Linux -> SHA-256 only -> new version directory, `current` switched
│   ├── Staged -> checks continue; Restart to update stays offered meanwhile
│   │   ├── Another release this install takes (update, required) -> download -> replaces the staged one
│   │   └── Same release, rollout deferred, unconfirmed switch, rollback, failure -> keep the staged one
│   └── Sidebar Update, menu bar / tray: Restart to update; a dev build lists its
│       first changes and What's new (https://anoma.ly/mumble/dev) under it (updater/notes.rs)
│       ├── One more check first (manifest within 15 s) -> Checking / Downloading -> newest staged
│       │   └── A release published after the relaunch -> the next launch's check
│       └── macOS -> renamex_np swap (also at quit) -> relaunch -> old bundle removed at launch
├── macOS location not replaceable (read-only, translocated, root-owned) -> Download update (DMG)
├── Package manager (pacman, Nix) -> its command, once it serves another version
├── Source build, test build -> never checks
└── Failure -> keep the running version, back off, Check for updates retries

Check for updates pressed (footer, menu bar, app menu)   // maintain.update-feedback (#1120)
├── The menu bar's and app menu's open the window at General
└── Footer -> "Checking for updates…" -> "Mumble is up to date" | "Couldn't check for updates"
    // the hourly check alone keeps "Check for updates"; check:
    // an_asked_update_check_says_how_it_went (fixture; not run on a Mac)

DisableUpdates at launch                 // maintain.managed-updates
├── No checks, downloads, or installs; sidebar: Managed by your organization
├── macOS app menu -> Check for Updates omitted
└── macOS menu bar -> Disabled "Updates Managed by Your Organization"
```

Administrators set `DisableUpdates` on macOS as a Boolean in the `anomaly.mumble`
defaults domain, normally through a configuration profile
([#100](https://github.com/anomalyco/hex/issues/100)), and on Windows as a
non-zero DWORD under `SOFTWARE\Policies\Anomaly\Mumble` in HKLM or HKCU. Mumble
reads it once at launch, so a forced value wins and a change applies after
relaunch. Releases through 2.3.1 read
`com.kitlangton.hex2`; there is no fallback to it, so profiles must name
`anomaly.mumble`.

[updater.rs](../../src/updater.rs) owns the worker and the shared status the
window, menu bar, and tray show; `updater/protocol.rs` passes the release
service's shared test vectors (`tests/fixtures/release-protocol`), and the
per-OS modules have tests for activation, the macOS bundle swap and
signature checks (ad-hoc fixtures against the real requirement), the rollout
bucket, receipts, the channel choice, never comparing versions (a legacy
2.3.2-dev.29 follows 0.0.0-dev-N; dev to stable is a switch), the switch
waiting for confirmation, only a different self-taken release replacing a staged
update (`only_a_different_release_this_install_takes_replaces_a_staged_update`),
Restart to update's final check and one-time report
(`restart_to_update_checks_once_more_and_reports_once`), Linux re-staging keeping
the running version (`replacing_a_staged_update_keeps_the_running_version`), and
Windows Authenticode (a Microsoft-signed system file). No test drives the worker
through a real re-stage and restart; a channel rolled back while an update is
staged still restarts into the staged release. The
Update channel row shows only while this install updates itself (not test,
source, managed, or package-manager installs); the `mac-settings-dev-channel`
scene renders it, and `rune_ui` tests cover saving and hiding it.
Not yet observed: a published dev channel release, a switch between channels on
a real install, a real update on any OS from the release service, the macOS
swap and relaunch of a Developer ID build, a Windows update of a signed build.
Manifests are unsigned; on Linux a compromised HTTPS origin could serve any
executable with a matching manifest, which Linux code signing cannot catch.

**Fixed in 2.1.13:** older releases through 2.1.12 used app-global dispatch from
inside the sidebar click, losing the action while the active window was borrowed.
The click regression passed in debug and optimized release builds. The menu-bar
path was exercised on the installed local 2.1.11 build and opened the native
Software Update window; no installation was performed. That path remains the
workaround for older versions, including after Remind Me Later. A repaired
installed-sidebar click remains unverified.

The [historical release verification](release-verification.md) records publication
evidence through 2.1.20, including signing, artifact checks, CI results, and skipped
native checks. Those release-specific results do not verify the current working
tree, installed-app behavior, or physical Linux compatibility; retain each entry's
stated limits when using it as evidence.

[Recovery](recovery.md) separates supported recovery from known defects.
[desktop/login_item.rs](../../src/desktop/login_item.rs), [macos_rune/status_item.rs](../../src/macos_rune/status_item.rs),
[updater.rs](../../src/updater.rs), the
[identity test](../../scripts/test-app-identity.sh), and
[app validation](../../scripts/validate-app.sh) are maintenance evidence starting
points. Preview badges do not prove installed-app login registration or a signed
update from an older supported build.

### Managed feature locks (macOS)

```ts
Configuration profile forces a key in anomaly.mumble     // maintain.managed-locks
├── DisableOpenCodeProcessing -> no mode rewrites
├── DisableTransforms -> no transforms; the mumble.config.ts host never starts
├── RestrictTransformsToLoopback -> transforms on; fetch reaches loopback only
│   (resolved addresses, every redirect, no proxy); exec/execFile refused
├── DisableLabs -> every Labs feature off
└── Window: setting off, disabled, "Managed by your organization";
    mumble settings set / mumble labs on / local API refuse with code "managed"
```

Only values a profile forces count (`CFPreferencesAppValueIsForced`), read once
per process; a `defaults write` value locks nothing. Windows and Linux read
none of these keys. [policy.rs](../../src/policy.rs) owns the keys:
`AppSettings` turns locked switches off as it loads and writes the saved values
back on save, so the person's own settings return when the profile goes away.
Work that follows a list rather than a switch checks the policy where it
starts: `Profile::configured` (rewrites, transformations), OpenCode generation
in `dictation/processor.rs`,
`PersonalCommands::start`, and `personal_commands/script.rs` with
`net::loopback_agent`. Admin docs: `site/public/docs/admin.md`
(https://anoma.ly/mumble/docs/admin).

Checks: `policy::tests`, `a_managed_lock_reads_off_and_never_rewrites_the_saved_switch`,
`a_managed_setting_is_refused`, `managed_locks_drop_the_rewrite_and_transformations`,
`switches_hold_every_feature`, and
`restricted_to_loopback_fetch_stays_on_this_computer_and_runs_no_programs` (a
redirect off the computer and a non-loopback address both fail before
connecting). Not yet observed: a real configuration profile on a managed Mac,
or the managed rows in the window.

## Other Platforms And Consumers

```ts
Linux beta                               // not macOS feature parity
├── mumble start -> One per-user Mumble -> Alt-Space capture -> Stream to the cloud service -> Paste
├── mumble / mumble app -> Window in the same process; closing it leaves Mumble running
│   └── Ctrl+Q in the window or Quit Mumble in the sidebar -> quits Mumble (#808)
│       // check: ctrl_q_quits_mumble (fixture; not run on a GNOME session)
├── Recording sounds -> Shortcut press / capture stop
├── Shortcuts -> One state machine, three sources        // linux.shortcuts
│   ├── X11 -> Root-window key grabs; General records the chord
│   ├── Wayland + GlobalShortcuts portal -> Desktop grants the triggers
│   │   ├── General > Change shortcut -> Press chord (Esc cancels)
│   │   │   └── Saved -> New portal session asks for it as preferred_trigger
│   │   │       under an id named for it (`dictation-CTRL-ALT-space`), so a desktop
│   │   │       that keeps an id's stored trigger (GNOME, KDE) binds the new one (#575);
│   │   │       an unchanged shortcut keeps its id and any trigger set in the desktop
│   │   │       // observed on the niri fork: a new session gets it, a rebind in the same one fails
│   │   │       // #575 fix checked by a unit test only, not on a GNOME session
│   │   ├── Each row: "Your desktop granted <trigger>" (it may assign another);
│   │   │   an empty trigger (KDE's for one it keeps unbound) counts as refused
│   │   │   └── Hyprland (XDG_CURRENT_DESKTOP) -> its portal names no trigger, since
│   │   │       its config binds them: muted "Your desktop assigns this key itself,
│   │   │       so Mumble can't confirm it", no refusal note, info log (#635)
│   │   │       // check: a_missing_trigger_is_unconfirmed_where_the_desktop_assigns_keys
│   │   │       // (the row's data only); not run on a Hyprland session
│   │   ├── Portal version 2 -> Desktop shortcuts > Open -> ConfigureShortcuts,
│   │   │   the desktop's own dialog; ShortcutsChanged updates each row
│   │   │   ├── The call fails (the frontend says version 2 but the desktop's backend,
│   │   │   │   such as GNOME's, lacks it) -> the row goes away instead of a dead
│   │   │   │   Open button, and process.log warns (#723)
│   │   │   │   // no check; not run on a GNOME session
│   │   │   // a desktop keeps a stored binding over a new preferred_trigger for the same id
│   │   ├── Refused shortcut -> Danger row + how to free it (niri: config.kdl)
│   │   │   under that shortcut's own row (Dictation, Output);
│   │   │   the rest keep working
│   │   └── The portal's backend fails the bind (Response 2 or a bus error, not
│   │       a dismissal; niri with the GNOME backend, #1014) -> as without the
│   │       portal below; this listener run doesn't bind again, a restart
│   │       (Enable, a settings change) tries once more
│   │       // check: only_backend_failures_count_as_a_missing_portal (the
│   │       // classification only); not run on niri
│   ├── Wayland without the portal -> Bind `mumble` commands in the desktop
│   │   ├── General hides shortcut rows and lists the commands
│   │   └── GNOME: Add GNOME keyboard shortcuts -> custom shortcuts running
│   │       `mumble dictation toggle` (#781); written only while the listener has no
│   │       portal, and removed once the portal (or X11) binds the shortcuts, so a
│   │       press never both starts a portal hold and toggles it off (#931)
│   │       // check: entries_exist_only_without_the_portal (the rule only)
│   └── Listener starting or stopped -> `mumble status` reports Unregistered,
│       never the X11 mode
├── mumble dictation start|stop|toggle, mumble paste-last
│   -> Same events as a shortcut                     // linux.commands
│   ├── start|toggle --raw -> Dictate without processing's events
│   ├── Idempotent: start while recording / stop while idle -> Nothing
│   └── Mumble missing or not listening -> One-line error, non-zero exit
│       // as for mumble quit|restart|status
├── mumble dictation cancel -> Discards whatever records, any mode
├── Toggle's cancel shortcut (Escape by default), only while recording
│   ├── X11 -> Grabbed only for the recording
│   └── Portal -> Never bound: bind `mumble dictation cancel` in the desktop
│       └── KDE startup -> before the bind, the pre-#579 `mumble` set (with the
│           pre-#492 `cancel` on Escape), and every action under `anomaly.mumble`
│           this bind doesn't ask for (`cancel`, `cancel-dictation`, and the ids
│           of shortcuts changed since, which kept their keys so the new id could
│           never take them, #898) are removed from kglobalaccel (`unregister`);
│           other apps untouched
│           // check: only_requested_shortcuts_are_shown (a stale grant stays
│           // out), kde_actions_mumble_no_longer_binds_are_removed (the list only);
│           // the KDE removal is not run on a Plasma session
│       └── KDE's approval dialog -> Mumble waits for it up to 30 min, not 60 s:
│           giving up closed the session, and KDE then kept every shortcut
│           with no key and never asked again (#898). A bind left unapproved
│           (dismissed, canceled, abandoned) is noted in
│           `kde-unapproved-shortcuts`; the next bind removes those ids while
│           they still have no key, so the dialog asks again; a key set in
│           System Settings since stays
│           // checked in a headless KWin 6.8 session (xdg-desktop-portal-kde
│           // 6.8.2): the dialog stays up with Mumble "Enabling…"; with the note,
│           // keyless ids are removed and the dialog returns; without it they
│           // stay; Mumble is listed under Applications in System Settings ›
│           // Shortcuts. Approving with OK was not exercised (no input there)
├── Recording HUD -> Dot scope, as on macOS, on the
│   │   top or bottom edge, recording with the voice's level, then transcribing
│   │   until accepted output finishes; Hidden position -> none   // linux.hud
│   ├── Display -> picked at each capture start (#486)
│   │   ├── X11 -> the monitor under the pointer (XRandR monitors); while shown,
│   │   │   checked every 250 ms; moves once two checks agree (#807)
│   │   ├── Layer-shell (niri, Sway, Hyprland) -> no output pinned: the
│   │   │   compositor's focused output
│   │   └── GNOME, KDE (XDG_CURRENT_DESKTOP) -> the primary display
│   └── Rune overlay in Mumble's process (rune_hud, rune_hud.wgsl on the GPU)
│       ├── Wayland -> layer-shell overlay layer, no keyboard, empty input region
│       ├── X11 -> override-redirect window, empty input shape
│       ├── Wayland without layer-shell (zwlr_layer_shell_v1; GNOME) and XWayland
│       │   (`DISPLAY`) -> only the HUD opens on XWayland: override-redirect ARGB
│       │   window, notification type, empty input shape, primary monitor's edge,
│       │   scale from Xft.dpi; the rest of Mumble (window, portal shortcuts) stays
│       │   on Wayland (#465)
│       │   // seen in headless mutter 51 + Xwayland on apollo: HUD at the top
│       │   // centre, animating, focus unchanged, the pointer over it hits the
│       │   // window below; not yet on a real GNOME Shell session
│       │   ├── Drawn on the compositor's GPU only, opaque when its driver offers
│       │   │   no translucent alpha (NVIDIA), never moved to an iGPU or lavapipe
│       │   │   that can't make its swapchain (#979, #982)
│       │   └── Surface refuses its configuration -> that overlay alone fails with
│       │       wgpu's reason (Live text or the HUD); the window and other overlays
│       │       keep running; the next show tries again
│       │       // checked with a noop-backend Rune test; not on NVIDIA hardware
│       ├── Window's Rune app panics anyway -> `mumble window show` errors and
│       │   `mumble status` reports the window and HUD stopped, with the reason (#979)
│       ├── Neither (Rune refused it, or no layer-shell and no XWayland)
│       │   -> process.log warns once; `mumble status` reports `hud_limitation`
│       │   and the listener warning
│       └── No window loop (no GPU or display, `mumble dictate`) -> no HUD
├── Tray host (StatusNotifierWatcher) -> Tray icon: click opens the window; menu as Windows' (tray_menu, linux/tray, rune::Tray)
│   └── No tray host -> No icon; status and sounds remain available
├── Microphone -> System default or a saved device; `--device` overrides
│   └── System default -> ALSA's default, resolved at each stream open // no live follow
├── General > Launch at login -> writes or removes ~/.config/autostart/anomaly.mumble.desktop (X-Mumble-Managed; linux/login_item)
│   ├── Background portal (GNOME, KDE; xdg-desktop-portal 1.20+) -> the portal writes it (X-XDP-Autostart=anomaly.mumble; linux/background)
│   ├── Installed by mise or asdf -> starts the version manager's shim, if it runs Mumble from home
│   └── An entry without the mark -> error beside the row; Mumble never replaces it
├── Microphone failure -> Same recovery as macOS (RecoveringAudioInput)
│   ├── Stream fails -> Cancel an unfinished capture, "reopening" failure, retry with backoff
│   ├── Startup open fails -> Retry with backoff; the listener keeps running
│   └── Released while idle, on-demand open fails -> Failed entry; stays released
│   └── Overload (Xrun) glitch -> Logged; not a failure
├── Paste -> Restore the previous clipboard 500 ms after the shortcut  // linux.clipboard-restore
│   ├── Before the transcript -> snapshot every format (32 MiB, 1 s); empty -> cleared again
│   ├── Later paste, copy, or another client replaced the transcript -> left alone
│   ├── Over the cap or unreadable -> transcript stays; the log says why
│   ├── Paste only reached the clipboard -> transcript stays for Ctrl+V
│   └── Wayland without ext- or wlr-data-control -> transcript stays (KWin offers ext, #719)
│       ├── Settings › Output -> note pointing to Labs › Restore clipboard on GNOME while it is off
│       └── Labs `gnome-clipboard-restore` on and this user's Xwayland already running
│           -> snapshot, check, and restore through XWayland's CLIPBOARD (#1111)
│           // never connects first, so never starts XWayland; checks: xwayland_* in
│           // linux/clipboard.rs, gnome_desktops_point_to_the_clipboard_lab_until_it_is_on;
│           // not run on GNOME
│   └── Wayland: wl-copy, then the shortcut, chosen at the first paste  // linux.wayland-paste
│       ├── Asked by a modifier shortcut -> wait for its portal release + 250 ms
│       │   (2 s cap without a release); after a mumble command -> 250 ms
│       │   // Hyprland merges held physical modifiers into virtual-keyboard keys (#353)
│       ├── Compositor offers zwp_virtual_keyboard_manager_v1 and wtype is installed -> wtype
│       │   └── V sent on evdev KEY_V (placeholder releases first), so Hyprland's
│       │       binds read Ctrl(+Shift)+V, not Ctrl(+Shift)+Escape (#444)
│       ├── Otherwise -> RemoteDesktop portal: keyboard-only session, approved once
│       │   (onboarding or Settings can ask first, `setup.paste-access`)
│       │   (restore token, persist_mode 2) -> Ctrl+V as keysyms
│       │   ├── 15 s after the last paste -> session closes when its grant restores, so
│       │   │   GNOME's remote-control indicator shows only around pastes (#718); a grant
│       │   │   that can't restore keeps its session
│       │   ├── Session showed the dialog -> 250 ms for it to hand focus back
│       │   └── Restored or open session -> 150 ms after wl-copy (minus the reopen), so the target holds the new
│       │       selection before Ctrl+V (GNOME pasted nothing or the old clipboard, #578)
│       │       // fixed delay, no acknowledgement; not observed on a real GNOME session
│       └── Neither works -> Failed: "Copied to the clipboard; press Ctrl+V…" naming
│           the RemoteDesktop portal; the clipboard keeps the transcript
└── Portal identity -> Register as `anomaly.mumble` (reverse-DNS: GNOME 49/50's
    shortcuts provider refused the bare `mumble`, so binding failed, #579)
    ├── Installs ship anomaly.mumble.desktop (StartupWMClass=anomaly.mumble); windows
    │   report app_id / WM_CLASS anomaly.mumble -> one launcher entry
    ├── Icon=anomaly.mumble -> packages install hicolor 16–512 px PNGs (deb/rpm refresh
    │   the icon cache); otherwise Mumble writes them to ~/.local/share/icons/hicolor
    │   at startup and fixes Icon= in managed entries it wrote (#652)
    ├── Startup -> Mumble's own managed mumble.desktop (launcher, autostart) renamed
    │   to anomaly.mumble.desktop; the installer does the same; unmanaged ones kept
    └── No anomaly.mumble.desktop in XDG data dirs -> Mumble writes
        ~/.local/share/applications/anomaly.mumble.desktop (X-Mumble-Managed),
        NoDisplay when an older package's mumble.desktop already shows Mumble,
        removed once a package ships anomaly.mumble.desktop; never rewritten while
        its program is there; pointed at this Mumble (or a version manager's
        shim) once its program is gone or superseded by that shim (#810)
        // KDE bindings and RemoteDesktop grants stored under `mumble` are asked again once
        // checked by unit tests only, not on a GNOME 49 session
GNOME portal shortcuts (#723)            // linux.gnome-shortcuts
├── New GNOME or KDE Plasma setup -> dictation default Ctrl+Alt+Space (GNOME's
│   window menu and Plasma's KRunner keep Alt+Space; #898)
│   // checked in a headless KWin 6.8 session: the portal's dialog flags Alt+Space
├── Granted key GNOME keeps (Alt+Space, Super+Space, Alt+Tab, Print, …) -> red note under the row
└── Keys changed in GNOME Settings -> ShortcutsChanged -> pressing them dictates at once;
    the row note, the Status hint and `mumble status --json` listener.shortcuts show them
    // observed on apollo: gnome-control-center 51.1 provider, portal-gnome 50, stand-in Shell
Enable / Disable (General, tray)         // linux.listener-persist
├── Saves enable_on_launch -> a disabled Mumble stays disabled after a restart (#575)
└── GNOME (XDG_CURRENT_DESKTOP) Disable -> after the listener's session closes, a
    fresh session binds no shortcuts: GNOME Settings stores that empty set, so
    Mumble's entries leave its list; Enable shows GNOME's approval dialog again
    // never on KDE: an empty bind drops the user's keys (#482)
GNOME startup -> pre-#579 `mumble` set listed in GNOME Settings -> removed through
    gsettings (reset its shortcuts, drop it from `applications`); other apps untouched
    // observed on apollo with gnome-control-center 51.1's provider, portal-gnome 50,
    // a stand-in org.gnome.Shell, not a full GNOME session
```

Clipboard restoration lives in [linux/paste.rs](../../src/linux/paste.rs)
(generation and timing) and [linux/clipboard.rs](../../src/linux/clipboard.rs)
(snapshot and serving). Observed on apollo (2026-10-29), in nested headless Sway:
`native_wayland_clipboard_shortcut` restores a two-format clipboard and keeps an
empty one empty; a full dictation from a file-fed ALSA microphone pasted into a
GTK target, then Ctrl+V pasted the original text; an X11 round trip on Xwayland
restored a 3 MB image through INCR. Not observed on X11 dictation, KDE, or
Hyprland.
The Wayland paste choice lives in [linux/paste.rs](../../src/linux/paste.rs) and
[linux/remote_desktop.rs](../../src/linux/remote_desktop.rs). Observed: the
wtype path in the headless-Sway smoke and the virtual-keyboard probe on niri
(apollo). niri's portal frontend advertises RemoteDesktop but its GNOME
backend fails CreateSession, so wtype stays first there. The RemoteDesktop
paste itself (KWin, Mutter) has unit tests for its key sequence only and is not
yet observed on a real desktop. The modifier wait (`ShortcutKeys` in
[linux/input.rs](../../src/linux/input.rs)) has unit tests for its timing only;
it is not yet observed on Hyprland, and a modifier held past the wait still
changes the paste. wtype numbers its keymap from evdev code 1 in order of first
use, and Hyprland matches binds through its configured layout, so a bare
`-k v` read as Escape and a Ctrl+Shift+Escape bind swallowed Terminal paste
([#444](https://github.com/anomalyco/hex/issues/444)). Mumble now releases 46
never-pressed placeholder keysyms first so V lands on `KEY_V`;
`wtype_sends_v_on_the_v_key` checks the numbering against wtype's rule, not a
real Hyprland session.

```ts
Windows beta                             // x64 and ARM64
├── Install -> Setup.exe or install-windows.ps1 -> mumble.exe + install.json receipt
├── Launch -> DLLs load from System32 and Mumble's folder only, never PATH (#587)
│   └── DX12 compiles shaders with FXC (System32), never a dxcompiler.dll on PATH
├── GPU -> Rune draws with DX12 only (Vulkan only without a DX12 adapter) on
│   the lowest-power adapter; a GPU whose device fails hands the window to the
│   next adapter (#662)
│   └── A launch that crashed before its first frame (gpu-draw.pending left in
│       the data folder, same version; cleared by a window or HUD frame or a
│       clean quit) -> high-performance GPU (WGPU_POWER_PREF=high), warned in
│       process.log, until Mumble updates or the file is deleted
├── Present cadence -> after any change the window presents every refresh for
│   1 s, then rests: windowed G-SYNC/FreeSync sees one rate change per
│   interaction, not a swing per hover (#689; Rune e2ea18e4; not run on Windows)
├── Mumble app -> General, tray, HUD; Enabled = shortcut works in every app
├── HUD display -> the monitor under the pointer (GetCursorPos) at each capture
│   start, inside that monitor's work area, at its own DPI (#486); while
│   shown, checked every 250 ms; moves once two checks agree (#807)
├── Launch again -> Running app shows its window; hidden (login) launch -> no-op
├── Tray icon -> White Mumble mark; light taskbar (SystemUsesLightTheme) -> black mark
│   └── Theme read when the tray is created; a later switch applies at the next launch
├── Tray menu (tray_menu, shared with Linux)    // maintain.tray
│   ├── Recording / Transcribing… status line and tooltip while the listener works
│   ├── Show Mumble · Paste last dictation (refocuses the app the menu covered)
│   └── Update item · Enable / Disable Mumble · Quit Mumble
├── Single-modifier shortcut -> Left / Either / Right side picker, as macOS
├── Recordable keys -> letters, digits, F1–F24, named keys, mouse buttons (#794)
│   ├── Punctuation -> saved by virtual key (`oem 1`…`oem 102`), so a layout
│   │   switch keeps it; every keycap (Settings chips, recorder, tray) shows
│   │   what it types on the layout now (`Ù`, `HotkeyKey::keycap`, #815)
│   ├── Numpad -> `Num 1`, `Num *`, `Num Enter`, apart from the top row and
│   │   matched by scan code + extended flag, so Num Lock off still matches;
│   │   the recorder runs its own keyboard hook for them (`NumpadWatch`), so
│   │   Num Lock off records Num 8, not Up (#815)
│   ├── Menu, Pause, Scroll Lock -> may stand alone
│   ├── Insert, Home, End, Page Up, Page Down alone -> Labs › Special keys
│   │   alone; off, a saved one goes back to its default
│   └── Any other key -> the recorder says "Unsupported key" and starts over
├── Ctrl+Space -> Hold or Toggle -> Capture -> Stream (cloud) -> Paste
├── Toggle's cancel shortcut (Escape by default) -> Discards a toggled recording;
│   the hook takes it only while one records. Otherwise Escape -> Nothing (#786)
├── One modifier alone (Right Alt, Right Ctrl) -> Shortcut; Alt/Win release masked (no menu, Start)
├── Microphone switched off in Settings › Privacy -> Onboarding Microphone row -> Allow opens it
├── Microphone -> System default or a saved device
│   └── Windows default input changes -> Switch once capture is idle // cpal's default-device report
├── Paste -> Waits for Shift, Alt, and Win; a held Ctrl is kept and harmless
│   └── Ctrl+V carries scan codes from the foreground layout, so Chromium apps
│       see KeyboardEvent.code: VS Code's terminal takes it as Paste instead of
│       sending ^V to a TUI inside it (#618; from VS Code's source, not run)
├── Update staged (managed install) -> Tray, sidebar footer: Restart to update
├── Another release (unmanaged install) -> Tray, sidebar footer: Download update
├── Another channel chosen -> Tray, sidebar footer: Switch to Stable 2.3.1 (confirm)
├── Otherwise -> Sidebar footer: Check for updates
└── Modes, History, Activity, transformations -> as on macOS
```

Windows updates follow `maintain.updates` ([updater.rs](../../src/updater.rs),
`updater/windows.rs`). CI test builds never check. The tray item and tooltip
([windows/rune.rs](../../src/windows/rune.rs)) follow the published update
state. They pass the cross-compiled Clippy check, but no one has clicked them
on a real Windows PC. The white tray mark vanished on a light taskbar
([#460](https://github.com/anomalyco/hex/issues/460)); a light taskbar now gets
the same mark in black (`light_taskbar_gets_a_dark_mark_with_the_same_shape`
checks the pixels, not a real taskbar). Releases are Authenticode-signed once Azure Artifact
Signing is configured ([windows-signing.md](../windows-signing.md)).

**Known defect, outside Mumble:** Discord's game detection matches the running
`mumble.exe` to *HEX: Shards of Fate*
([#445](https://github.com/anomalyco/hex/issues/445)). The executable name stays
as it is because the managed install layout, updater, shortcut, and login entry
use it. The workaround is to hide the entry in Discord's Registered Games
([Windows guide](../windows.md#known-issues)). No check covers it.

No Windows local transcription API. See the
[Windows guide](../windows.md) for the parity table, streaming notes, and
release flow; [check-windows](../../.github/workflows/check-windows.yml) builds,
lints, and tests x64 and builds and tests ARM64 on every pull request. Physical
smoke tests on Windows 10 and on ARM64 hardware have not been run.

```ts
Linux updates                            // maintain.linux-updates
├── Direct install (versions/ + install.json receipt) -> release service -> staged -> sidebar footer: Restart to update
├── Source install (source-build marker) -> Sidebar footer: Updated from your source checkout
├── Local build (MUMBLE_LOCAL_BUILD=1) -> Sidebar footer: Local build · updates off; Version 2.3.1 (local <commit>); no checks, no channel row
├── pacman (AUR mumble-bin: /usr/bin, root-owned) or Nix store -> Sidebar footer: Updated by your package manager
└── Anything else (a downloaded executable) -> Sidebar footer: Download update
```

The install kind comes from `updater/linux.rs`. The 2026-10-23 apollo
observation (a staged update and the sidebar button) predates the release
service and is not evidence for it.

Wayland shortcuts need either the desktop's GlobalShortcuts portal
or the user's own keybindings for the `mumble` commands; Mumble reads no input
devices. Click-through and a physical portal or keybinding press still need
native evidence. Its [smoke](../../scripts/test-wayland-paste.sh) explicitly
isolates `MUMBLE_APPLICATION_SUPPORT_DIR`, pastes while another keyboard holds
Ctrl and Alt, opens the window through a second launch, and checks that
closing it leaves the same process answering.

Shortcut rules are pure and tested once for every source in
[linux/input.rs](../../src/linux/input.rs): hold, double-tap lock, toggle,
double-tap only, Escape ending nothing, Paste Last, and command idempotence,
plus X11 auto-repeat filtering and a portal release that never arrives
(Hyprland's Lua binds drop it when the modifier comes up first; see
[dictation.md](dictation.md)). The X11 grab tests run on an isolated display in
CI; `commands_and_grabs_share_the_x11_listener` drives grabs and commands
through one listener. [scripts/test-linux-service.py](../../scripts/test-linux-service.py)
checks that commands fail without a listening Mumble.
On 2026-10-22 the portal path was observed on niri with xdg-desktop-portal-gnome
using synthetic uinput keys against an isolated service: granted triggers,
hold and Escape while held (Escape no longer cancels since #786), double-tap lock, Paste Last, and commands. A
physical keyboard press has not been recorded.

`linux.hud`: [linux/rune_hud.rs](../../src/linux/rune_hud.rs) keeps the
listener's generation slot (an old listener cannot hide a restarted HUD) and
turns its reports into [rune_hud.rs](../../src/rune_hud.rs)'s events;
reports before the window's app exists are dropped. The shaders render
offscreen with `cargo test --bin mumble rune_hud_frames -- --ignored`
(2026-10-28 on apollo: Dot scope frames match the Metal look; 2026-10-29 on
apollo: recording, processing, and done after Dot scope became the default).
The GTK HUD it replaced is deleted. The text pill was observed on
2026-10-22 in a headless sway on apollo (isolated data root, real microphone,
`mumble service`): `mumble dictation start` showed the pill at the top edge and it
was gone after `mumble dictation stop` and transcription. The Rune overlay itself
was also observed passing a virtual-pointer click through to the window below
and leaving keyboard focus where it was (Rune `examples/background`). Not
observed: X11 with Mumble, niri, KWin, Mutter.

See the [Linux guide](../linux.md), [linux/app.rs](../../src/linux/app.rs),
[linux/portal.rs](../../src/linux/portal.rs), and
[Linux CI](../../.github/workflows/check-linux.yml) for existing checks.
The [direct installer/updater](../../src/updater/linux.rs) and
[installer tests](../../scripts/test-install-linux-release.sh) do not establish
a published, supported-host-validated signed release. [Nix](../nix.md) owns its
package updates; Mumble updates only managed direct installs. Its Home Manager
module merges `programs.mumble.settings` into Mumble on every activation through
`mumble settings apply` ([control-cli.md](control-cli.md)); `nix/checks.nix`
evaluates the module, not a live activation.

```ts
Linux process                            // maintain.linux-service
├── mumble / mumble app -> Mumble running? -> Activate{activation token} over the socket -> exit 0
│   ├── No Mumble, mumble.service loaded -> session.env -> systemctl start (Type=notify) -> Activate
│   ├── No Mumble, no unit or custom data dir -> This process becomes Mumble
│   │   └── Custom data dir -> Keep the supplied desktop environment; no user-manager query
│   │       or Wayland socket discovery (unless MUMBLE_SYSTEMD_UNIT explicitly opts in)
│   └── Two racing launches -> One Mumble; the other forwards and exits 0
├── mumble start / desktop autostart / mumble service -> Same, without the window
├── mumble dictation|paste-last -> Socket worker -> Listener queue (never the UI thread)
├── mumble status -> Published status; mumble quit / Quit Mumble / launcher action -> Drain, exit 0
├── Open the window -> Maps at 980×760 (min = max), tiling WMs float it -> Resizable from 860×560
├── Close the window -> Keep microphone, accepted jobs, and HUD
│   └── Uncommitted shortcut capture -> Cancel and restore the prior listening state
├── Crash -> systemd Restart=on-failure -> Mumble without a window
├── Update -> Restart the unit -> Mumble reopens the window
├── mumble restart without a display (SSH) -> Keep the saved session.env
├── Saved XAUTHORITY file gone (a reboot: Xwayland's cookie is new each login)
│   -> Take the user manager's current XAUTHORITY, else drop it (#465)
└── Window fails (no display, no GPU) -> Logged; Mumble keeps running
```

Sources: the process, host, and window seam in [linux/app.rs](../../src/linux/app.rs),
the single-instance socket and systemd lifecycle in
[linux/service.rs](../../src/linux/service.rs), the CLI in
[linux.rs](../../src/linux.rs), and the managed installer/Nix user units.
`mumble status` queries the running Mumble; `--lines` explicitly reads historical
observations. `mumble app --hidden` and `mumble start` start Mumble without the window;
`mumble stop` equals `mumble quit`. Closing the window does not mean Disable; the
General's listener card Enable/Disable controls the listener, Quit Mumble the process. No
root daemon or new input grants. The window runs on Rune's app lifetime: it
opens or focuses with the forwarded activation token and keeps its state
across close and reopen. Observed in a headless sway on apollo on 2026-10-22
with [test-wayland-paste.sh](../../scripts/test-wayland-paste.sh), then with
GPUI and Rune builds (open in the same process, close, still answering,
reopen, quit); not rerun since GPUI was removed. On niri
(apollo, 2026-10-22, isolated data directory, listener off) launches spawned
through `niri msg action spawn` opened the window in the instance's process,
`close-window` left the same process answering, a relaunch opened a new
window in it, and `mumble quit` exited; the window never took focus although niri
passed `XDG_ACTIVATION_TOKEN` (an IPC spawn's token, without user input, so
this does not show whether a launcher's token focuses it). Not yet on KWin,
Mutter, or i3.

Listener starts consume the host's settings snapshot; the foreground
`mumble dictate` command loads its own snapshot once. Invalid startup settings leave
defaults available only for editing and block capture until a successful settings
commit or a restart with valid settings. Dismissing the load error does
not authorize recording with defaults. Feedback is initialized at these entry
points, so a delayed listener start cannot reset a newer live volume selection.

Checks: [test-linux-service.py](../../scripts/test-linux-service.py) exercises
the real socket and process lifetime without a display or
audio device; it passed on apollo (Arch, niri) on 2026-10-22.
It also checks custom-root isolation with a stale Xauthority path and an
answering Wayland socket nearby: no `systemctl` call or adopted display.
`closing_the_window_cancels_only_an_uncommitted_capture`,
`commands_route_only_to_a_started_listener`, and
`adopting_saved_settings_restarts_only_for_listener_settings` use controlled
runtime workers. The virtual-microphone and Wayland scripts open the window
through a second launch; the Wayland script passed on apollo (headless sway,
`WLR_RENDERER=gles2`, before GPUI was removed), the X11 virtual-microphone script
has not run since the single-process change. A throwaway `Type=notify` user unit
on apollo (`MUMBLE_SYSTEMD_UNIT`) started ready through `mumble start`, restarted
after SIGABRT and SIGKILL without a window, stopped cleanly on
`systemctl stop` and `mumble quit`, and an older `Type=simple` unit still started.
These checks do not establish a physical desktop logout, cross-version signed
update, or microphone/target-app success on the user's devices.

`malformed_settings_cannot_start_even_after_dismissing_the_error` in
[linux/app.rs](../../src/linux/app.rs) checks the failure gate and recovery after a
simulated successful settings commit; it passed on apollo on 2026-10-22
beside the live service.
`listener_startup_ignores_the_previous_session_until_a_new_one_arrives` checks the
listener owner's startup observation and stop signal; the exact production owner
and test passed in an isolated standard-library-only Rust harness on macOS. That
harness does not compile the Linux application or prove its runtime integration;
the test has **not been executed on Linux**. These checks do not exercise physical
devices or persistence failure on disk.

The [September 4 service-build evidence](release-verification.md#linux-service-build)
records an Arch/i3 client-independent ownership check, not transcription accuracy
or physical logout, Wayland, or signed cross-version update proof.

```ts
Linux General > Sound volume              // dictate.feedback.linux; X11 + Wayland
├── Default / older settings -> 50%
├── Off -> No recording tones
└── 25% / 50% / 75% / 100% -> Save and apply live; preview the start sound
Linux General > Sounds                    // dictate.sound-theme
└── Signature / Classic -> Save and apply live; preview the start sound

Capture                                  // mumble app and mumble dictate
├── Shortcut starts capture -> Queue start sound immediately, before audio arrives
├── Hold passes 300 ms -> While dictating mute/pause begins, never at the press
├── Finish retained recording -> Stop sound at the release, while the 200 ms
│   tail is still captured; a muted output unmutes first   // dictate.stop-tone
│   └── Clip turns out near-silent or speechless -> Stop sound already played,
│       nothing pasted, no error sound or note; the log has its level
├── Brief discarded tap -> Start sound, but no stop sound or transcription
└── Dictation pasted -> Done sound; failures play the error sound
```

The shared [feedback player](../../src/feedback.rs) uses Mumble's bundled sounds
([Feedback](dictation.md#feedback)) on every platform; only macOS wake/sleep
use `afplay`.
Decoding happens before the Linux capture loop; the output device opens for
the first tone and is released five minutes after the last one, as described
in [macOS feedback](dictation.md#feedback). Playback admission is bounded and
nonblocking. Output preparation failures are
logged without blocking dictation. The Settings window saves the volume to
`settings.json` and the service reloads it without restarting the listener. The start sound does not wait
for the 300 ms capture-retention threshold; input delivery and audio-device
latency still apply. **While dictating** mute or pause does: the audio loop
requests it when the capture's audio reaches 300 ms past the press, and not
during a release tail. Until 2026-10-22 the service muted at the press. This
ordering is checked by reading the loop, not by a test or a muted device.

Checks: `bundled_sounds_decode_without_an_audio_device` and
`feedback_admission_never_waits_for_playback` in the player.
`start_sound_precedes_audio_and_does_not_change_short_tap_discard` in
[linux/dictation.rs](../../src/linux/dictation.rs) exercises the production start
path with a sound spy: immediate feedback without any audio, while a 100 ms tap
still discards. Existing timing checks in [dictation.rs](../../src/dictation.rs)
cover the unchanged intentional-hold boundary. These are not proof of audible
native playback or physical hotkey-to-sound timing.

The [September 4 sound-build evidence](release-verification.md#linux-recording-sounds-builds)
records the initial and immediate-start builds. It does not establish audible
native playback, measured physical key-to-sound latency, or Wayland UI behavior.

Both Linux CI triggers include `tests/**`, and the
[Nix source fileset](../../nix/package.nix) includes those sources. The
`keyboard_layout` harness skips on Linux; packaging it does not provide macOS
layout evidence.

```ts
Local API dictation (/dictations)          // macOS, Linux
├── Running desktop capture API -> Separate microphone ownership protocol
│   ├── No silence gate                  // model output, unlike hotkey dictation;
│   │                                     // only recognition-hint echoes are dropped
│   ├── Linux -> The listener records it (dictation::service owner)
│   │   ├── Shortcut or `mumble dictation` records -> 409 dictation-busy
│   │   ├── Client's dictation records -> shortcut starts and ends nothing
│   │   ├── Lapsed lease or owner's cancel -> only that dictation is dropped
│   │   └── Microphone lost -> finish answers microphone-lost (no kept clip)
│   └── Windows -> serviceCapture false, 404 service-capture-unavailable
└── Text -> The owner's /finish; nothing pastes
```

Linux and Windows serve the same local API from the running app
([linux/app.rs](../../src/linux/app.rs), [windows/rune.rs](../../src/windows/rune.rs));
a failed start only logs, and Mumble runs on without it.

Linux `/dictations` (#981) shares the macOS HTTP contract and the owner,
lease, and level logic in [dictation/service.rs](../../src/dictation/service.rs);
the listener ([linux/dictation.rs](../../src/linux/dictation.rs)) records it
without a hold threshold or release tail. Unit tests cover ownership, lease
expiry, busy refusal beside a shortcut capture, and the HTTP round trip
(`local_api` tests now run on Linux). A live Linux microphone run through
`/dictations` is unverified.

## Developer And Prototype Surfaces

```ts
macOS debug runtime                      // not release navigation
├── Activity / HUD Lab / mumble dev -> Observation and developer controls
└── HUD Lab in release -> Only in the playground (Labs › Playground)

`mumble app -- --snap OUT --scene NAME` -> Rendered fixtures    // not proof capture/download/update happened
Linux `mumble` -> the Linux window on Rune, in the Mumble process // dev.rune-ui
├── Calls the host directly: saves settings through it; woken on host changes
├── Sidebar: General, Modes, History
├── Transcription picker -> ChooseModel / CancelModel; progress from the snapshot
├── First run -> the onboarding story; Microphone only while the privacy switch is off
├── Modes: "Choose a file…" through file_dialog (FileChooser portal); drag to reorder
├── Modes: app icons (hicolor theme or pixmaps, PNG or SVG) in the picker,
│   chips, and mode rows; monogram while loading or without an icon
├── Model and application searches: Up/Down highlight, Enter picks it
├── Not yet: right-click delete
└── macOS/Windows surfaces as scenes too (Capabilities + preview data):
    Voice Filter, HUD Lab, Activity, permission rows, Launch at login, Dock icon, shortcut
    sides, the permissions page
macOS `mumble app` -> one menu-bar process, Rune window // dev.rune-ui-macos
├── rune::App on the main thread: accessory, windowless unless setup/onboarding/Dock icon open it
├── Same AppKit loop: status item, updater, Mumble/File/Window menu, Dock reopen event, the HUD pump (16 ms timer; the HUD itself is a Rune overlay)
├── Runtime thread: starts the listener once setup is ready
├── Listener, local API: their own threads
├── macos_host: AppKit event monitor records chords (Option, Fn); NativeHost does permissions,
│   login item, Mumble Microphone, relaunch
├── rune::FileDialogs (file_dialog) chooses Modes' "Choose a file…" .app
├── First run: practice dictations from the listener's event log; a listener that failed to start
├── Sidebar and pane header move the window (Rune El::window_drag; the title bar is transparent)
├── While the window shows, its poll re-checks timed state every 250 ms (Rune Tx::wake_after); close hides it (Dock policy restored)
├── ⌘Q / menu-bar Quit / Ctrl-C quit and join
└── Modes app icons: Finder's icon (NSWorkspace) in the picker, chips, and mode rows
Windows `mumble app` -> one tray process, Rune window // dev.rune-ui-windows
├── Runtime thread owns WindowsDesktopHost (listener, OpenCode, transformations, updater)
├── Named event: a second launch shows the window; tray Show / Enable-Disable / Quit
├── rune::App: --hidden starts windowless, close hides (keep_running); Quit or no tray ends it
├── General adds Launch at login; no terminal paste; chord capture polls
├── HUD: shared indicator model + CPU shader in a Rune overlay
├── Modes: "Choose a file…" opens rune::FileDialogs (file_dialog) on its own thread
├── Modes app icons: the executable's first icon (PrivateExtractIconsW)
└── Not yet: custom caption
iOS prototype -> Record/transcribe/copy and keyboard handoff
Unmerged Windows / deferred features -> Not part of this map
```

Sources: [macos_rune.rs](../../src/macos_rune.rs),
[control/developer_control.rs](../../src/control/developer_control.rs),
`SCENES` in [rune_ui.rs](../../src/rune_ui.rs), [iOS guide](../../ios/README.md).
iOS physical-device proof and temporary-WAV failure cleanup remain gaps;
best-effort deletion is not guaranteed deletion.

`dev.rune-ui` lives in [rune_ui.rs](../../src/rune_ui.rs) and its pane modules;
its tests drive the Rune runtime headlessly (navigation; General rows per
shortcut source, fitting the default window with Advanced collapsed, Shortcut mode's modal, saves through the host, Quit Mumble, microphone-release
confirmation, shortcut recording and Escape, conflicts; History select, delete, two-step
clear, copy; Modes add/delete, corrections, transformations (drag reorder saves once
on release), application conflicts, a chosen file joining the mode, arrow
keys in the model and application searches; the picker's ChooseModel/CancelModel; setup's download; draft
saves; and, from macOS/Windows preview data, each OS's panes and rows, Launch
at login, permission warnings, and
the macOS tour's permissions, practice, and relaunch; Voice Filter's
experiment gate, modes, sliders, and demo requests, HUD Lab's indicator requests, Activity, and shortcut
sides) and compare 62 offscreen
GPU renders with [tests/golden](../../tests/golden). In these tests the macOS and
Windows requests are recorded, never performed; the macOS native host
(`dev.rune-ui-macos`) has no automated test of its own. They skip the goldens without a GPU
adapter and do not prove a real window, input method, or physical shortcut.
Before the single-process change, a headless script against an isolated
service saved a new mode and a correction. See
[the Rune port plan](../plans/rune-ui.md) for what was observed.

`dev.rune-ui-macos` lives in [macos_rune.rs](../../src/macos_rune.rs) and
[rune_ui/macos_host.rs](../../src/rune_ui/macos_host.rs). On hailmary
(macOS 26.2, Command Line Tools only) it compiles, passes clippy
`-D warnings` and the macOS unit tests (including `macos_host`'s chord
capture), and a debug `mumble app` with an isolated data root started, showed
its 980×760 window (CGWindowList; `lsappinfo`: Foreground while open),
answered `mumble dev status/show/hud`, and quit cleanly on Ctrl-C.
`macos_host`'s tests cover the practice feed over a written event log. After
GPUI's removal the same host built with a local Command Line Tools stub passed
clippy and all 648 macOS tests on hailmary (2026-10-22); it was not launched
again. Those builds stub the HUD's Metal shader (Xcode is missing), so the HUD
failed to load its shader there; window contents (including the practice
step), dragging the window, the open panel, the status item, reopen, shortcut capture, and Sparkle have not been
seen or exercised, and no screenshot could be taken over SSH.

`dev.rune-ui-windows` lives in [windows/rune.rs](../../src/windows/rune.rs) and
[rune_ui/windows_host.rs](../../src/rune_ui/windows_host.rs). It has only
been cross-compiled (`cargo xwin clippy -D warnings` for
`x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, last after GPUI's
removal on 2026-10-22); it
has never run on Windows, and no test covers it. It replaces the GPUI app
earlier Windows releases shipped.

`each_os_lists_only_the_panes_it_can_run` in
[rune_ui/tests/panes.rs](../../src/rune_ui/tests/panes.rs) checks pane
availability from the capabilities; it does not exercise native navigation
clicks.

Scenes and tests use fixture state (`Service::Fixture`, `PlatformHost::Fixture`):
requests are recorded, never performed. They do not establish native device
discovery or installed-app behavior.

`default_paste_hotkeys_follow_build_capabilities` in
[app_settings.rs](../../src/app_settings.rs) checks the production default runtime
and settings projections: Paste Last is available in both build profiles. It
does not establish native keyboard delivery.

`structural_event_equality_preserves_serialized_identity` in
[desktop/events.rs](../../src/desktop/events.rs) covers Activity's full-event equality across
event variants, equal timestamps, non-ASCII text, and optional/error payloads.
It does not exercise native Activity selection or scrolling.

**Executed September 3, 2026, source simplification pass:** 444 Rust tests passed
in both debug and release (nine opt-in tests ignored in each), with all twelve
keyboard-layout child scenarios passing in each profile. Formatting and strict
Clippy passed in both profiles. These checks cover the working-tree cleanup, not
a new published app; no installed-app, microphone, or native Linux test was run.

## September 22, 2026 Cleanup Verification

The working-tree cleanup passed 481 Rust tests in debug and 480 in release, with
twelve opt-in tests ignored in each profile and all twelve keyboard-layout child
scenarios passing per profile. Strict all-target/all-feature Clippy, formatting,
app/helper signing-input guards, and changed shell-script syntax checks passed.
The command SDK passed 52 tests; the public SDK passed 56; both typechecks and
builds passed. A packed public SDK imported both Promise and Effect entry points
from an isolated consumer.

The isolated embedded-service smoke passed API 2 handoff, health/models routes,
absence of discovery-file publication, and shutdown on host-lease closure. It did
not open a microphone or run inference. No signed helper artifact, installed-app
settings flow, provider, physical capture/paste, or Linux integration was tested.
The Linux listener-owner harness and source review do not replace Linux CI.
The optimized Modes preview built and launched, but screenshot capture failed
with `could not create image from window`; native visual layout is unverified.

## Grow The Map

```ts
One feature file
├── Sub-features -> Small action/outcome diagrams with stable IDs
├── Entry points -> Every user route and its prerequisites
├── Existing checks -> Source/test links and what they do not prove
└── Gotchas -> Exceptions, known defects, missing native proof
```

This is the map-only stage of the verification workflow, not a working driver.
Keep checks beside the behavior and tests with the implementation. Split new
files only when useful; link shared failure behavior to Recovery.

Future evidence should name the behavior ID, entry point, revision/binary,
platform, prerequisites, outcome, and artifact location. Report skipped paths.
Never publish tokens, personal transcripts, or unredacted diagnostics as proof.
This map does not authorize operating a live installation.
