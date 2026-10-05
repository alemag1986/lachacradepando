#!/usr/bin/env python3
"""Switch this bot to the verified production HTTPS endpoint without dropping updates."""
import urllib.request
from integration_env import read_env, telegram

URL = "https://lachacradepando.magnati.dev"

def main():
    values = read_env()
    if not values.get("TELEGRAM_WEBHOOK_SECRET") or not values.get("TELEGRAM_ALLOWED_USER_IDS"):
        raise RuntimeError("Webhook secret and user whitelist must be configured first")
    with urllib.request.urlopen(URL + "/health", timeout=30) as response:
        if response.status != 200:
            raise RuntimeError("Production health check failed")
    existing = telegram(values, "getWebhookInfo")
    endpoint = URL + "/telegram/webhook"
    if existing.get("url") and existing["url"] != endpoint:
        raise RuntimeError("Bot has a different webhook; refusing to replace it without approval")
    if not telegram(values, "setWebhook", {"url": endpoint,
        "secret_token": values["TELEGRAM_WEBHOOK_SECRET"], "allowed_updates": ["message"],
        "drop_pending_updates": False, "max_connections": 10}):
        raise RuntimeError("Telegram did not confirm registration")
    info = telegram(values, "getWebhookInfo")
    if info.get("url") != endpoint:
        raise RuntimeError("Registered webhook does not match production")
    print("Production Telegram webhook verified:", endpoint)
    print("Pending updates:", info.get("pending_update_count", 0))
    print("Recent delivery error reported:", bool(info.get("last_error_message")))

if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError) as error:
        raise SystemExit("Webhook activation stopped: " + str(error))
