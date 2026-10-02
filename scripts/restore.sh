#!/usr/bin/env bash
set -Eeuo pipefail
[[ $# -eq 1 ]] || { echo "Usage: $0 /absolute/path/to/backup.tar.gz" >&2; exit 2; }
archive="$1"
[[ "$archive" = /* && -f "$archive" ]] || { echo "Provide an existing absolute archive path" >&2; exit 2; }
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
COMPOSE_FILE="${COMPOSE_FILE:-$ROOT_DIR/docker-compose.yml}"
DATA_DIR="${WOODSHOP_DATA_DIR:-$ROOT_DIR/data}"
[[ "$DATA_DIR" = /* ]] || { echo "WOODSHOP_DATA_DIR must be an absolute path" >&2; exit 2; }
compose=(docker compose --project-directory "$ROOT_DIR" -f "$COMPOSE_FILE")
[[ -d "$DATA_DIR" ]] || { echo "Missing data directory: $DATA_DIR" >&2; exit 1; }
[[ ! -L "$DATA_DIR" && "$DATA_DIR" != / && "$DATA_DIR" != "$ROOT_DIR" && "$DATA_DIR" != "${HOME:-}" && -f "$DATA_DIR/woodshop.db" && -d "$DATA_DIR/photos" ]] || { echo "Restore target must be an existing Woodshop data directory, not a broad directory or symlink" >&2; exit 2; }
temp="$(mktemp -d "${TMPDIR:-/tmp}/woodshop-restore.XXXXXX")"
stage="$temp/stage"
restore_parent="$(dirname -- "$DATA_DIR")"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
preserved="$restore_parent/$(basename -- "$DATA_DIR")-pre-restore-$stamp-$$"
old_moved=0
new_installed=0
writer_stopped=0

rollback() {
  status=$?
  set +e
  if (( status != 0 && old_moved == 1 )); then
    if (( new_installed == 1 )); then mv -- "$DATA_DIR" "$restore_parent/$(basename -- "$DATA_DIR")-failed-restore-$stamp-$$"; fi
    mv -- "$preserved" "$DATA_DIR"
  fi
  if (( status != 0 && writer_stopped == 1 )); then
    "${compose[@]}" up -d --no-deps --force-recreate woodshop >/dev/null 2>&1 || echo "Automatic restart failed; inspect preserved data at $preserved" >&2
  fi
  rm -rf -- "$temp"
  exit "$status"
}
trap rollback EXIT

python3 "$ROOT_DIR/scripts/archive.py" extract "$archive" "$stage"
[[ -f "$stage/woodshop.db" && -f "$stage/SHA256SUMS" && -d "$stage/photos" ]] || { echo "Backup data incomplete" >&2; exit 1; }
sqlite3 "$stage/woodshop.db" 'PRAGMA integrity_check;' | grep -qx ok || { echo "SQLite integrity check failed" >&2; exit 1; }

uid="$(stat -f '%u' "$DATA_DIR" 2>/dev/null || stat -c '%u' "$DATA_DIR")"
gid="$(stat -f '%g' "$DATA_DIR" 2>/dev/null || stat -c '%g' "$DATA_DIR")"
"${compose[@]}" stop woodshop
writer_stopped=1
mv -- "$DATA_DIR" "$preserved"
old_moved=1
mkdir -m 700 "$DATA_DIR"
new_installed=1
cp -a -- "$stage/." "$DATA_DIR/"
rm -f -- "$DATA_DIR/woodshop.db-wal" "$DATA_DIR/woodshop.db-shm"
if chown -R "$uid:$gid" "$DATA_DIR" 2>/dev/null; then :; else
  # macOS Docker Desktop maps the invoking user through its VM; Linux production
  # must preserve the prior bind owner, while non-root macOS keeps local ownership.
  if [[ "$(uname -s)" != Darwin ]]; then echo "Unable to restore data ownership" >&2; exit 1; fi
fi
"${compose[@]}" up -d --no-deps --force-recreate woodshop
for attempt in {1..60}; do
  if "${compose[@]}" exec -T woodshop curl --fail --silent http://127.0.0.1:8080/health >/dev/null; then break; fi
  if [[ "$attempt" == 60 ]]; then echo "Restored app failed health check" >&2; exit 1; fi
  sleep 2
done
old_moved=0
echo "Restore healthy. Prior data preserved at $preserved."
