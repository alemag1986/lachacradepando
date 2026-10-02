"""Private local configuration helpers; never print credential values."""
from pathlib import Path
import json
import os
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def read_env():
    values = {}
    path = ROOT / ".env"
    if path.exists():
        for line in path.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            key, separator, value = line.partition("=")
            if not separator:
                continue
            value = value.strip()
            if value.startswith('"') and value.endswith('"'):
                value = json.loads(value)
            elif value.startswith("'") and value.endswith("'"):
                value = value[1:-1]
            values[key.strip()] = value
    return values


def save_env(values):
    path = ROOT / ".env"
    if path.is_symlink():
        raise RuntimeError("Refusing a symlink at .env")
    temporary = ROOT / ".env.setup.tmp"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w") as handle:
            for key, value in values.items():
                handle.write(f"{key}={json.dumps(str(value))}\n")
        temporary.replace(path)
        path.chmod(0o600)
    finally:
        temporary.unlink(missing_ok=True)


def request_json(url, payload=None, bearer=None, timeout=30):
    headers = {"Accept": "application/json"}
    if bearer:
        headers["Authorization"] = f"Bearer {bearer}"
    body = None
    if payload is not None:
        headers["Content-Type"] = "application/json"
        body = json.dumps(payload).encode()
    request = urllib.request.Request(url, data=body, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            content = response.read(8 * 1024 * 1024 + 1)
            if len(content) > 8 * 1024 * 1024:
                raise RuntimeError("API response exceeded the limit")
            return json.loads(content)
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"API returned HTTP {error.code}") from None
    except (urllib.error.URLError, TimeoutError, ValueError):
        raise RuntimeError("API connection or response failed") from None


def telegram(values, method, payload=None, timeout=30):
    token = values.get("TELEGRAM_BOT_TOKEN", "")
    if not token:
        raise RuntimeError("TELEGRAM_BOT_TOKEN is not configured")
    result = request_json(f"https://api.telegram.org/bot{token}/{method}", payload, timeout=timeout)
    if not result.get("ok"):
        raise RuntimeError("Telegram rejected the request")
    return result["result"]
