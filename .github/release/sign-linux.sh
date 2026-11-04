#!/usr/bin/env bash
# sign-linux.sh <dist>: signs each Linux executable in <dist>
# (mumble-<version>-linux-<x64|arm64>) with Anomaly's ed25519 release key,
# MUMBLE_RELEASE_SIGNING_KEY (PEM), and writes the raw 64-byte signature beside
# it as <file>.sig. The signed message is
#   "anomaly.mumble.linux-artifact.v1\n<sha256 of the file>\n"
# Mumble's updater (src/updater/linux.rs) and install-linux.sh verify it with the
# public key in linux-signing.pub, which this script checks every signature
# against first, so a mismatched key fails the run before anything publishes.
. "$(dirname "$0")/lib.sh"

dist=$(posix "$1")
release=$(cd "$(dirname "$0")" && pwd)
: "${MUMBLE_RELEASE_SIGNING_KEY:?Set MUMBLE_RELEASE_SIGNING_KEY}"
: "${MUMBLE_VERSION:?Set MUMBLE_VERSION}"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
(umask 077 && printf '%s\n' "$MUMBLE_RELEASE_SIGNING_KEY" > "$temporary/key.pem")
signed=0
for arch in x64 arm64; do
  file="$dist/mumble-$MUMBLE_VERSION-linux-$arch"
  [ -f "$file" ] || continue
  sha=$(sha256 "$file" | cut -c1-64)
  printf 'anomaly.mumble.linux-artifact.v1\n%s\n' "$sha" > "$temporary/message"
  openssl pkeyutl -sign -rawin -inkey "$temporary/key.pem" -in "$temporary/message" -out "$file.sig" ||
    die "Could not sign $(basename "$file") with MUMBLE_RELEASE_SIGNING_KEY."
  [ "$(wc -c < "$file.sig" | tr -d ' ')" = 64 ] || die "The signature of $(basename "$file") is not 64 bytes."
  openssl pkeyutl -verify -rawin -pubin -inkey "$release/linux-signing.pub" \
    -in "$temporary/message" -sigfile "$file.sig" > /dev/null ||
    die "MUMBLE_RELEASE_SIGNING_KEY does not match linux-signing.pub, the key Mumble trusts."
  echo "Signed $(basename "$file") ($sha)."
  signed=$((signed + 1))
done
[ "$signed" -gt 0 ] || die "No Linux executables in $dist to sign."
