#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$root/scripts/macos-signing.sh"
# MUMBLE_TEST_BUILD=1 builds the CI test app: ad-hoc signed, under its own
# name and bundle ID so it installs beside the released Mumble and never
# touches its permissions; it never checks for updates.
# MUMBLE_LOCAL_BUILD=1 marks a build made from a checkout for its owner's own
# machine (Developer ID signed as usual): it shows "Local build · updates off"
# with its commit and never checks for or offers updates.
# MUMBLE_RELEASE_UNSIGNED=1 builds the release app (Mumble, anomaly.mumble) ad-hoc
# signed: the Release workflow's build job, which holds no credentials. Its
# sign-macos job signs it with the Developer ID (.github/release/sign-macos.sh).
if [ "${MUMBLE_RELEASE_UNSIGNED:-}" = 1 ]; then
  team_id=
  identity=-
  app_name=Mumble
  bundle_id=anomaly.mumble
elif [ "${MUMBLE_TEST_BUILD:-}" = 1 ]; then
  team_id=
  identity=-
  app_name="Mumble Test"
  bundle_id=anomaly.mumble.test
else
  team_id=${MUMBLE_TEAM_ID:?Set MUMBLE_TEAM_ID to the Apple Developer signing team}
  identity=$(mumble_codesign_identity "$team_id")
  app_name=Mumble
  bundle_id=anomaly.mumble
fi
icon_output="$root/target/AppIcon.assets"
icon_info="$root/target/AppIcon-info.plist"
version=$(mumble_version "$root")
build_number=$(mumble_build_number "$version")
bundle_name="$app_name.app"
executable_name=mumble

bundle="$root/target/app/$bundle_name"
executable="$bundle/Contents/MacOS/$executable_name"

sign() {
  if [ "$identity" = - ]; then
    codesign --force --sign - "$@"
  else
    codesign --force --timestamp --options runtime --sign "$identity" "$@"
  fi
}

export MACOSX_DEPLOYMENT_TARGET=15.0
# Cargo may run through a wrapper that does not forward the environment (Kit's
# HiveMind-backed cargo), so build-time variables also travel as --config.
env_args=()
if [ -z "${MUMBLE_COMMIT:-}" ] && { [ "${MUMBLE_LOCAL_BUILD:-}" = 1 ] || [ "${MUMBLE_TEST_BUILD:-}" = 1 ]; }; then
  MUMBLE_COMMIT=$(git -C "$root" rev-parse --short=7 HEAD 2>/dev/null || true)
fi
for name in MUMBLE_LOCAL_BUILD MUMBLE_TEST_BUILD MUMBLE_COMMIT MUMBLE_BUILD_CHANNEL; do
  value=${!name:-}
  if [ -n "$value" ]; then
    case "$value" in
      *\'*) echo "$name must not contain a single quote" >&2; exit 1 ;;
    esac
    env_args+=(--config "env.$name='$value'")
  fi
done
cargo build --locked --release ${env_args[@]+"${env_args[@]}"} --manifest-path "$root/Cargo.toml"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$bundle/Contents/Library"
cp "$root/target/release/mumble" "$executable"
cp "$root/app/Info.plist" "$bundle/Contents/Info.plist"
cp "$root/LICENSE" "$root/THIRD_PARTY_NOTICES.md" "$bundle/Contents/Resources/"
MUMBLE_VERSION="$version" MUMBLE_BUILD_NUMBER="$build_number" MUMBLE_TEAM_ID="$team_id" MUMBLE_CODESIGN_IDENTITY="${identity#-}" MUMBLE_SKIP_INSTALL_SCRIPT=1 \
  "$root/scripts/build-mumble-microphone-driver.sh" "$bundle/Contents/Library" >&2
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build_number" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $bundle_id" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleExecutable $executable_name" "$bundle/Contents/Info.plist"
plutil -replace CFBundleName -string "$app_name" "$bundle/Contents/Info.plist"
plutil -replace CFBundleDisplayName -string "$app_name" "$bundle/Contents/Info.plist"
# Links into the playground (src/rune_ui/story_link.rs); a test build's own
# scheme never opens the installed Mumble.
url_scheme=mumble
[ "${MUMBLE_TEST_BUILD:-}" = 1 ] && url_scheme=mumble-test
/usr/libexec/PlistBuddy -c "Set :CFBundleURLTypes:0:CFBundleURLName $bundle_id" "$bundle/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleURLTypes:0:CFBundleURLSchemes:0 $url_scheme" "$bundle/Contents/Info.plist"
rm -rf "$icon_output"
mkdir -p "$icon_output"
xcrun actool "$root/app/AppIcon.icon" \
  --compile "$icon_output" \
  --platform macosx \
  --minimum-deployment-target 15.0 \
  --app-icon AppIcon \
  --standalone-icon-behavior all \
  --output-partial-info-plist "$icon_info" \
  --output-format human-readable-text \
  --warnings \
  --notices >&2
cp "$icon_output/AppIcon.icns" "$icon_output/Assets.car" "$bundle/Contents/Resources/"

sign "$bundle/Contents/Library/MumbleMicrophone.driver"
sign --entitlements "$root/.github/release/mumble.entitlements" "$bundle"
codesign --verify --deep --strict --verbose=2 "$bundle"
echo "$bundle"
