#!/usr/bin/env python3
"""Local-only Telegram polling bridge to the app's authenticated webhook handler."""
from concurrent.futures import ThreadPoolExecutor
import argparse
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from integration_env import read_env, telegram


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app-url", default="http://localhost:8097")
    args = parser.parse_args()
    url = urllib.parse.urlparse(args.app_url)
    if url.scheme != "http" or url.hostname not in ("localhost", "127.0.0.1", "::1") or url.username or url.password or url.path not in ("", "/") or url.query or url.fragment:
        raise RuntimeError("App URL must be an HTTP loopback origin")
    values = read_env()
    if not values.get("TELEGRAM_ALLOWED_USER_IDS") or not values.get("TELEGRAM_WEBHOOK_SECRET"):
        raise RuntimeError("Configure allowed user IDs and webhook secret first")
    if telegram(values, "getWebhookInfo").get("url"):
        raise RuntimeError("This bot already has a webhook; polling will not change it")
    endpoint = args.app_url.rstrip("/") + "/telegram/webhook"

    def forward(update):
        request = urllib.request.Request(endpoint, data=json.dumps(update).encode(), headers={"Content-Type": "application/json", "X-Telegram-Bot-Api-Secret-Token": values["TELEGRAM_WEBHOOK_SECRET"]})
        try:
            with urllib.request.urlopen(request, timeout=180) as response:
                if response.status != 200:
                    raise RuntimeError("Local app rejected an update")
        except (urllib.error.URLError, TimeoutError):
            raise RuntimeError("Local webhook failed; updates will be retried") from None

    print("Local Telegram bridge running. Send pedido or entrega to your bot. Ctrl+C stops it.")
    offset = None
    with ThreadPoolExecutor(max_workers=8) as executor:
        while True:
            payload = {"timeout": 25, "allowed_updates": ["message"]}
            if offset is not None:
                payload["offset"] = offset
            try:
                updates = telegram(values, "getUpdates", payload, timeout=35)
                results = [executor.submit(forward, update) for update in updates]
                for result in results:
                    result.result()
                if updates:
                    offset = max(update["update_id"] for update in updates) + 1
            except RuntimeError as error:
                print(f"{error}; retrying in 3 seconds.", file=sys.stderr)
                time.sleep(3)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Local Telegram bridge stopped.")
    except (RuntimeError, OSError) as error:
        print(f"Bridge failed: {error}", file=sys.stderr)
        sys.exit(1)
