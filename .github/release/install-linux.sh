#!/bin/sh
set -eu

# Installs Mumble from the Anomaly release service (docs/plans/release-service.md):
# the stable manifest names the linux-x64 or linux-arm64 executable with its
# size and SHA-256, and install.json marks the install as one Mumble updates itself.
# Anomaly's ed25519 Linux release key (.github/release/linux-signing.pub):
# every executable must carry its signature as the release's <target>-sig
# file, over "anomaly.mumble.linux-artifact.v1\n<sha256>\n".
signing_key=MCowBQYDK2VwAyEAv60C5iII/xRLXJ0hx+ecfBbGkEKZpDfYVzAwB81P99g=
origin=${MUMBLE_RELEASE_BASE_URL:-https://anoma.ly/release}
origin=${origin%/}
# MUMBLE_INSTALL_CHANNEL=dev installs the newest dev build and keeps the
# install on the Dev update channel; =stable requires the stable release.
# Unset, it installs stable, or the newest dev build while the release
# service has no stable Mumble release yet.
channel=${MUMBLE_INSTALL_CHANNEL:-}
case "$channel" in
  '' | stable | dev) ;;
  *)
    echo "MUMBLE_INSTALL_CHANNEL must be stable or dev." >&2
    exit 1
    ;;
esac
install_dir=${MUMBLE_INSTALL_DIR:-"$HOME/.local/bin"}
data_home=${XDG_DATA_HOME:-$HOME/.local/share}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
support_dir=${MUMBLE_APPLICATION_SUPPORT_DIR:-"$data_home/mumble"}
applications="$data_home/applications"
autostart="$config_home/autostart"
units="$config_home/systemd/user"
unit="$units/mumble.service"

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Mumble installation requires $1." >&2
    exit 1
  fi
}

fail() {
  echo "$1" >&2
  exit 1
}

# fetch <max bytes> <max seconds> <url> <file> [curl option...]: HTTPS only,
# and never more than one byte past <max bytes> on disk. Before 8.4.0 (Debian
# 12 has 7.88), curl enforces --max-filesize only when the server announces
# the length, so head caps the transfer too; a cut-off transfer fails, and
# callers check the exact size afterwards.
fetch() {
  limit=$1 seconds=$2 url=$3 file=$4
  shift 4
  rm -f "$temporary/fetch-failed"
  {
    curl --location --proto '=https' --proto-redir '=https' \
      --silent --show-error --connect-timeout 10 --max-time "$seconds" \
      --max-filesize "$limit" "$@" "$url" || : > "$temporary/fetch-failed"
  } | head -c "$((limit + 1))" > "$file"
  if [ -e "$temporary/fetch-failed" ]; then
    rm -f "$temporary/fetch-failed"
    return 1
  fi
}

# download <max bytes> <max seconds> <url> <file>: fetch, failing on an HTTP
# error.
download() {
  fetch "$@" --fail
}

safe_user_directory() {
  path=$1
  if [ -L "$path" ]; then
    fail "Refusing directory symlink: $path"
  fi
  mkdir -p "$path"
  if [ "$(stat -c %u "$path")" != "$(id -u)" ]; then
    fail "Mumble requires a user-owned directory: $path"
  fi
  if find "$path" -maxdepth 0 -perm /022 -print -quit | grep -q .; then
    fail "Mumble refuses a group- or world-writable directory: $path"
  fi
}

managed_directory() {
  path=$1
  if [ -L "$path" ]; then
    fail "Refusing managed symlink: $path"
  fi
  mkdir -p "$path"
  if [ "$(stat -c %u "$path")" != "$(id -u)" ]; then
    fail "Mumble requires a user-owned directory: $path"
  fi
  chmod 700 "$path"
}

# Mumble's desktop entries: its own, or the autostart entry the desktop's
# Background portal writes for it (#812).
mumble_entry() {
  grep -Fxq 'X-Mumble-Managed=true' "$1" || grep -Fxq 'X-XDP-Autostart=anomaly.mumble' "$1"
}

