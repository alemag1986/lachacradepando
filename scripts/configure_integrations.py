#!/usr/bin/env python3
"""Run in your terminal so API credentials remain private."""
import argparse
import getpass
import secrets
import sys
from integration_env import read_env, save_env, request_json, telegram


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--telegram", action="store_true")
    parser.add_argument("--openrouter", action="store_true")
    args = parser.parse_args()
    values = read_env()
    defaults = {
        "APP_ENV": "development", "APP_PORT": "8097",
        "DATABASE_URL": "sqlite:data/woodshop.db", "DATA_DIR": "data",
        "PUBLIC_BASE_URL": "http://localhost:8097", "RUST_LOG": "info",
        "TELEGRAM_WEBHOOK_SECRET": secrets.token_urlsafe(32),
        "TELEGRAM_ALLOWED_USER_IDS": "", "TELEGRAM_ALLOWED_CHAT_IDS": "",
        "OPENROUTER_MODEL": "google/gemma-4-26b-a4b-it",
        "OPENROUTER_FALLBACK_MODEL": "qwen/qwen3.5-35b-a3b",
    }
    for key, value in defaults.items():
        if not values.get(key):
            values[key] = value
    all_services = not args.telegram and not args.openrouter
    if args.telegram or all_services:
        token = getpass.getpass("Telegram bot token (hidden; Enter keeps existing): ").strip()
        if token:
            values["TELEGRAM_BOT_TOKEN"] = token
        bot = telegram(values, "getMe")
        print(f"Telegram verified: @{bot.get('username', 'bot')}")
        ids = input("Allowed Telegram user IDs, comma separated (Enter keeps existing): ").strip()
        if ids:
            if any(not value.strip().isdigit() or int(value.strip()) <= 0 for value in ids.split(",")):
                raise RuntimeError("Each allowed user ID must be a positive integer")
            values["TELEGRAM_ALLOWED_USER_IDS"] = ",".join(value.strip() for value in ids.split(","))
        save_env(values)
        print("Telegram settings saved privately in .env.")
    if args.openrouter or all_services:
        key = getpass.getpass("OpenRouter API key (hidden; Enter keeps existing): ").strip()
        if key:
            values["OPENROUTER_API_KEY"] = key
        if not values.get("OPENROUTER_API_KEY"):
            raise RuntimeError("OpenRouter API key is required")
        request_json("https://openrouter.ai/api/v1/key", bearer=values["OPENROUTER_API_KEY"])
        save_env(values)
        print("OpenRouter key verified and saved privately in .env. No paid inference was made.")
    print("Restart the app after updating settings.")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, KeyboardInterrupt, EOFError) as error:
        print("Setup cancelled." if isinstance(error, (KeyboardInterrupt, EOFError)) else f"Setup failed: {error}", file=sys.stderr)
        sys.exit(1)
