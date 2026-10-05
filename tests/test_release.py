"""Release privacy/snapshot checks use fake settings and disposable databases only."""
from contextlib import closing, redirect_stdout
import io
from pathlib import Path
import sqlite3
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import prepare_release
from integration_env import request_json

class ReleaseTests(unittest.TestCase):
    def test_bundle_excludes_cloud_token_and_keeps_consistent_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ["Cargo.toml", "Cargo.lock", "Dockerfile", ".dockerignore", "Caddyfile", "docker-compose.yml", "README.md"]:
                (root / name).write_text("fixture")
            for name in ["src", "migrations", "templates", "static", "scripts", "ops", "data/photos"]:
                (root / name).mkdir(parents=True, exist_ok=True)
            (root / "data/photos/example.jpg").write_bytes(b"photo fixture")
            with closing(sqlite3.connect(root / "data/woodshop.db")) as db:
                db.execute("PRAGMA journal_mode=WAL")
                db.execute("CREATE TABLE orders(id INTEGER PRIMARY KEY)")
                db.execute("INSERT INTO orders VALUES(1)")
                db.commit()
                values = {key: "fixture" for key in ["TELEGRAM_BOT_TOKEN", "TELEGRAM_WEBHOOK_SECRET", "TELEGRAM_ALLOWED_USER_IDS", "OPENROUTER_API_KEY", "CADDY_ADMIN_USER"]}
                values.update(DIGITALOCEAN_API_TOKEN="CLOUD_SECRET_MUST_NOT_SHIP", CADDY_ADMIN_PASSWORD_HASH="$2a$12$" + "A" * 53)
                with patch.object(prepare_release, "ROOT", root), patch.object(prepare_release, "read_env", return_value=values), redirect_stdout(io.StringIO()):
                    prepare_release.main()
            bundle = next((root / ".deploy").glob("release-*.tar.gz"))
            self.assertEqual(bundle.stat().st_mode & 0o777, 0o600)
            with tarfile.open(bundle) as archive:
                env = archive.extractfile(".env").read().decode()
                self.assertNotIn("DIGITALOCEAN_API_TOKEN", env)
                self.assertNotIn("CLOUD_SECRET_MUST_NOT_SHIP", env)
                self.assertIn("https://lachacradepando.magnati.dev", env)
                self.assertNotIn("data/woodshop.db-wal", archive.getnames())
                self.assertNotIn("data/woodshop.db-shm", archive.getnames())
                snapshot = root / "snapshot.db"
                snapshot.write_bytes(archive.extractfile("data/woodshop.db").read())
                with closing(sqlite3.connect(snapshot)) as db:
                    self.assertEqual(db.execute("SELECT count(*) FROM orders").fetchone()[0], 1)
                    self.assertEqual(db.execute("PRAGMA integrity_check").fetchone()[0], "ok")

    def test_successful_empty_api_response_is_not_reported_as_failure(self):
        response = MagicMock()
        response.status = 204
        response.read.return_value = b""
        response.__enter__.return_value = response
        with patch("urllib.request.urlopen", return_value=response):
            self.assertEqual(request_json("https://api.digitalocean.com/v2/example", payload={}), {})
