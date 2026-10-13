#!/usr/bin/env bash
# Start, stop or check the loopback-only UI fixture (scripts/native-parity-fixture.mjs)
# that every client preview in this directory talks to on 127.0.0.1:3001.
# In an Amp orb it runs as the supervised service "caper-fixture"; elsewhere it
# runs in the background with a PID file. It never proxies to Caper or an SFU.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
state="${CAPER_EMULATOR_STATE:-${XDG_CACHE_HOME:-$HOME/.cache}/caper-emulators}"
health=http://127.0.0.1:3001/health

in_orb() { command -v amp >/dev/null 2>&1 && amp orb service list >/dev/null 2>&1; }
healthy() { curl --silent --fail --max-time 2 "$health" >/dev/null; }

start() {
  healthy && return 0
  if in_orb; then
    amp orb service start caper-fixture --command "node $root/scripts/native-parity-fixture.mjs" >/dev/null
  else
    mkdir -p "$state"
    nohup node "$root/scripts/native-parity-fixture.mjs" >"$state/fixture.log" 2>&1 &
    echo "$!" >"$state/fixture.pid"
  fi
  for _ in {1..50}; do healthy && return 0; sleep 0.2; done
  echo "The fixture did not answer on $health." >&2
  exit 1
}

stop() {
  if in_orb; then
    amp orb service stop caper-fixture >/dev/null 2>&1 || true
  fi
  if [[ -f "$state/fixture.pid" ]]; then
    kill "$(cat "$state/fixture.pid")" 2>/dev/null || true
    rm -f "$state/fixture.pid"
  fi
}

case "${1:-start}" in
  start) start && echo "Fixture ready at http://127.0.0.1:3001" ;;
  stop) stop ;;
  status) healthy && echo running || { echo stopped; exit 1; } ;;
  *) echo "Usage: $0 start|stop|status" >&2; exit 2 ;;
esac
