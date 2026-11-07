#!/usr/bin/env bash
# assemble.sh <in> <dist>: gathers the release files from the verified
# artifacts in <in>/<name>/ into <dist>, refusing any file that is not one of
# the release service targets for MUMBLE_VERSION (docs/plans/release-service.md)
# or that two artifacts both hold.
. "$(dirname "$0")/lib.sh"

in=$(posix "$1")
dist=$(posix "$2")
v=$MUMBLE_VERSION
mkdir -p "$dist"
for file in "$in"/*/*; do
  name=$(basename "$file")
  [ "$name" = SHA256SUMS ] && continue
  case "$name" in
    "mumble-$v-linux-x64" | "mumble-$v-linux-x64.deb" | "mumble-$v-linux-x64.rpm" | \
      "mumble-$v-linux-arm64" | "mumble-$v-linux-arm64.deb" | "mumble-$v-linux-arm64.rpm" | \
      "mumble-$v-macos-arm64.zip" | "mumble-$v-macos-arm64.dmg" | \
      "mumble-$v-windows-x64.exe" | "mumble-$v-windows-arm64.exe" | \
      "mumble-$v-windows-x64.com" | "mumble-$v-windows-arm64.com" | \
      "mumble-setup-$v-windows-x64.exe" | "mumble-setup-$v-windows-arm64.exe") ;;
    *) die "$name is not a release file of Mumble $v." ;;
  esac
  [ ! -e "$dist/$name" ] || die "Two artifacts hold $name."
  cp "$file" "$dist/$name"
done
(cd "$dist" && ls -l && sha256 -- *) | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
