# Nix

The flake packages Mumble for `x86_64-linux` and `aarch64-linux`. Both build
from source; `aarch64-linux` evaluates but has not been built or run on real
ARM64 hardware yet. It supports the Linux X11 beta
and native Wayland on compatible wlroots compositors. Wayland requires
layer-shell for the recording overlay, the clipboard protocol used by
`wl-copy`, and the virtual-keyboard protocol used by `wtype`. Global shortcuts
come from the desktop's GlobalShortcuts portal, or from your own compositor
keybindings running the `mumble` commands. This is not a claim of support for every
Wayland desktop; GNOME and KDE are not covered by this contract.

## Run Or Install

From a checkout, with Nix flakes enabled:

```sh
nix build
nix run . -- app
```

Or add `mumble.url = "github:supabitapp/mumble"` to your configuration's flake inputs
and install `mumble.packages.${system}.default` (`x86_64-linux` or
`aarch64-linux`). The package supplies the
`mumble` command and desktop launcher, native libraries, and the `curl`,
`wl-copy`, and `wtype` runtime dependencies. Your system still needs a
working audio session and a Vulkan driver for Settings, even when inference
falls back to the CPU. On non-NixOS hosts, Nix GUI
applications may require the host's usual Nix graphics integration.

On NixOS, Mumble built from its own pinned nixpkgs can use an older glibc than
your system's graphics drivers need. Mumble then finds no suitable Vulkan
adapter and its window does not open. Make Mumble's nixpkgs follow your
system's, so both use the same glibc:

```nix
{
  inputs.mumble = {
    url = "github:supabitapp/mumble";
    # Use the name of your own nixpkgs input here.
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

Dictation is transcribed by Claude or Codex through the sign-in their own
apps hold; there is no local model to download. Launch Mumble as your normal
desktop user. Do not run Mumble as root.

The Nix store is not Mumble's user-local managed installation layout. In-app
updates do not replace a Nix installation. Update the flake input and
rebuild or switch your Nix/Home Manager configuration instead. Remove any
old source-install XDG autostart entry before enabling a second autostart
mechanism; do not run two Mumble listeners.

## NixOS

Import `mumble.nixosModules.mumble` and enable the package:

```nix
{
  imports = [ mumble.nixosModules.mumble ];
  programs.mumble.enable = true;
}
```

This installs Mumble system-wide. It does not start a root service or enable
autostart for every account. Use the
Home Manager module below for per-user autostart. Both modules accept an
explicit `programs.mumble.package` override. The flake also exports
`mumble.overlays.default` for configurations that prefer `pkgs.mumble`.

## Home Manager

```nix
{
  imports = [ mumble.homeManagerModules.mumble ];
  programs.mumble = {
    enable = true;
    autostart = true;
    # Use your compositor's session target when appropriate:
    # systemdTarget = "sway-session.target";
  };
}
```

The module always installs the systemd **user** service. Autostart defaults to
`false`; when enabled, the unit is wanted by `graphical-session.target` by default.
It waits for a Wayland socket or an X11 window manager before launching `mumble service`,
which reports readiness to systemd (`Type=notify`) once its socket answers.
Mumble is one process: `mumble` opens its window, starting the unit when it is not
running, and `mumble start` starts it without the window. Closing the window never
stops Mumble; a tray icon shows when the desktop has a tray host. The unit stops with the configured target
and retries failed starts without a tight restart loop; a crash restart opens no
window. `mumble quit` (or **Quit Mumble** in the window) leaves it stopped until the
next session or the next launch.

Your session manager must import its current `DISPLAY`, `XAUTHORITY`,
`WAYLAND_DISPLAY`, and `XDG_RUNTIME_DIR` values as applicable into the systemd
user environment **before** starting the target, and stop the target at
logout. Use your compositor's systemd integration, such as Home Manager's
Sway integration, rather than starting the target from a shell login.
The module does not create a graphical session or grant permission to use
one. A bare `startx`/i3 session without systemd session integration will not
start this service automatically.

`mumble start` records the current display/session environment in an owner-only
file read by the unit; it clears stale variables when switching between X11 and
Wayland. There is one Mumble per user. Launching it from a different desktop
explicitly restarts the unit for that session. The unit names Mumble's default
data folder, so Mumble still treats it as its own supervised service: a stale
`XAUTHORITY` from an earlier login (GNOME's XWayland cookie) is replaced with
the user manager's current one, keeping the HUD working after a re-login (#1140).

Home Manager's normal service-switch policy applies: newly enabled services
can start during a switch if the session target is already active, and
package changes can restart a running service. Do not switch during an
active recording. Inspect failures with:

```sh
systemctl --user status mumble.service
journalctl --user -u mumble.service
```

### Declarative Settings

```nix
{
  programs.mumble = {
    enable = true;
    shortcut = "control+option+space"; # dictation_hotkey
    provider = "claude"; # claude or codex
    language = "de";
    settings = {
      shortcut_mode = "toggle";
      sound_effect_volume = 0.4;
      paste_with_shift = true;
      "labs.live_text" = true;
    };
  };
}
```

`settings` takes the dotted keys `mumble settings list` shows, with JSON values:
booleans, numbers, text such as shortcuts and choices, `null` to clear an
optional setting, and attribute sets or lists for its `json` settings. The
`shortcut`, `provider`, and `language` options are shorthands for
`dictation_hotkey`, `transcription.provider`, and `transcription.language`.

Every activation merges them into Mumble with `mumble settings apply`, after
systemd units reload. Declared keys overwrite Mumble's values; anything you
change in the window that isn't declared keeps its value until you declare
it. A running Mumble saves and adopts the settings at once, so it never writes
older values back from memory; with Mumble stopped they go straight into
`settings.json`. `settings.json` stays an ordinary writable file. An unknown
key or invalid value applies nothing and prints a warning, without failing
the switch.

## Wayland Shortcuts

Mumble reads no input devices, so neither module adds users to the `input`
group, installs udev rules, or grants capabilities. With a GlobalShortcuts
portal, the desktop grants the shortcuts Settings records; the portal
identifies Mumble by the package's `anomaly.mumble.desktop` entry, so keep the package's
`share/applications` on `XDG_DATA_DIRS` (NixOS and Home Manager profiles do).
Without the portal, bind the commands in your compositor, for example in Sway:

```sh
bindsym F9 exec mumble dictation start
bindsym --release F9 exec mumble dictation stop
```

`mumble dictation toggle` and `mumble paste-last`
are also available; see the
[Linux guide](linux.md#shortcuts). The paste helper is `wtype`; no privileged
input-injection fallback is installed.

## Development

```sh
nix develop
cargo build --locked
cargo test --locked
cargo run --locked -- app
```

The package and shell share the native build environment and runtime
library/plugin paths. OpenBLAS link ordering is handled by `build.rs`, not
by Nix-only `RUSTFLAGS`. The shell also supplies the paste and download
helpers, so it does not rely on the installed app's wrapper. Desktop commands
still require a real graphical/audio session and the permissions described above.

`nix flake check` evaluates the module and shell-environment regressions and
builds the package with its normal display-free Rust tests. Tests explicitly
marked ignored because they require a desktop remain ignored; the Nix build
does not exercise live input or the cloud services.

The initial packaging design builds on [PR #29](https://github.com/anomalyco/hex/pull/29).
Declarative settings follow
[@SymphonySimper's request in #588](https://github.com/anomalyco/hex/issues/588).
