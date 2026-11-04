# Mumble

Mumble is voice dictation for Mac, Windows and Linux. Hold a key, speak, and it
pastes.

## How it works

- hold the shortcut, speak, then let go
- Mumble transcribes with Claude or Codex, through the sign-in their apps already
  hold, and pastes into whatever app you're in
- choose the service and language in Settings › General
- experimental features live in Labs, at the bottom of the sidebar

## Downloads

Stable is the current release. Dev is the newest build of `dev` and updates
itself from the dev channel.

| Platform | Stable | Dev |
| --- | --- | --- |
| macOS (Apple silicon) | [DMG](https://anoma.ly/mumble/download/macos-arm64-dmg) | [DMG](https://anoma.ly/mumble/download/dev/macos-arm64-dmg) |
| Windows x64 | [Installer](https://anoma.ly/mumble/download/windows-x64-setup) | [Installer](https://anoma.ly/mumble/download/dev/windows-x64-setup) |
| Windows ARM64 | [Installer](https://anoma.ly/mumble/download/windows-arm64-setup) | [Installer](https://anoma.ly/mumble/download/dev/windows-arm64-setup) |
| Linux x86_64 | [Executable](https://anoma.ly/mumble/download/linux-x64) | [Executable](https://anoma.ly/mumble/download/dev/linux-x64) |
| Linux x86_64 (Debian, Ubuntu) | [.deb](https://anoma.ly/mumble/download/linux-x64-deb) | [.deb](https://anoma.ly/mumble/download/dev/linux-x64-deb) |
| Linux x86_64 (Fedora, openSUSE) | [.rpm](https://anoma.ly/mumble/download/linux-x64-rpm) | [.rpm](https://anoma.ly/mumble/download/dev/linux-x64-rpm) |
| Linux ARM64 | [Executable](https://anoma.ly/mumble/download/linux-arm64) | [Executable](https://anoma.ly/mumble/download/dev/linux-arm64) |
| Linux ARM64 (Debian, Ubuntu) | [.deb](https://anoma.ly/mumble/download/linux-arm64-deb) | [.deb](https://anoma.ly/mumble/download/dev/linux-arm64-deb) |
| Linux ARM64 (Fedora, openSUSE) | [.rpm](https://anoma.ly/mumble/download/linux-arm64-rpm) | [.rpm](https://anoma.ly/mumble/download/dev/linux-arm64-rpm) |

Or install with:

- Homebrew: `brew install --cask anomalyco/tap/mumble`
- AUR: `yay -S mumble-bin`
- install script (Linux): `curl -fsSL https://anoma.ly/mumble/install-linux.sh | sh`
- Nix: `nix profile add github:supabitapp/mumble`

More docs are in [`docs/`](docs/README.md).

## Contributing

We don't take pull requests. [Open an issue](https://github.com/supabitapp/mumble/issues)
describing the bug or idea, and our agents usually turn it around fast.

MIT licensed.
