# Mumble on the Anomaly release service

Contract between Mumble's release workflow (publishing) and Mumble's in-app updater
(client). The service and its rules are `anomalyco/release`
(`docs/protocol.md`, `docs/publishing.md`); this file pins Mumble's names.

## Names

```ts
https://anoma.ly/release/mumble/              the service (its dev stage, dev.anoma.ly/release, only tests the service)
├─ index.json                              download page and installers
└─ desktop/<channel>.json                  the manifest the app polls; channels stable, beta, dev
   └─ release.targets
      ├─ macos-arm64          Mumble.zip         ditto zip, root Mumble.app (updater)
      ├─ macos-arm64-dmg      Mumble.dmg         notarized DMG (download page)
      ├─ windows-x64          mumble.exe         bare executable (updater)
      ├─ windows-x64-cli      mumble.com         console entry point beside mumble.exe (updater)
      ├─ windows-x64-setup    MumbleSetup.exe    Inno Setup installer (download page)
      ├─ windows-arm64        mumble.exe
      ├─ windows-arm64-cli    mumble.com
      ├─ windows-arm64-setup  MumbleSetup.exe
      ├─ linux-x64            mumble             bare executable (updater, installer, AUR)
      ├─ linux-x64-deb        mumble.deb         Debian/Ubuntu package (package manager updates it)
      ├─ linux-x64-rpm        mumble.rpm         Fedora/openSUSE package
      ├─ linux-arm64          mumble
      ├─ linux-arm64-deb      mumble.deb
      └─ linux-arm64-rpm      mumble.rpm
```

Published names carry no version or target: each file lives at
`/release/mumble/files/<sha256>/<name>`, which tells them apart, and the service
serves it with `Content-Disposition: attachment; filename="<name>"`, so a
download is saved as `mumble`, `Mumble.dmg`, and so on. The builds and the GitHub
release mirror keep versioned names (`mumble-<v>-macos-arm64.dmg`, …), which
must be unique there; `.github/release/release-service.sh` maps them.
(`MumbleSetup.exe`, not `Mumble Setup.exe`: the service's file names allow no
spaces.)

- App `mumble`, artifact `desktop`. `stable` is the public channel; every push
  to `dev` publishes `0.0.0-dev-<run number>` on the `dev` channel. `beta` is
  registered for later use.
- Every executable and archive is signed before upload (Developer ID +
  notarization on macOS, Authenticode on Windows); the service stores bytes and
  their size and SHA-256, and manifests are not signed. Linux executables
  carry an ed25519 signature over their SHA-256 as the `linux-<arch>-sig`
  target (`mumble.sig`), which the updater and installer require.
- Versions are `X.Y.Z` on stable. The service refuses an equal or lower
  version unless the current one is revoked.

## Distributions

Recorded in every release; package-manager entries gain `version` only after
that job succeeds (the `distribution` action):

| Kind | Fields | Command |
| --- | --- | --- |
| `direct` | | (updates itself) |
| `aur` | `package: mumble-bin` | `yay -Syu mumble-bin` |
| `brew` | `cask: anomalyco/tap/mumble` | `brew upgrade --cask anomalyco/tap/mumble` |
| `nix` | `attribute: github:supabitapp/mumble` | `nix profile upgrade mumble` |

## Direct-install receipt

The installers write `install.json`
(`{"app":"mumble","artifact":"desktop","channel":"stable","method":"direct","root":"<root>"}`)
at the root of the user-owned install, and the client self-updates only when
it is present and the root is writable by this user alone:

- Linux: `~/.local/share/mumble/install.json` (`install-linux.sh`;
  `scripts/install-linux.sh` source builds write none and keep their
  `source-build` marker).
- Windows: `%APPDATA%\mumble\install.json` (`install-windows.ps1` and
  the Setup.exe).
- macOS: the client decides from the bundle's location (a writable,
  non-translocated `Mumble.app` is direct).

`MUMBLE_RELEASE_BASE_URL` (HTTPS only) points a build at another service origin
for local tests against a fixture service; nothing defaults to another
origin.

## Old clients (2.3.1 and earlier)

`https://anoma.ly/hex/files/{linux-update.json,windows-update.json,appcast.xml}`
stay frozen at 2.3.1 in the `mumble-production-files` bucket. At the first
release on the service, the appcast gains one informational item pointing at
https://anoma.ly/mumble (see `docs/releasing.md`).

## Client

`src/updater.rs` (all three OSes) polls `<origin>/mumble/desktop/<channel>.json`
10 seconds after launch, hourly (the Dev channel every 10 minutes and when the window gains focus, at most once a minute), and on Check now, with `If-None-Match` and
`User-Agent: mumble/<channel>/<version>` (the build's own channel and version,
e.g. `mumble/dev/0.0.0-dev-171`, `mumble/stable/2.3.1`), which the service counts as
a running install; failed checks back off but never past hourly. It
decides with the protocol's client rules (`updater/protocol.rs`, which passes
the service's `test-vectors/` copied to `tests/fixtures/release-protocol`).
Versions are never compared: Mumble takes the channel's current release whenever
it is not the running build, identified on Linux and Windows by the
executable's SHA-256 (the target file itself) and on macOS by its exact
version. `required` releases skip the rollout. The checked channel is the
Update channel setting, else the channel the build was published on
(`MUMBLE_BUILD_CHANNEL`, stamped by Release; older builds: `dev` for a dev
version, else the receipt's). A setting other than the build's channel is a
switch: Settings, the Windows tray, and the macOS menu offer "Switch to
Stable 2.3.1", and nothing downloads until the user confirms.

```ts
install method              distribution   channel           acts on install
macOS writable Mumble.app      direct         stable            unzip beside it, codesign -R app/update-requirement.txt, swap at restart/quit
macOS read-only/translocated direct        stable            Download update → macos-arm64-dmg URL
Linux versions/ + receipt   direct         receipt's         copy into versions/<v>, switch current
Linux pacman / Nix          aur / nix      stable            show the distribution's command
Windows versions/ + receipt direct         receipt's         Authenticode (AUTHENTICODE_SIGNER), versions\<v>, current-version
source and test builds      (never check)
anything else               unknown        stable            Download update → notes_url
```

Target URLs are absolute-path references (`/release/mumble/files/<sha256>/<name>`)
resolved against the manifest URL (RFC 3986), so they keep the manifest's
host (`anoma.ly` or `dev.anoma.ly`). Downloads resume into `<support>/updates/<sha256>/` and are
checked against `size` and `sha256` before any other use. The rollout bucket
is `<support>/update-bucket`.
