# Install The Linux Beta

The Linux beta targets x86_64 and ARM64 (aarch64) Linux: i3/X11 and compatible wlroots-based Wayland
compositors such as Hyprland or Sway. Audio uses ALSA, including PipeWire systems
with ALSA compatibility. Transcription is cloud-only: dictation goes to Claude
or Codex through the sign-in those apps already hold, so Mumble runs no local
model. The window still needs a Vulkan driver to draw;
`vulkan-icd-loader` alone is not a driver. Install a driver for your GPU
or a software Vulkan driver. See the Wayland requirements below.

x86_64 machines need a 2013 or newer processor with AVX2 (Intel Haswell / AMD Excavator or later); on an older processor Mumble shows a message and quits instead of starting.

Linux opens the same window as macOS and Windows: Settings, Modes,
and History.
Paste on Wayland uses `wtype` where the compositor offers the virtual keyboard
protocol (wlroots compositors, niri) and the desktop's RemoteDesktop portal
elsewhere (KDE, GNOME). Shortcuts are general: the desktop's GlobalShortcuts portal when it has one, otherwise
your desktop's own keybindings running `mumble` commands (see
[Shortcuts](#shortcuts)).

For Nix/NixOS, use the [Nix guide](nix.md). Nix owns updates to that installation;
Mumble's signed updater owns only the direct-install layout below.

## One Mumble Process

Mumble on Linux is **one process per user** (and per data directory), not a
system-wide/root daemon. It owns the microphone, shortcuts, workers, sounds,
paste, History, the recording pill, and the window. The window
is optional: closing it stops neither dictation nor accepted work, and
launching Mumble again opens it. When the desktop has a tray (a
StatusNotifierItem host, such as Waybar, KDE, or Omarchy's bar), Mumble shows a
tray icon: click it to open the window, or use its menu: **Show Mumble**,
**Paste last dictation**, the update once one
is ready (**Restart to update**), **Enable / Disable Mumble**, and **Quit Mumble**;
while Mumble records or transcribes, the menu and tooltip say so. The mark is white,
and black when the desktop's color scheme prefers light; Mumble reads that setting
when it creates the tray, so a later switch applies at the next launch. KDE's
portal takes the scheme from the window palette, so a mixed theme with light
windows and a dark panel (such as Breeze Twilight) gets a black mark on a dark
panel. Without a tray, quit with **Quit Mumble** at the bottom of the window's
sidebar, the launcher's **Quit Mumble** action, or `mumble quit`.

The window opens at 980×760 logical pixels and floats on tiling window
managers with no window rule: it maps with its minimum size equal to its
maximum, which sway, Hyprland, niri, and X11 managers such as i3 float, then
drops the maximum so you can resize it down to 860×560 or up to any size.
macOS and Windows open the same resizable window.

Settings live in `settings.json` in Mumble's data directory, the same file macOS
and Windows use.

```sh
mumble                             # open the window, starting Mumble if needed
mumble start                       # start Mumble in the background; no window
mumble status [--json]             # query the running Mumble, not an old log entry
mumble quit [--force]              # finish accepted work, release the microphone, exit
mumble restart [--force]           # restart the systemd unit in this desktop's environment
                                # (over SSH: the one it last ran in)
mumble settings set sound_effects off   # every control command (docs/cli.md)
journalctl --user -u mumble.service # diagnostics of the supervised process
```

`mumble app` is the same as `mumble`, `mumble start` runs `mumble app --hidden` in the
background and waits until Mumble answers, and `mumble stop` is the same as
`mumble quit`; older desktop and autostart entries keep working after an update
replaces only the executable. `mumble quit` and `mumble restart` refuse while a
dictation records or transcribes unless `--force`. Linux runs the same
control commands as macOS ([docs/cli.md](cli.md)): `settings`,
`window`, `vocabulary`, `history clear`, `check-for-updates`, `onboarding
reset`, `launch-at-login` (the XDG autostart entry), `logs`, `commands
init|check`, and `history`.

A launch first asks the running Mumble over its socket. If one answers, it
forwards `Activate` with the launcher's `XDG_ACTIVATION_TOKEN` or
`DESKTOP_STARTUP_ID` and exits 0; the running process opens or focuses its
window. If none runs and the installer's (or Home Manager's) `mumble.service` user
unit is loaded, the launch writes an owner-only `session.env` and starts the
unit, which reports readiness to systemd (`Type=notify`) once its socket
answers, then forwards `Activate`. Otherwise (a custom data directory,
`cargo run`, a desktop without systemd) the launching process becomes Mumble
itself. Two racing launches produce one Mumble; the loser forwards and exits 0.

`session.env` holds the display and session variables, the cursor theme and
input method (`XCURSOR_THEME`, `XCURSOR_SIZE`, `XMODIFIERS`, `GTK_IM_MODULE`,
`QT_IM_MODULE`), and, when set, `XDG_DATA_DIRS` and the locale (`LANG`,
`LANGUAGE`, `LC_ALL`, `LC_CTYPE`, `LC_MESSAGES`). Missing display variables
explicitly clear stale Wayland/X11 values inherited by the user manager; missing
locale and data-directory variables are left to the user manager. A saved
`XAUTHORITY` whose file is gone (Xwayland writes a new cookie at each login, so
a reboot strands the old path) is replaced at startup by the user manager's
current value, or dropped so Xlib uses `~/.Xauthority`. A unit started with no
display at all (the desktop never imported its environment into systemd) adopts
the user manager's `WAYLAND_DISPLAY`, else the newest Wayland socket of yours in
`XDG_RUNTIME_DIR` that answers. Hyprland imports nothing by default; add this to
its config so the service, its paste helpers, and the HUD get your session:

```ini
exec-once = dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
```

When the HUD can't show, Mumble's log says why: no display, no layer-shell (and
no XWayland), or the overlay failing to open, with its error. No
permissions are granted. The unit is `PartOf=graphical-session.target`;
desktops managing a custom session target must bind its lifetime to their
login/logout lifecycle. Systemd restarts a crashed Mumble (`Restart=on-failure`)
without a window; after **Update**, the restarted Mumble reopens the window.
There is one Mumble per user: a launch from another desktop session restarts the
supervised unit in the new session, and refuses to take over an unsupervised
one (quit it first).

The socket is owner-only in Mumble's data directory, with same-user peer
credentials, versioned bounded frames, bounded queues, and I/O deadlines. It is
not a network API. It carries
`Activate` and `Status`. The `mumble` control
commands reach Mumble through its local API's `POST /control`; dictation
commands go straight from there to the listener, never through the window.
The local API's `/dictations` also
records on the listener while Mumble is listening, as on macOS: one capture at a
time, so a client's dictation and the shortcut never end each other.

For development, run `cargo run --release`. `mumble dictate` remains an explicit
foreground diagnostic listener without the window or HUD and cannot coexist
with Mumble's listener. With
`MUMBLE_APPLICATION_SUPPORT_DIR` set, launches never start, stop, or restart the
installed user unit; they forward to the Mumble using that data directory or
become it. `MUMBLE_SYSTEMD_UNIT` names another unit for isolated tests.
`mumble logs -n 40` is the diagnostic-log view (`mumble status --lines` is gone).

The window runs on Rune's app lifetime (`rune::App`): Mumble starts it without a
window, each `Activate` opens or focuses it with the forwarded token (on
Wayland through `xdg_activation_v1`; without a token most compositors only
mark it urgent), and closing it keeps the window's state for the next open.

## Install A Published Build

Published builds exist for x86_64 and ARM64, as a bare executable, a `.deb`,
and an `.rpm`. They are built on Debian 12 and need glibc 2.36 or later:
Debian 12, Ubuntu 23.04, Fedora 37, current Arch, or newer.

### Install script

The script installs the bare executable for your architecture into a
user-local layout (`~/.local/share/mumble`) that updates itself. It installs the
stable release, or the newest dev build (on the Dev update channel) while
there is no stable release yet; set `MUMBLE_INSTALL_CHANNEL=stable` or `=dev` to
choose:

```sh
curl -fsSL https://anoma.ly/mumble/files/install-linux.sh | sh
```

If a runtime library is missing, the installer names the packages to install
(with Arch names). Then open Mumble from your application launcher, or run
`~/.local/bin/mumble`. Mumble transcribes with the chosen cloud service (Claude or
Codex), so sign in to that app for dictation to work.

To install the dependencies up front, or to inspect the installer before
running it:

```sh
sudo pacman -S --needed alsa-lib curl jq \
  libxkbcommon libxkbcommon-x11 libx11 libxcb openblas openssl util-linux \
  vulkan-icd-loader wl-clipboard wtype xxd
curl --proto '=https' --tlsv1.2 -fsSLO \
  https://anoma.ly/mumble/files/install-linux.sh
less install-linux.sh
sh install-linux.sh
~/.local/bin/mumble
```

The installer verifies the signed release feed, exact artifact size, and
SHA-256 before installing. It uses no root access. It creates the managed
version layout, `~/.local/bin/mumble`, a desktop launcher, and an XDG autostart
entry and a `mumble.service` user unit. It does not start recording during installation.
General › **Launch at login** turns that autostart entry on or off
(`~/.config/autostart/anomaly.mumble.desktop`); Mumble leaves an entry it did not write alone.
Where the desktop has the Background portal (GNOME, KDE), Mumble asks it at startup
to run in the background, and Launch at login goes through it: the portal writes
and removes that same autostart entry (marked `X-XDP-Autostart=anomaly.mumble`).
Elsewhere Mumble writes the entry itself. The launcher entry below is written only
when no installed package or data directory provides one, and left as it is
while the program it starts is there. Installed by a version manager (mise,
asdf), both entries start its shim (`~/.local/share/mise/shims/mumble`), which
outlives each version, so an upgrade can't strand them. An entry Mumble wrote
whose program is gone points at this Mumble the next time it starts, or goes
once a package ships its own.
The launcher entry is `~/.local/share/applications/anomaly.mumble.desktop`, named
for Mumble's app id `anomaly.mumble`, which its windows also report. Earlier installs
named both entries `mumble.desktop`; Mumble and the installer rename Mumble's own.
Its icon is `anomaly.mumble`: packages install it in the hicolor theme, and a
direct install gets it in `~/.local/share/icons/hicolor` when Mumble first starts.

Remove the application while preserving logs and settings with:

```sh
curl -fsSL https://anoma.ly/mumble/files/install-linux.sh | sh -s -- uninstall
```

or, with a saved copy of the installer, `sh install-linux.sh uninstall`.

### Upgrading from 2.3.1

HEX 2.3.1 read every `/dev/input` device on Wayland, so it needed the `input`
group. Current builds don't: they use the GlobalShortcuts portal or `mumble`
commands bound in your desktop (see [Shortcuts](#shortcuts)). Don't grant input
access; install the new build instead.

The new build is a separate install in `~/.local/share/mumble` and imports
nothing from 2.3.1's `~/.local/share/voice-control`, so set up your settings
again. The installer refuses to replace 2.3.1's files, so remove them
first (the paths below assume the default XDG directories):

```sh
systemctl --user disable --now mumble.service
rm -f ~/.config/systemd/user/mumble.service ~/.local/bin/mumble \
  ~/.local/share/applications/mumble.desktop ~/.config/autostart/Mumble.desktop
systemctl --user daemon-reload
rm -rf ~/.local/share/voice-control   # optional: 2.3.1's settings, logs, and models
curl -fsSL https://anoma.ly/mumble/files/install-linux.sh | sh
~/.local/bin/mumble
```

On Hyprland, bind the shortcut in your config as described in
[Shortcuts](#shortcuts).

### Debian, Ubuntu, Fedora, and openSUSE

The `mumble` package installs `/usr/bin/mumble`, the desktop entry, and the systemd
user unit `mumble.service` (the same files as the AUR package below). Download
it for your architecture from <https://anoma.ly/mumble/download/linux-x64-deb>
(`linux-arm64-deb`, `linux-x64-rpm`, `linux-arm64-rpm`), then:

```sh
sudo apt install ./mumble.deb            # Debian, Ubuntu
sudo dnf install ./mumble.rpm            # Fedora (zypper install on openSUSE)
systemctl --user enable mumble.service   # optional: start Mumble at login
```

There is no apt or dnf repository yet. Mumble never replaces a package-installed
executable: when a new stable release is out, Settings shows the command that
downloads and installs its package. Package installs follow the stable channel,
so a package from the dev channel learns of the next stable release only.

### AUR

From the next release on, the AUR package `mumble-bin` installs `/usr/bin/mumble`,
the desktop entry, and the systemd user unit `mumble.service`, from the same
release executable, for `x86_64` and `aarch64`. It is not on the AUR yet. With
an AUR helper:

```sh
yay -S mumble-bin
systemctl --user enable mumble.service   # optional: start Mumble at login
```

pacman updates it; Mumble never replaces a package-installed executable, and
Settings says "Updated by your package manager". It is unrelated to the
official `hex` package (a hex dump tool that installs `/usr/bin/hx`).

## Install From Source

Install the native dependencies and, for a source build, the Rust toolchain:

```sh
sudo pacman -S --needed base-devel git rustup python alsa-lib curl jq openssl xxd \
  util-linux libxkbcommon libxkbcommon-x11 libx11 libxcb openblas \
  vulkan-headers vulkan-icd-loader \
  shaderc spirv-headers clang cmake pkgconf wl-clipboard wtype
rustup default stable
```

Build and install Mumble as your desktop user:

```sh
git clone https://github.com/supabitapp/mumble.git
cd mumble
./scripts/install-linux.sh
~/.local/bin/mumble
```

Do not run the installer as root. It installs a user-local binary, desktop
launcher, and autostart entry in the same layout as published builds, and
marks the version as a source build (`source-build` beside the executable).
The in-app updater never replaces a source build; Settings shows "Updated from
your source checkout", and you update by pulling and running the installer
again. Installing a published release over it switches back to signed
updates. With a custom
`MUMBLE_APPLICATION_SUPPORT_DIR`, `cargo run --release` runs an
unmanaged development Mumble instead; without one it forwards to the installed Mumble.

## Dictate

Hold **Alt-Space**, speak, then release. Double-tap the shortcut to keep
recording, and press the same chord again to finish. **Shortcut mode** picks how the shortcut
triggers: **Hold or double-tap** (the default), **Hold**, **Tap** (starts and
stops with single presses), or **Double-tap** (waits for two complete taps,
and needs a shortcut with a key).
**Ctrl-Alt-V** pastes the last dictation again. Settings shows listener state,
Start/Stop, and operational errors. Changing a shortcut or a setting the
listener reads at start temporarily stops listening and restores the previous
running state after the edit.

Modes apply corrections, OpenCode rewriting, and transformations per
application or website; the application picker lists `.desktop` entries. On
X11, Mumble reads the focused application from the active window's process name
(`code`, `chrome`). On Wayland, compositors with the foreign-toplevel protocol
(wlroots compositors such as Sway, niri, Hyprland) report the activated
window's app ID, which Mumble names by its `.desktop` entry
(`google-chrome-stable`, `firefox`). GNOME and KDE don't report it, so the
Global mode applies there.

Website rules read the focused page's address through AT-SPI, the desktop
accessibility bus, on X11 and Wayland. Firefox exposes it whenever the
accessibility bus is running. Chrome and other Chromium browsers expose it
only with accessibility turned on: launch them with
`--force-renderer-accessibility` (for example in
`~/.config/chrome-flags.conf`), or run a screen reader. Without it, the
application rule applies. Completed dictations are kept in History.

**Turn off the mic when not dictating** (General) releases the microphone between dictations; it then opens
on the shortcut press without pre-roll. **While dictating** can mute the default
output through `wpctl` (PipeWire) or `pactl` (PulseAudio), or pause playing
MPRIS media players over D-Bus, once the hold passes 300 ms, so shortcut chords
and short taps never do; Mumble restores only what it changed.
Cancelling shortcut capture preserves the old binding unless the new binding
has already been saved; listening resumes only if it was previously running.

### Bluetooth headsets

A Bluetooth headset can't play high-quality audio (A2DP) and use its
microphone at once. While any application records from the headset's
microphone, PipeWire (WirePlumber) or the headset switches it to the
hands-free profile (HFP/HSP): mono, call-quality sound in both directions.
This is how Bluetooth audio works, not something Mumble chooses, and how quickly
the headset switches back depends on the headset and WirePlumber's settings.

If Mumble records from the headset's microphone, **Turn off the mic when not
dictating** trades off differently on and off:

- **On** opens the microphone only when you press the shortcut,
  so each dictation switches the headset to hands-free and back. Playback
  drops out or pauses during the switch, and the microphone takes longer to
  open, so the first words can be lost.
- **Off** (the default) keeps the microphone open, so the headset stays in hands-free
  mode, with call-quality playback, for as long as Mumble listens.

The recommended setup is a separate microphone for input (the laptop's
built-in microphone or a USB microphone) and the Bluetooth earbuds for output
only. Pick that microphone in General's **Microphone** list, or make it the
system's default input (for example with `wpctl set-default`), so nothing
opens the headset's microphone.

General › Advanced's Bluetooth option (`avoid_bluetooth_microphone`, on by
default) helps only part of the way:

- It applies only while the microphone is **System default** and the default
  input is a Bluetooth headset. A Bluetooth microphone you picked in Settings,
  or `--device`, is always used as named.
- It needs another hardware input that isn't reported unplugged, and
  `pactl` (PipeWire's PulseAudio server or PulseAudio) to move the stream.
  With no other input, Mumble records from the headset.
- On Linux, Mumble can only open ALSA's default device, which is the Bluetooth
  source, and then moves its own stream to the other microphone. The stream
  is briefly on the headset first, so the headset may still switch to
  hands-free mode, and Mumble doesn't switch its profile back. Forcing the
  headset's profile is up to your sound settings.
- It changes only Mumble's own recording, never other applications or the
  system default.

To avoid the switch entirely, use the recommended setup above.

## Shortcuts

Mumble gets its shortcuts one of three ways, and all of them drive the same
dictation rules:

- **X11:** Mumble grabs the shortcuts Settings records.
- **Wayland with the GlobalShortcuts portal** (for example niri with
  `xdg-desktop-portal-gnome`, GNOME, or KDE): Mumble asks the desktop for the
  shortcuts Settings records. The desktop decides: it may grant them, assign
  another key, or refuse one that it or another binding already uses. Settings
  lists what the desktop granted under **Desktop shortcuts**, with an error for
  any it refused. A desktop may keep an older binding when you record a new
  shortcut (KDE does); where its portal supports it, **Desktop shortcuts >
  Open** shows the desktop's own dialog for Mumble's shortcuts, and Settings
  follows what you choose there. KDE asks once in a "Global Shortcuts
  Requested" dialog; Mumble waits for your answer, and if you cancel it KDE asks
  again the next time Mumble binds. Mumble's shortcuts are listed under **Mumble** in
  System Settings › Keyboard › Shortcuts. The portal identifies Mumble by its app id
  `anomaly.mumble`, its installed `anomaly.mumble.desktop` entry, and by inspecting
  the Mumble process. A downloaded executable has no entry, so Mumble writes a minimal
  `~/.local/share/applications/anomaly.mumble.desktop` that runs it (a managed
  install replaces it); if the portal still can't find it, restart the portal with
  `systemctl --user restart xdg-desktop-portal`. Run `mumble service` directly:
  a setgid wrapper such as `newgrp`, once used to grant input-device access,
  makes the portal refuse Mumble. The desktop delivers the keys to Mumble, not to the
  focused application. On GNOME and KDE Plasma, new setups dictate with Ctrl+Alt+Space,
  because GNOME keeps Alt+Space for the window menu and Plasma for KRunner; Settings warns when a shortcut uses a key GNOME
  keeps for itself. Keys you change in GNOME Settings › Keyboard work at once,
  and Mumble's Settings and `mumble status --json` show them. **Disable** (General or the tray) keeps Mumble disabled
  across restarts until you enable it again. On GNOME, Disable also removes
  Mumble's entries from Settings › Keyboard › Global Shortcuts; Enable then asks
  you to approve them again. Mumble also removes the duplicate list that versions
  before `anomaly.mumble` left under the id `mumble`.
- **Hyprland** has the portal but takes every shortcut without a key, so
  nothing happens until you bind one in your config, and the key Settings
  records is not used. Mumble can't tell whether your config binds it. Prefer
  keys without modifiers, because Hyprland adds every modifier you still hold
  to Mumble's paste (see [Native Wayland Requirements](#native-wayland-requirements)).
  Avoid Alt in the bind too: Hyprland takes the key (Space) but lets Alt
  through to the focused window, so Electron apps with a hidden menu bar
  (Slack, Discord, VS Code) see Alt pressed and released alone, move focus to
  their menu, and the paste lands there instead of the message box
  (#1023). Mumble still logs the
  paste as sent. Ctrl+Space or a key without modifiers works.
  Hyprland names each shortcut `anomaly.mumble:<shortcut>`:
  `anomaly.mumble:dictation`, `anomaly.mumble:paste-last`, and
  `anomaly.mumble:raw-dictation` (Dictate without processing). The names stay
  the same when you record another key in Settings (#1376), so a bind keeps
  working; `hyprctl globalshortcuts` lists them while Mumble runs. For example:

```lua
hl.bind("End", hl.dsp.global("anomaly.mumble:dictation"))
hl.bind("F10", hl.dsp.global("anomaly.mumble:paste-last"))
```

  Traditional Hyprland configs write
  `bind = , End, global, anomaly.mumble:dictation`. Earlier builds put the
  recorded key in the name (`anomaly.mumble:dictation-ALT-space`), and binds to
  `mumble:dictation` or `mumble:paste-last` (names from before Mumble's app id was
  `anomaly.mumble`) are older still; both do nothing now, so replace them. Remove any `cancel-dictation`
  bind: Mumble no longer has it. Hyprland 0.56's Lua binds send no release when the modifier comes
  up before the key (Alt before Z); 0.57 fixes this. On 0.56, Toggle is
  unaffected: the next press still finishes. In Hold, recording continues
  until the next press and release, so lift the key first or use Toggle.
  For hold-to-talk that doesn't depend on the portal's release, bind Mumble's
  own commands instead (below), for example
  `bind = , End, exec, mumble dictation start` with
  `bindr = , End, exec, mumble dictation stop`, or one
  `bind = , End, exec, mumble dictation toggle`. Pasting into a terminal needs
  **Terminal paste shortcut** (Ctrl+Shift+V) unless the terminal accepts
  Ctrl+V.
- **Wayland without the portal** (for example Sway): nothing registers global
  shortcuts. Settings lists these commands; bind them in your desktop's own
  keyboard configuration:

```sh
mumble dictation start     # bind to a key's press: start recording
mumble dictation stop      # bind to the same key's release: stop and transcribe
mumble dictation toggle    # one key: start, then stop and transcribe
mumble paste-last          # paste the last dictation again
mumble dictation toggle --raw  # dictate without processing: paste the words as heard
```

For example, hold-to-dictate on F9 in Sway, and a toggle in any desktop that
runs commands on key presses only:

```sh
bindsym F9 exec mumble dictation start
bindsym --release F9 exec mumble dictation stop
bindsym Ctrl+Alt+d exec mumble dictation toggle
```

GNOME 46 and older (Ubuntu 24.04) have no shortcuts portal. There, Settings ›
Desktop shortcuts › **Add GNOME keyboard shortcuts** writes Mumble's own entries
into GNOME Settings › Keyboard › Custom Shortcuts with your shortcuts:
`mumble dictation toggle` (GNOME runs custom shortcuts on the key press only, so
dictation toggles) and `mumble paste-last`. Mumble removes them while it's disabled and puts them back when you enable
it; **Remove** takes them out for good. Keys you change in GNOME Settings stay.
Mumble never touches your other custom shortcuts.

`mumble dictation stop --print` stops the same way, then waits up to 60 s and
writes the transcript to stdout instead of pasting it, after the same
processing a paste gets. Only that command waits; the listener stops
recording at once, so a following `start` isn't blocked. No speech exits 4
and prints nothing; a failure, cancel, or timeout prints nothing, exits 1,
and never pastes. History keeps the text as Printed, and Paste Last skips it.
See `docs/cli.md`.

The commands are safe to bind anywhere, including X11 beside the grabs. They
talk to the running Mumble and never start it: without one, or while Mumble is
not listening, they print a one-line error and exit non-zero. Starting while recording,
and stopping while idle do nothing, so key repeat is
harmless. Start and stop bypass Hold, Toggle, and double-tap: start records,
stop finishes. A shortcut press also finishes a recording a command started.

In **Tap** mode, a **Cancel** shortcut (Escape by default) throws away the
recording: nothing is pasted or saved, and dictations already transcribing
still finish. On X11, Mumble grabs that key only while a recording runs, so it
works normally everywhere else. On Wayland a desktop shortcut would hold the
key all the time, so Mumble binds nothing; bind `mumble dictation cancel` to a key in
your desktop's keyboard shortcuts instead. `mumble dictation cancel` works in any
mode. In the Hold modes there is no cancel: every dictation finishes and pastes.
On KDE, Mumble removes the Cancel dictation shortcut older builds asked for,
so its key returns to your applications.

Shortcuts need a non-modifier key. Modifier-only shortcuts, such as Super
alone, are not supported on Linux.

Recording sounds are enabled at 50% volume by default, including when upgrading
older settings. A start sound is queued immediately when the shortcut starts capture,
a stop sound marks the end of a retained recording (not completed transcription
or paste), and a cancel sound acknowledges cancellation of an active capture.
Brief taps still play the start sound, but recordings shorter than 300 ms remain
discarded and do not play the stop sound. **Sound volume** in Settings
offers Off, 25%, 50%, 75%, and 100%; changes save immediately without restarting
the listener and preview the start sound unless set to Off. `mumble dictate` uses
the same saved volume. Playback uses the default audio output through a bounded
background worker; unavailable audio output does not prevent dictation.

Closing the window leaves Mumble running on both X11 and Wayland. The
recording/processing pill never takes focus or intercepts clicks, with or
without the window open: a Rune overlay that is a layer-shell surface on
Wayland and an override-redirect window on X11. On a Wayland compositor without
layer-shell (GNOME), only the pill opens on XWayland, as that X11 window with an
empty input shape on the primary monitor; the rest of Mumble, including the window
and portal shortcuts, stays on Wayland. It needs the window's event
loop, so it is unavailable when the window cannot start (no display or GPU)
and in `mumble dictate`. **Recording indicator** places it at the top or bottom of the
screen, or turns it off.

Insertion uses the clipboard and **Ctrl-V** for new installations. Enable
**Terminal paste shortcut** in Settings for targets that require **Ctrl-Shift-V**.
Existing X11 settings retain their previous Ctrl-Shift-V behavior until changed.
The target application must accept the selected binding.

After a paste, Mumble puts the previous clipboard back, as on macOS
([#391](https://github.com/anomalyco/hex/issues/391)). Before writing the
transcript it reads every format the clipboard offers (text, rich text, images),
up to 32 MiB within one second, and 500 ms after the paste shortcut serves them
again from Mumble until something else copies. It restores only if nothing newer
replaced the transcript in the meantime: a later dictation, a copy, or another
application. An empty clipboard stays empty. A larger or unreadable clipboard
is left holding the transcript, and Mumble's log says why. When automatic paste
fails, the transcript stays on the clipboard for Ctrl+V. Nothing confirms the
target read the paste, so a target stalled past the delay can paste the
restored clipboard instead (see [#24](https://github.com/anomalyco/hex/issues/24)).
On Wayland, restoring needs the compositor's `ext-data-control` protocol
(KDE Plasma 6, recent wlroots compositors) or `wlr-data-control` (wlroots
compositors, niri, Hyprland); without either the transcript stays, and
Settings › Output says so.

GNOME offers neither. There you can opt in to Labs › Restore clipboard on
GNOME (`mumble labs on gnome-clipboard-restore`, off by default): Mumble then reads
and restores the clipboard through XWayland, which GNOME keeps in sync with
Wayland apps both ways. It works only while XWayland is already running (an X11
app is open and `DISPLAY` is set); Mumble never starts XWayland, and otherwise the
transcript stays and the log says why. The same rules apply: nothing newer is
ever overwritten.

On GNOME, Mumble pastes through the RemoteDesktop portal, and GNOME shows its
remote-control indicator while that session is open. Mumble closes it 15 seconds
after the last paste and reopens it at the next one with the grant you
approved, so you're asked only once.

## Native Wayland Requirements

- Export the compositor's nonempty `WAYLAND_DISPLAY` in the launch environment.
  Mumble and its window (winit) select the same native backend even when XWayland's
  `DISPLAY` is also present. `XDG_SESSION_TYPE` alone does not select Wayland.
- Install `wl-clipboard`. Mumble copies each transcript, then sends Ctrl+V (or
  Ctrl+Shift+V with terminal paste) through one of two general paths, chosen at
  the first paste:
  - `wtype`, when the compositor offers the virtual keyboard protocol
    (`zwp_virtual_keyboard_manager_v1`; wlroots compositors and niri) and
    `wtype` is installed.
  - Otherwise the desktop's RemoteDesktop portal
    (`org.freedesktop.portal.RemoteDesktop`; KDE, GNOME). The first paste asks
    you to allow Mumble to control the keyboard; Mumble keeps the portal's restore
    token (`remote-desktop-token` in Mumble's data folder), so it asks once. Revoke
    it in the desktop's settings.
  - With neither, the dictation fails with "Copied to the clipboard; press
    Ctrl+V to paste" and the transcript stays on the clipboard.

  There is no silent XWayland, direct-typing, uinput, or privileged
  input-injection fallback; the only XWayland uses besides the HUD are the
  opt-in clipboard restore above and the opt-in floating meeting capsules
  below. Transcript text travels on stdin, not command-line
  arguments.
- The overlay uses layer-shell. Without it (GNOME), only the HUD opens on
  XWayland as an override-redirect, click-through window that never takes
  focus. The meeting capsules take clicks, so there a call offer opens the
  Meetings pane instead, unless you opt in to Labs › Floating meeting capsules
  on GNOME (`mumble labs on gnome-meeting-capsules`, off by default): the offer
  and recording capsule then float over the call on XWayland, taking clicks
  on their buttons but never keyboard focus. It works only while XWayland is
  already running; Mumble never starts it. With neither, Settings reports that the HUD is unavailable rather than
  presenting a normal focus-taking window as an overlay.
- Shortcuts come from the GlobalShortcuts portal or from your own keybindings
  for the `mumble` commands ([Shortcuts](#shortcuts)). Mumble reads no input devices
  and needs no `input` group membership or other device permission. If the
  desktop closes the portal session, the listener stops with an error and an
  active recording is cancelled; enable it again from Settings or `mumble start`.
- Mumble can't see which keys are down on Wayland, and some compositors merge
  held physical modifiers into its paste: on Hyprland, Alt still held from
  Alt+Z turns `wtype`'s Ctrl+V into Ctrl+Alt+V, and a paste-last bound to
  Ctrl+Alt+V is matched by Hyprland again and never reaches the app. The
  RemoteDesktop portal shares the desktop's keyboard the same way. So when a
  shortcut with modifiers asks for a paste, Mumble waits for its release (the
  portal's Deactivated) plus 250 ms, at most 2 s after a press whose release
  never comes. After a `mumble` command, whose keybinding Mumble can't see, it waits
  250 ms. A modifier still held past that changes the paste, so bind keys
  without modifiers (End, F-keys) or let go of the modifiers before the
  transcript arrives. Mumble logs a dictation as pasted once it sent the paste
  shortcut; it can't confirm the application accepted it. A compositor that
  passes a shortcut's Alt through to the app (Hyprland) can also open an
  Electron app's menu when Alt comes up, which takes the paste (see
  [Shortcuts](#shortcuts)).

## Updates And Validation

Managed installs check for signed updates when Mumble starts and every
24 hours until an update is ready. Mumble downloads, verifies, and activates an
available update, then offers an **Update** button in the window that restarts
the unit and reopens the window. The next launch
through `~/.local/bin/mumble` also uses the activated version. The signed update
path is implemented but still awaiting a complete cross-version validation on
the supported Arch/i3 host.

See [`plans/linux.md`](plans/linux.md) for the engineering contract and remaining
validation work. The native Wayland implementation builds on the feature
proposed in [PR #28](https://github.com/anomalyco/hex/pull/28), with separate
input-state, lifecycle, and paste regression coverage. Physical device,
compositor, and target-application smoke tests remain necessary before a release.
`scripts/test-wayland-paste.sh /path/to/compiled-test-executable /path/to/mumble` runs the real
paste helpers against a GTK target under an isolated headless Sway compositor.
Its first paste runs while another virtual keyboard holds Ctrl and Alt.
It then checks that every format of the previous clipboard comes back after the
paste, and that an empty clipboard stays empty.
It also starts an isolated Mumble, opens its window through a second `mumble app`
(the window must belong to the same process), repeats paste while the window is
open, closes the window, checks that the same process still answers, opens the
window again (`MUMBLE_WAYLAND_REOPEN=0` skips that), and quits. It requires the GTK development
libraries, Sway, `wl-clipboard`, `wtype`, `dbus-run-session`, and a Vulkan driver
(Mesa's software driver is sufficient), and must run as a non-root user. It uses
isolated app data and no microphone; CI runs it with the display-free suite and
Xvfb hotkey tests.

`python3 scripts/test-linux-service.py /path/to/mumble` is a display-free,
device-free single-instance check: commands fail fast without Mumble, `mumble start`
becomes Mumble, second launches (`start`, `app --hidden`, `service`, `app`) forward
and exit 0, a window request without a display fails only the window,
same-user socket permissions, version-1 clients get an error, invalid,
oversized, and stalled frames, crash/stale-socket recovery, two racing launches
yield one Mumble, `mumble quit` drains and removes the socket, and a custom data
directory never runs `systemctl`.
