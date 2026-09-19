#!/usr/bin/env bash
# kpop.db を SQL ダンプして kpop-tui-data リポジトリに push する（変更があるときのみ）
# 復元: python3 -c "import sqlite3,sys; sqlite3.connect('kpop.db').executescript(open('kpop.sql').read())"
set -euo pipefail

SRC_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DB_PATH="${KPOP_DB:-$SRC_DIR/kpop.db}"
DATA_REPO="${KPOP_DATA_REPO:-$(dirname "$SRC_DIR")/kpop-tui-data}"

if [ ! -f "$DB_PATH" ]; then
    echo "backup-data: DB not found: $DB_PATH" >&2
    exit 1
fi
if [ ! -d "$DATA_REPO/.git" ]; then
    echo "backup-data: data repo not found: $DATA_REPO" >&2
    exit 1
fi

# 一時ファイルに書いてから置き換える（途中で失敗しても前回のダンプを壊さない）
tmp="$DATA_REPO/.kpop.sql.tmp"
python3 - "$DB_PATH" "$tmp" <<'PY'
import sqlite3, sys
src = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
with open(sys.argv[2], "w", encoding="utf-8") as f:
    for line in src.iterdump():
        f.write(line + "\n")
src.close()
PY
mv "$tmp" "$DATA_REPO/kpop.sql"

cd "$DATA_REPO"
git add kpop.sql
if git diff --cached --quiet; then
    echo "backup-data: no changes"
    exit 0
fi

git commit -q -m "backup: $(date '+%Y-%m-%d %H:%M:%S')"
git push -q origin HEAD
echo "backup-data: pushed"
