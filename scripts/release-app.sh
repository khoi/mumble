#!/bin/sh
set -eu

# Builds the release Mumble.app from a clean, pushed commit, signs it with the
# Developer ID of MUMBLE_TEAM_ID, notarizes and staples the app and
# its DMG, and leaves the two release files the Release workflow publishes
# to the Anomaly release service (docs/plans/release-service.md):
#   dist/mumble-<version>-macos-arm64.dmg   download (target macos-arm64-dmg)
#   dist/mumble-<version>-macos-arm64.zip   in-app update, root Mumble.app (target macos-arm64)
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
: "${MUMBLE_TEAM_ID:?Set MUMBLE_TEAM_ID to the Apple Developer signing team}"
. "$root/scripts/macos-signing.sh"
version=$(mumble_version "$root")
build_number=$(mumble_build_number "$version")
dist="$root/dist"
dmg="$dist/mumble-$version-macos-arm64.dmg"
zip="$dist/mumble-$version-macos-arm64.zip"

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != prepare ]; }; then
  echo "Usage: $0 [prepare]" >&2
  exit 1
fi
if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before releasing." >&2
  exit 1
fi
# GitHub Actions checks out the exact pushed commit, detached from any branch.
if [ "${GITHUB_ACTIONS:-}" != true ]; then
  upstream=$(git -C "$root" rev-parse --abbrev-ref '@{upstream}' 2>/dev/null || true)
  if [ -z "$upstream" ] || [ "$(git -C "$root" rev-parse HEAD)" != "$(git -C "$root" rev-parse "$upstream")" ]; then
    echo "Push the release commit to its upstream branch before releasing." >&2
    exit 1
  fi
fi

# Staging builds publish a prerelease (X.Y.Z-dev.N); the binary must report it.
"$root/scripts/set-crate-version.sh" "$root" "$version"
export MUMBLE_VERSION="$version" MUMBLE_BUILD_NUMBER="$build_number"
bundle=$("$root/scripts/build-app.sh")
"$root/scripts/validate-app.sh" "$bundle" Mumble.app "$version" "$build_number" >/dev/null
mkdir -p "$dist"
rm -f "$dmg" "$zip"
"$root/scripts/notarize-app.sh" "$bundle" "$dmg" Mumble
# The updater unpacks this beside the installed bundle and swaps it in.
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bundle" "$zip"

echo "$dmg"
echo "$zip"
