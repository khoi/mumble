#!/bin/sh
set -eu

# Launches a packaged Mumble.app with throwaway Application Support for a few
# seconds and fails if it exits early.
# Permission prompts and onboarding may appear; nothing is granted or kept.
if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <Mumble.app path>" >&2
  exit 1
fi

bundle=$1
seconds=${MUMBLE_SMOKE_SECONDS:-15}
support=$(mktemp -d "${TMPDIR:-/tmp}/mumble-smoke.XXXXXX")
pid=
trap '[ -z "$pid" ] || kill -KILL "$pid" 2>/dev/null || true; rm -rf "$support"' EXIT HUP INT TERM

fail() {
  echo "Mumble smoke test failed: $*" >&2
  cat "$support/logs/process.log" >&2 2>/dev/null || cat "$support/output.log" >&2 || true
  exit 1
}

MUMBLE_APPLICATION_SUPPORT_DIR="$support" "$bundle/Contents/MacOS/mumble" > "$support/output.log" 2>&1 &
pid=$!
sleep "$seconds"
if ! kill -0 "$pid" 2>/dev/null; then
  status=0
  wait "$pid" || status=$?
  pid=
  fail "exited with status $status within $seconds seconds"
fi
[ -s "$support/logs/process.log" ] || fail "wrote no process log"

kill -INT "$pid"
waited=0
while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 10 ]; do
  sleep 1
  waited=$((waited + 1))
done
if kill -0 "$pid" 2>/dev/null; then
  echo "Mumble was still running 10 seconds after SIGINT; killed it." >&2
fi
echo "Mumble ran for $seconds seconds from $bundle."
