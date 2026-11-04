#!/bin/sh
# Publishes Mumble to the Anomaly release service through its shared client,
# vendored as release-client.sh (anomalyco/release docs/publishing.md), so
# nothing fetched at run time executes here.
# The client trades the job's GitHub OIDC token for a one-hour publish
# session, uploads in parallel (skipping files already stored), retries with
# backoff and checks the published files. Runs only in the Release
# workflow's publish jobs, which have `id-token: write`.
#
#   release-service.sh publish <dist> <notes-url>   # upload every target, then the release
#   release-service.sh distribution <kind> <version>  # a package manager now serves <version>
#
# RELEASE is https://anoma.ly/release; MUMBLE_VERSION the version; MUMBLE_RELEASE_CHANNEL
# the channel (default stable). `publish` writes <dist>/manifest.json. A rerun
# of the channel's current version succeeds when it is exactly these files.
set -eu

: "${RELEASE:?Set RELEASE to the release service origin}"
: "${MUMBLE_VERSION:?Set MUMBLE_VERSION}"
channel=${MUMBLE_RELEASE_CHANNEL:-stable}
case "$RELEASE" in
  https://anoma.ly/release) ;;
  *) echo "RELEASE must be https://anoma.ly/release." >&2; exit 1 ;;
esac

temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
client() { RELEASE=$RELEASE bash "$(dirname "$0")/release-client.sh" "$@"; }

# The package managers that mirror Mumble, each as the service lists it.
distributions='[
  {"kind": "aur", "package": "mumble-bin", "command": "yay -Syu mumble-bin"},
  {"kind": "deb-x64", "package": "mumble", "command": "curl -fLo /tmp/mumble.deb https://anoma.ly/release/mumble/desktop/stable/linux-x64-deb && sudo apt install /tmp/mumble.deb"},
  {"kind": "deb-arm64", "package": "mumble", "command": "curl -fLo /tmp/mumble.deb https://anoma.ly/release/mumble/desktop/stable/linux-arm64-deb && sudo apt install /tmp/mumble.deb"},
  {"kind": "rpm-x64", "package": "mumble", "command": "curl -fLo /tmp/mumble.rpm https://anoma.ly/release/mumble/desktop/stable/linux-x64-rpm && sudo dnf install /tmp/mumble.rpm"},
  {"kind": "rpm-arm64", "package": "mumble", "command": "curl -fLo /tmp/mumble.rpm https://anoma.ly/release/mumble/desktop/stable/linux-arm64-rpm && sudo dnf install /tmp/mumble.rpm"},
  {"kind": "brew", "cask": "anomalyco/tap/mumble", "command": "brew upgrade --cask anomalyco/tap/mumble"},
  {"kind": "nix", "attribute": "github:khoi/mumble", "command": "nix profile upgrade mumble"}
]'

publish() {
  dist=$1
  notes_url=$2
  version=$MUMBLE_VERSION
  # target=local file=published name, as docs/plans/release-service.md pins.
  # Published names carry no version or target: the sha256 in the path tells
  # files apart. A run without a platform's signed build publishes the others.
  set --
  for entry in \
    "macos-arm64=mumble-$version-macos-arm64.zip=Mumble.zip" \
    "macos-arm64-dmg=mumble-$version-macos-arm64.dmg=Mumble.dmg" \
    "windows-x64=mumble-$version-windows-x64.exe=mumble.exe" \
    "windows-x64-setup=mumble-setup-$version-windows-x64.exe=MumbleSetup.exe" \
    "windows-arm64=mumble-$version-windows-arm64.exe=mumble.exe" \
    "windows-arm64-setup=mumble-setup-$version-windows-arm64.exe=MumbleSetup.exe" \
    "linux-x64=mumble-$version-linux-x64=mumble" \
    "linux-x64-sig=mumble-$version-linux-x64.sig=mumble.sig" \
    "linux-x64-deb=mumble-$version-linux-x64.deb=mumble.deb" \
    "linux-x64-rpm=mumble-$version-linux-x64.rpm=mumble.rpm" \
    "linux-arm64=mumble-$version-linux-arm64=mumble" \
    "linux-arm64-sig=mumble-$version-linux-arm64.sig=mumble.sig" \
    "linux-arm64-deb=mumble-$version-linux-arm64.deb=mumble.deb" \
    "linux-arm64-rpm=mumble-$version-linux-arm64.rpm=mumble.rpm"
  do
    target=${entry%%=*}
    rest=${entry#*=}
    local_name=${rest%%=*}
    name=${rest#*=}
    [ -f "$dist/$local_name" ] || continue
    set -- "$@" --target "$target=$dist/$local_name=$name"
  done
  [ $# -gt 0 ] || { echo "No release files in $dist." >&2; exit 1; }
  jq -n --arg channel "$channel" --arg version "$version" --arg notes_url "$notes_url" \
    --argjson distributions "$distributions" \
    '{channel: $channel, version: $version, notes_url: $notes_url,
      distributions: ([{kind: "direct"}] + $distributions)}' > "$temporary/release.json"
  client publish --app mumble --artifact desktop --release "$temporary/release.json" \
    --out "$dist/manifest.json" --verify-downloads "$@"
}

distribution() {
  # The whole entry, so recording a version never drops its command.
  entry=$(printf '%s' "$distributions" | jq -ec --arg kind "$1" --arg version "$2" \
    'map(select(.kind == $kind))[0] // error("unknown distribution \($kind)") | . + {version: $version}')
  client distribution --app mumble --artifact desktop --channel "$channel" --entry "$entry"
}

case "${1:-}" in
  publish)
    [ $# -eq 3 ] || { echo "Usage: $0 publish <dist> <notes-url>" >&2; exit 1; }
    publish "$2" "$3"
    ;;
  distribution)
    [ $# -eq 3 ] || { echo "Usage: $0 distribution <kind> <version>" >&2; exit 1; }
    distribution "$2" "$3"
    ;;
  *)
    echo "Usage: $0 publish <dist> <notes-url> | distribution <kind> <version>" >&2
    exit 1
    ;;
esac
