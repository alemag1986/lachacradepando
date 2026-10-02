#!/usr/bin/env bash
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
BACKUP_DIR="${BACKUP_DIR:-$ROOT_DIR/backups}"
COMPOSE_FILE="${COMPOSE_FILE:-$ROOT_DIR/docker-compose.yml}"
DATA_DIR="${WOODSHOP_DATA_DIR:-$ROOT_DIR/data}"
[[ "$DATA_DIR" = /* ]] || { echo "WOODSHOP_DATA_DIR must be an absolute path" >&2; exit 2; }
compose=(docker compose --project-directory "$ROOT_DIR" -f "$COMPOSE_FILE")
mkdir -p -- "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
stage="$(mktemp -d "$BACKUP_DIR/.woodshop-backup.XXXXXX")"
archive="$BACKUP_DIR/woodshop-$timestamp.tar.gz"
[[ ! -e "$archive" ]] || { echo "A backup already exists for this second; retry shortly" >&2; exit 1; }
trap 'rm -rf -- "$stage"' EXIT

# SQLite online backup API gives a consistent database image while writes continue.
container_snapshot="/tmp/woodshop-backup-$timestamp-$$.db"
"${compose[@]}" exec -T woodshop sqlite3 /data/woodshop.db ".backup $container_snapshot"
"${compose[@]}" cp "woodshop:$container_snapshot" "$stage/woodshop.db"
"${compose[@]}" exec -T woodshop rm -f "$container_snapshot"
sqlite3 "$stage/woodshop.db" 'PRAGMA integrity_check;' | grep -qx ok || { echo "SQLite integrity check failed" >&2; exit 1; }
if [[ -d "$DATA_DIR/photos" ]]; then cp -a -- "$DATA_DIR/photos" "$stage/photos"; else mkdir "$stage/photos"; fi
python3 "$ROOT_DIR/scripts/archive.py" create "$stage" "$archive"
chmod 600 "$archive"

if [[ -n "${S3_DESTINATION:-}" ]]; then
  command -v aws >/dev/null || { echo "S3_DESTINATION set but aws CLI is unavailable" >&2; exit 1; }
  aws s3 cp --only-show-errors "$archive" "${S3_DESTINATION%/}/$(basename "$archive")"
fi
python3 "$ROOT_DIR/scripts/archive.py" prune "$BACKUP_DIR"
printf 'Created verified backup: %s\n' "$archive"
