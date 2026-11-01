#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ -f "$HOME/.cargo/env" ]; then
  # Load rustup environment when present.
  . "$HOME/.cargo/env"
fi

mode="${1:-run}"

start_server() {
  export EXAM_WEB_DIR="${EXAM_WEB_DIR:-$ROOT/web/dist}"
  exec cargo run -p exam-server
}

case "$mode" in
  dev)
    cargo run -p exam-server &
    server_pid=$!
    trap 'kill "$server_pid"' EXIT
    cd "$ROOT/web"
    npm install
    npm run dev
    ;;
  build)
    cd "$ROOT/web"
    npm install
    npm run build
    ;;
  run)
    if [ ! -f "$ROOT/web/dist/teacher.html" ]; then
      cd "$ROOT/web"
      npm install
      npm run build
      cd "$ROOT"
    fi
    start_server
    ;;
  *)
    echo "usage: $0 [dev|build|run]"
    exit 1
    ;;
esac
