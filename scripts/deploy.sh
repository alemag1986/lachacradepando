#!/usr/bin/env bash
set -Eeuo pipefail
ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$ROOT_DIR"
[[ -f .env ]] || { echo "Create production .env first" >&2; exit 1; }
echo "Running required release checks before deployment..."
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests -p 'test_*.py'
cargo build --locked --release
install -d -m 700 data
docker compose config --quiet
docker compose build --pull
docker compose up -d --remove-orphans
for attempt in {1..60}; do
  if docker compose exec -T woodshop curl --fail --silent http://127.0.0.1:8080/health >/dev/null; then break; fi
  if [[ "$attempt" == 60 ]]; then echo "Application health check timed out" >&2; docker compose ps; exit 1; fi
  sleep 2
done

echo "Deployment healthy. Verify public HTTPS /health and docker compose ps."
