#!/usr/bin/env bash
# kpop-tui のブラウザ版を起動し、tailnet 内に公開する。
#
#   ./scripts/kpop-web.sh            起動 (既定ポート 8787)
#   ./scripts/kpop-web.sh 9000       ポート指定
#   ./scripts/kpop-web.sh --stop     tailscale serve の公開を解除
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release/kpop-web"

# sudo が要るかどうかは operator 設定次第
ts() {
  if tailscale serve status >/dev/null 2>&1; then
    tailscale "$@"
  else
    sudo tailscale "$@"
  fi
}

if [[ "${1:-}" == "--stop" ]]; then
  ts serve reset
  echo "tailscale serve を解除しました。"
  exit 0
fi

PORT="${1:-8787}"

if [[ ! -x "$BIN" ]]; then
  echo "kpop-web をビルドします…"
  (cd "$ROOT" && cargo build --release --bin kpop-web)
fi

# tailnet 内にHTTPSで公開（localhost:PORT へプロキシ）
ts serve --bg "$PORT" >/dev/null
HOST="$(tailscale status --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))')"

echo "──────────────────────────────────────────────"
echo "  tailnet:  https://$HOST/"
echo "  local:    http://127.0.0.1:$PORT/"
echo "  停止:     Ctrl+C  /  公開解除: $0 --stop"
echo "──────────────────────────────────────────────"

exec "$BIN" --addr "127.0.0.1:$PORT"
