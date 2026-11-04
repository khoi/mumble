#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export MUMBLE_TEAM_ID=TESTTEAM00
temporary=$(mktemp -d "${TMPDIR:-/tmp}/mumble-app-identity.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
bundle="$temporary/Mumble.app"
mkdir -p "$bundle/Contents"
plist="$bundle/Contents/Info.plist"
cp "$root/app/Info.plist" "$plist"

[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$plist")" = anomaly.mumble ]
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")" = mumble ]
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleDisplayName' "$plist")" = Mumble ]
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$plist")" = Mumble ]

expect_rejection() {
  if output=$(sh "$root/scripts/validate-app.sh" "$bundle" Mumble.app 2.1.0 20100 2>&1); then
    echo "Unexpectedly accepted an incompatible app." >&2
    exit 1
  fi
  printf '%s\n' "$output" | grep -Fq "$1"
}

/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.kitlangton.Hex' "$plist"
expect_rejection 'wrong bundle identifier'
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.kitlangton.hex.agent' "$plist"
expect_rejection 'wrong bundle identifier'
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier ly.anoma.Hex' "$plist"
expect_rejection 'wrong bundle identifier'
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.kitlangton.hex2' "$plist"
expect_rejection 'wrong bundle identifier'
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier anomaly.mumble' "$plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleExecutable mumble-watch' "$plist"
expect_rejection 'wrong executable name'
/usr/libexec/PlistBuddy -c 'Set :CFBundleExecutable mumble' "$plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleShortVersionString 2.1.0' "$plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleVersion 20100' "$plist"
mkdir -p "$bundle/Contents/MacOS"
printf '#!/bin/sh\n' >"$bundle/Contents/MacOS/mumble"
chmod +x "$bundle/Contents/MacOS/mumble"
expect_rejection 'packaged executable is not arm64-only'

for script in build-app.sh release-app.sh; do
  if output=$(env -u MUMBLE_TEAM_ID sh "$root/scripts/$script" 2>&1); then
    echo "Unexpectedly allowed $script without an explicit signing team." >&2
    exit 1
  fi
  printf '%s\n' "$output" | grep -Fq 'Set MUMBLE_TEAM_ID to the Apple Developer signing team'
done

for identity in 'Developer ID Application: Fixture (WRONGTEAM0)' 'Apple Development: Fixture (TESTTEAM00)'; do
  if output=$(MUMBLE_CODESIGN_IDENTITY="$identity" sh "$root/scripts/build-app.sh" 2>&1); then
    echo "Unexpectedly allowed an incompatible signing identity in build-app.sh." >&2
    exit 1
  fi
  printf '%s\n' "$output" | grep -Fq 'Signing identity is not a Developer ID Application identity for team TESTTEAM00'
done

echo "Rust app identity guards passed."