uninstall() {
  if [ -f "$unit" ] && grep -Fxq '# X-Mumble-Managed=true' "$unit"; then
    if command -v systemctl >/dev/null 2>&1; then
      active_unit=$(systemctl --user show mumble.service --property=FragmentPath --value 2>/dev/null || true)
      if [ "$active_unit" = "$unit" ]; then
        systemctl --user disable --now mumble.service || fail "Could not stop mumble.service; refusing to uninstall a running service."
      fi
    fi
    rm -f "$unit"
    if command -v systemctl >/dev/null 2>&1; then
      systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
  fi
  if [ -L "$support_dir" ]; then
    fail "Refusing managed symlink: $support_dir"
  fi
  if [ -e "$support_dir" ]; then
    managed_directory "$support_dir"
    managed_directory "$support_dir/updates"
    exec 9> "$support_dir/updates/update.lock"
    if ! flock -n 9; then
      fail "A Mumble install or update is already in progress."
    fi
  fi
  binary="$install_dir/mumble"
  expected="$support_dir/current/mumble"
  if [ -L "$binary" ] && [ "$(readlink "$binary")" = "$expected" ]; then
    rm -f "$binary"
  fi
  for desktop in \
    "$applications/anomaly.mumble.desktop" \
    "$autostart/anomaly.mumble.desktop" \
    "$applications/mumble.desktop" \
    "$autostart/mumble.desktop"
  do
    if [ -f "$desktop" ] && [ ! -L "$desktop" ] && mumble_entry "$desktop"; then
      rm -f "$desktop"
    fi
  done
  # The icons Mumble writes into the user's theme when no package ships them,
  # marked by a PNG text chunk.
  for icon in "$data_home"/icons/hicolor/*/apps/anomaly.mumble.png; do
    if [ -f "$icon" ] && [ ! -L "$icon" ] && LC_ALL=C grep -Faq 'X-Mumble-Managed=true' "$icon"; then
      rm -f "$icon"
    fi
  done
  if [ -d "$data_home/icons/hicolor" ] && [ ! -L "$data_home/icons/hicolor" ]; then
    touch "$data_home/icons/hicolor" 2>/dev/null || true
  fi
  rm -f "$support_dir/current" "$support_dir/.current.next" "$support_dir/install.json"
  rm -rf "$support_dir/versions"
  echo "Removed Mumble. Logs and settings remain in $support_dir."
}

case "$(uname -s) $(uname -m)" in
  "Linux x86_64") target=linux-x64 ;;
  "Linux aarch64") target=linux-arm64 ;;
  *)
    echo "Mumble supports x86_64 and aarch64 Linux." >&2
    exit 1
    ;;
esac
if [ "$(id -u)" -eq 0 ]; then
  echo "Install Mumble as your desktop user, not root." >&2
  exit 1
fi
umask 022
for command in find flock grep readlink stat; do
  require "$command"
done

case ${1:-install} in
  install) ;;
  uninstall)
    uninstall
    exit 0
    ;;
  *)
    echo "Usage: $0 [install|uninstall]" >&2
    exit 1
    ;;
esac

for command in sha256sum sort sync; do
  require "$command"
done

temporary=$(mktemp -d)
application_staged=
autostart_staged=
unit_staged=
binary_staged=
current_staged=
version_staged=
cleanup() {
  rm -rf "$temporary"
  for path in \
    "$application_staged" \
    "$autostart_staged" \
    "$unit_staged" \
    "$binary_staged" \
    "$current_staged" \
    "$version_staged"
  do
    if [ -n "$path" ]; then
      rm -f "$path"
    fi
  done
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
manifest="$temporary/manifest.json"
artifact_path=${MUMBLE_INSTALL_ARTIFACT:-$temporary/mumble}
if [ -n "${MUMBLE_INSTALL_ARTIFACT:-}" ] && [ -z "$channel" ]; then channel=stable; fi
version=${MUMBLE_INSTALL_VERSION:-}

if [ -z "${MUMBLE_INSTALL_ARTIFACT:-}" ]; then
  for command in curl jq openssl; do
    require "$command"
  done
  case "$origin" in
    https://*) ;;
    *) fail "MUMBLE_RELEASE_BASE_URL must be an https:// URL." ;;
  esac
  if [ -z "$channel" ]; then
    fetch 1048576 60 "$origin/mumble/desktop/stable.json" "$manifest" \
      --dump-header "$temporary/stable-headers" \
      || fail "Could not reach $origin."
    status=$(awk '/^HTTP\// { code = $2 } END { print code }' "$temporary/stable-headers")
    case "$status" in
      200) channel=stable ;;
      404)
        echo "Mumble has no stable release yet; installing the newest dev build, which keeps updating from the Dev channel. Set MUMBLE_INSTALL_CHANNEL=stable to require the stable release." >&2
        channel=dev
        ;;
      *) fail "$origin/mumble/desktop/stable.json answered $status." ;;
    esac
  fi
  manifest_url="$origin/mumble/desktop/$channel.json"
  download 1048576 60 "$manifest_url" "$manifest" || fail "Could not download $manifest_url."
  test "$(jq -r '"\(.schema) \(.app) \(.artifact) \(.channel)"' "$manifest")" = "1 mumble desktop $channel" \
    || fail "$manifest_url is not the Mumble desktop $channel manifest."
  version=$(jq -er .release.version "$manifest")
  url=$(jq -er --arg target "$target" '.release.targets[$target].url' "$manifest") \
    || fail "Mumble $version has no $target build."
  bytes=$(jq -er --arg target "$target" '.release.targets[$target].size' "$manifest")
  sha256=$(jq -er --arg target "$target" '.release.targets[$target].sha256' "$manifest")
  printf '%s\n' "$bytes" | grep -Eq '^[1-9][0-9]*$' \
    || fail "Linux release has an invalid size."
  printf '%s\n' "$sha256" | grep -Eq '^[0-9a-f]{64}$' \
    || fail "Linux release has an invalid checksum."
  test "$bytes" -le 536870912 || fail "Linux release exceeds the 512 MiB limit."
  # Target URLs are absolute paths on the manifest's host, under the app's
  # files (anomalyco/release docs/protocol.md).
  printf '%s\n' "$url" | grep -Eq '^/([A-Za-z0-9._-]+/)*mumble/files/[0-9a-f]{64}/[A-Za-z0-9._-]+$' \
    || fail "Linux release names an unexpected file URL."
  host=$(printf '%s\n' "$origin" | sed -E 's#^([a-z]+://[^/]+).*#\1#')

  download "$bytes" 600 "$host$url" "$artifact_path" || fail "Could not download Mumble $version."
  test "$(wc -c < "$artifact_path" | tr -d ' ')" = "$bytes" \
    || fail "Downloaded Linux release has the wrong size."
  test "$(sha256sum "$artifact_path" | awk '{ print $1 }')" = "$sha256" \
    || fail "Downloaded Linux release checksum does not match."

  # The signature ties the manifest's SHA-256 to Anomaly's release key, so a
  # tampered manifest cannot name another executable.
  signature_url=$(jq -er --arg target "$target-sig" '.release.targets[$target].url' "$manifest") \
    || fail "Mumble $version has no signature for $target."
  signature_bytes=$(jq -er --arg target "$target-sig" '.release.targets[$target].size' "$manifest")
  signature_sha256=$(jq -er --arg target "$target-sig" '.release.targets[$target].sha256' "$manifest")
  test "$signature_bytes" = 64 || fail "Linux release has an invalid signature."
  printf '%s\n' "$signature_url" | grep -Eq '^/([A-Za-z0-9._-]+/)*mumble/files/[0-9a-f]{64}/[A-Za-z0-9._-]+$' \
    || fail "Linux release names an unexpected signature URL."
  download 64 60 "$host$signature_url" "$temporary/mumble.sig" || fail "Could not download the Mumble $version signature."
  test "$(sha256sum "$temporary/mumble.sig" | awk '{ print $1 }')" = "$signature_sha256" \
    || fail "Downloaded Linux release signature checksum does not match."
  printf -- '-----BEGIN PUBLIC KEY-----\n%s\n-----END PUBLIC KEY-----\n' "$signing_key" > "$temporary/signing.pem"
  printf 'anomaly.mumble.linux-artifact.v1\n%s\n' "$sha256" > "$temporary/signed"
  openssl pkeyutl -verify -rawin -pubin -inkey "$temporary/signing.pem" \
    -in "$temporary/signed" -sigfile "$temporary/mumble.sig" > /dev/null 2>&1 \
    || fail "Linux release signature does not verify; refusing to install it."
  chmod 755 "$artifact_path"
fi
# The version is display text that names a directory, never compared.
printf '%s\n' "$version" | grep -Eq '^[0-9A-Za-z][0-9A-Za-z.+_-]{0,127}$' \
  || fail "Linux release has an invalid version."
test -x "$artifact_path" || fail "Mumble executable is missing: $artifact_path"

# Explain why the downloaded executable cannot start: a glibc older than the
# one it was built against, or the Arch packages behind missing libraries.
explain_unrunnable() {
  needed=$(grep -ao 'GLIBC_2\.[0-9][0-9]*' "$artifact_path" 2>/dev/null \
    | sed 's/^GLIBC_//' | sort -uV | tail -1)
  have=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{ print $2 }')
  if [ -n "$needed" ] && [ -n "$have" ] &&
    [ "$(printf '%s\n%s\n' "$needed" "$have" | sort -V | head -1)" != "$needed" ]; then
    fail "Mumble $version needs glibc $needed or newer, and this system has glibc $have. The Linux beta needs Debian 12, Ubuntu 23.04, Fedora 37, or a newer or rolling distribution."
  fi
  packages=
  if command -v ldd >/dev/null 2>&1; then
    for library in $(ldd "$artifact_path" 2>/dev/null | awk '/not found/ { print $1 }'); do
      case "$library" in
        libasound.*) package=alsa-lib ;;
        libopenblas.* | libblas.*) package=openblas ;;
        libvulkan.*) package=vulkan-icd-loader ;;
        libxkbcommon-x11.*) package=libxkbcommon-x11 ;;
        libxkbcommon.*) package=libxkbcommon ;;
        libX11.* | libX11-xcb.*) package=libx11 ;;
        libxcb*) package=libxcb ;;
        libXcursor.*) package=libxcursor ;;
        libXi.*) package=libxi ;;
        libXrandr.*) package=libxrandr ;;
        libfontconfig.*) package=fontconfig ;;
        libfreetype.*) package=freetype2 ;;
        libwayland-*) package=wayland ;;
        libudev.* | libsystemd.*) package=systemd-libs ;;
        libssl.* | libcrypto.*) package=openssl ;;
        *) package=$library ;;
      esac
      case " $packages " in
        *" $package "*) ;;
        *) packages="$packages $package" ;;
      esac
    done
  fi
  if [ -n "$packages" ]; then
    fail "Mumble needs libraries that are not installed. Install them, then run this installer again:
  sudo pacman -S --needed$packages"
  fi
  fail "Mumble could not run. Install the required Arch runtime libraries."
}

# The pipeline's status is awk's, so an executable that cannot start shows up
# as an empty version.
reported_version=$("$artifact_path" --version 2>/dev/null \
  | awk 'NF { value=$NF } END { print value }') || true
if [ -z "$reported_version" ]; then
  explain_unrunnable
fi
test "$reported_version" = "$version" \
  || fail "Downloaded executable reports the wrong version."

managed_directory "$support_dir"
managed_directory "$support_dir/versions"
managed_directory "$support_dir/updates"
exec 9> "$support_dir/updates/update.lock"
if ! flock -n 9; then
  echo "A Mumble install or update is already in progress." >&2
  exit 1
fi
current="$support_dir/current"
# Installs what was asked for, whatever version is installed now: versions
# are never compared.
if [ -e "$current" ] || [ -L "$current" ]; then
  if [ ! -L "$current" ]; then
    fail "Refusing unmanaged current version path: $current"
  fi
  readlink "$current" | grep -Eq '^versions/[0-9A-Za-z][0-9A-Za-z.+_-]{0,127}$' \
    || fail "Installed Mumble has an invalid current version link."
fi
version_dir="$support_dir/versions/$version"
managed_directory "$version_dir"

safe_user_directory "$install_dir"
safe_user_directory "$applications"
safe_user_directory "$autostart"
safe_user_directory "$units"
binary="$install_dir/mumble"
if printf '%s' "$binary" | LC_ALL=C grep -Eq '["\\[:cntrl:]]'; then
  fail "Mumble install path contains unsupported characters."
fi
expected_binary="$support_dir/current/mumble"
if printf '%s' "$support_dir" | LC_ALL=C grep -Eq '["\\[:cntrl:]]'; then
  fail "Mumble service path contains unsupported characters."
fi
if [ -L "$unit" ] || { [ -e "$unit" ] && ! grep -Fxq '# X-Mumble-Managed=true' "$unit"; }; then
  fail "Refusing to replace an unmanaged systemd unit: $unit"
fi
if { [ -e "$binary" ] || [ -L "$binary" ]; } \
  && { [ ! -L "$binary" ] || [ "$(readlink "$binary")" != "$expected_binary" ]; }
then
  fail "Refusing to replace an unmanaged file: $binary"
fi
if [ -e "$support_dir/current" ] && [ ! -L "$support_dir/current" ]; then
  fail "Refusing to replace an unmanaged file: $support_dir/current"
fi
# Named for Mumble's app id, which its windows and the desktop portal use.
application_entry="$applications/anomaly.mumble.desktop"
autostart_entry="$autostart/anomaly.mumble.desktop"
for desktop in "$application_entry" "$autostart_entry"; do
  if [ -L "$desktop" ]; then
    fail "Refusing to replace a desktop-entry symlink: $desktop"
  fi
  if [ -e "$desktop" ] && ! mumble_entry "$desktop"; then
    fail "Refusing to replace an unmanaged desktop entry: $desktop"
  fi
done

application_staged=$(mktemp "$applications/.anomaly.mumble.desktop.XXXXXX")
autostart_staged=$(mktemp "$autostart/.anomaly.mumble.desktop.XXXXXX")
unit_staged=$(mktemp "$units/.mumble.service.XXXXXX")
umask 077
cat > "$application_staged" <<EOF
[Desktop Entry]
Type=Application
Name=Mumble
Comment=Local voice dictation
Exec="$binary"
Icon=anomaly.mumble
Terminal=false
Categories=Utility;Accessibility;
StartupNotify=true
StartupWMClass=anomaly.mumble
Actions=quit;
X-Mumble-Managed=true

[Desktop Action quit]
Name=Quit Mumble
Exec="$binary" quit
EOF
cat > "$autostart_staged" <<EOF
[Desktop Entry]
Type=Application
Name=Mumble
Comment=Local voice dictation
Exec="$binary" start
Icon=anomaly.mumble
Terminal=false
X-GNOME-Autostart-enabled=true
X-Mumble-Managed=true
EOF
chmod 644 "$application_staged" "$autostart_staged"

# systemd expands percent specifiers, and ExecStart also expands dollar signs.
unit_binary=$(printf '%s' "$binary" | sed -e 's/%/%%/g' -e 's/\$/$$/g')
unit_support=$(printf '%s' "$support_dir" | sed 's/%/%%/g')
cat > "$unit_staged" <<EOF
# X-Mumble-Managed=true
[Unit]
Description=Mumble background dictation
PartOf=graphical-session.target
After=graphical-session.target
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
Type=notify
NotifyAccess=main
ExecStart="$unit_binary" service
Environment="MUMBLE_APPLICATION_SUPPORT_DIR=$unit_support"
EnvironmentFile=-$unit_support/session.env
Restart=on-failure
RestartSec=3
TimeoutStopSec=30
UMask=0077

[Install]
WantedBy=graphical-session.target
EOF
chmod 644 "$unit_staged"

version_staged="$version_dir/.mumble.install.$$"
current_staged="$support_dir/.current.next"
binary_staged="$install_dir/.mumble.next"
install -m755 "$artifact_path" "$version_staged"
sync -f "$version_staged"
mv -Tf "$version_staged" "$version_dir/mumble"
version_staged=
# A build from a checkout updates from that checkout, never from the
# release service.
if [ "${MUMBLE_INSTALL_SOURCE:-}" = 1 ]; then
  : > "$version_dir/source-build"
else
  rm -f "$version_dir/source-build"
fi
sync -f "$version_dir"
rm -f "$current_staged" "$binary_staged"
ln -s "versions/$version" "$current_staged"
mv -Tf "$current_staged" "$support_dir/current"
current_staged=
# The receipt that lets Mumble update itself; the support path was checked for
# quotes, backslashes, and control characters above.
if [ "${MUMBLE_INSTALL_SOURCE:-}" = 1 ]; then
  rm -f "$support_dir/install.json"
else
  printf '{"app":"mumble","artifact":"desktop","channel":"%s","method":"direct","root":"%s"}\n' \
    "$channel" "$support_dir" > "$support_dir/.install.json.next"
  mv -Tf "$support_dir/.install.json.next" "$support_dir/install.json"
fi
sync -f "$support_dir"
ln -s "$support_dir/current/mumble" "$binary_staged"
mv -Tf "$binary_staged" "$binary"
binary_staged=
sync -f "$install_dir"
mv -Tf "$application_staged" "$application_entry"
application_staged=
mv -Tf "$autostart_staged" "$autostart_entry"
autostart_staged=
# Earlier installs named both entries mumble.desktop.
for desktop in "$applications/mumble.desktop" "$autostart/mumble.desktop"; do
  if [ -f "$desktop" ] && [ ! -L "$desktop" ] && grep -Fxq 'X-Mumble-Managed=true' "$desktop"; then
    rm -f "$desktop"
  fi
done
mv -Tf "$unit_staged" "$unit"
unit_staged=
sync -f "$applications"
sync -f "$autostart"
sync -f "$units"
if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$applications" >/dev/null 2>&1 || true
fi

printf 'Installed Mumble %s to %s\n' "$version" "$binary"
printf 'Install a model: %s model install\n' "$binary"
printf 'Launch Mumble: %s\n' "$binary"
printf 'Start in the background: %s start (quit with %s quit)\n' "$binary" "$binary"
printf 'Uninstall: sh install-linux.sh uninstall\n'
