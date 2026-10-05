#!/usr/bin/env python3
"""Build a private first-deployment bundle without cloud-admin credentials."""
from datetime import datetime, timezone
from contextlib import closing
import os
from pathlib import Path
import re
import shutil
import sqlite3
import tarfile
import tempfile
from integration_env import ROOT, read_env

def main():
    values = read_env()
    keys = ["TELEGRAM_BOT_TOKEN", "TELEGRAM_WEBHOOK_SECRET", "TELEGRAM_ALLOWED_USER_IDS",
        "TELEGRAM_ALLOWED_CHAT_IDS", "OPENROUTER_API_KEY", "OPENROUTER_MODEL",
        "OPENROUTER_FALLBACK_MODEL", "CADDY_ADMIN_USER", "CADDY_ADMIN_PASSWORD_HASH"]
    required = ["TELEGRAM_BOT_TOKEN", "TELEGRAM_WEBHOOK_SECRET", "TELEGRAM_ALLOWED_USER_IDS",
        "OPENROUTER_API_KEY", "CADDY_ADMIN_USER", "CADDY_ADMIN_PASSWORD_HASH"]
    if any(not values.get(key) for key in required):
        raise RuntimeError("Required production setting is missing")
    if not re.fullmatch(r"\$2[aby]\$\d{2}\$[./A-Za-z0-9]{53}", values["CADDY_ADMIN_PASSWORD_HASH"]):
        raise RuntimeError("Website password hash is not valid bcrypt")
    production = {key: values.get(key, "") for key in keys}
    production.update(APP_ENV="production", APP_PORT="8080", APP_BIND_HOST="0.0.0.0",
        DATABASE_URL="sqlite:/data/woodshop.db", DATA_DIR="/data", RUST_LOG="info",
        PUBLIC_BASE_URL="https://lachacradepando.magnati.dev")
    private = ROOT / ".deploy"
    private.mkdir(mode=0o700, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    bundle = private / ("release-" + stamp + ".tar.gz")
    with tempfile.TemporaryDirectory(prefix="release-", dir=private) as temporary:
        stage = Path(temporary)
        envfile = stage / ".env"
        descriptor = os.open(envfile, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "w") as handle:
            for key, value in production.items():
                if any(character in value for character in ("'", "\n", "\r")):
                    raise RuntimeError("Unsupported character in production configuration")
                handle.write(key + "='" + value + "'\n")
        data = stage / "data"
        data.mkdir(mode=0o700)
        source = ROOT / values.get("DATA_DIR", "data")
        database = source / "woodshop.db"
        count = 0
        if database.exists():
            with closing(sqlite3.connect("file:" + str(database) + "?mode=ro", uri=True)) as live:
                with closing(sqlite3.connect(data / "woodshop.db")) as snapshot:
                    live.backup(snapshot)
                    if snapshot.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
                        raise RuntimeError("Database snapshot integrity failed")
                    count = snapshot.execute("SELECT count(*) FROM orders").fetchone()[0]
            (data / "woodshop.db").chmod(0o600)
        photos = source / "photos"
        if photos.exists():
            if any(path.is_symlink() for path in photos.rglob("*")):
                raise RuntimeError("Photo tree contains a symlink")
            shutil.copytree(photos, data / "photos")
        else:
            (data / "photos").mkdir(mode=0o700)
        for path in data.rglob("*"):
            path.chmod(0o700 if path.is_dir() else 0o600)
        def filter_member(member):
            return None if "__pycache__" in member.name.split("/") else member
        with tarfile.open(bundle, "x:gz") as archive:
            for name in ["Cargo.toml", "Cargo.lock", "Dockerfile", ".dockerignore", "Caddyfile",
                "docker-compose.yml", "src", "migrations", "templates", "static", "scripts", "ops", "README.md"]:
                archive.add(ROOT / name, arcname=name, filter=filter_member)
            archive.add(envfile, arcname=".env")
            archive.add(data, arcname="data")
    bundle.chmod(0o600)
    print("Private deployment bundle:", bundle)
    print("Order snapshot count:", count)
    print("DigitalOcean admin token excluded from production configuration.")

if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, sqlite3.Error) as error:
        raise SystemExit("Release preparation stopped: " + str(error))
