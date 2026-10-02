#!/usr/bin/env python3
"""Verify credentials and model availability without printing secrets or making inference."""
import argparse
import sys
from integration_env import read_env, request_json, telegram


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--discover-user-ids", action="store_true", help="Read sender IDs from pending private /start messages")
    args = parser.parse_args()
    values = read_env()
    if values.get("TELEGRAM_BOT_TOKEN"):
        bot = telegram(values, "getMe")
        print(f"Telegram: verified @{bot.get('username', 'bot')}")
        webhook = telegram(values, "getWebhookInfo")
        print("Telegram webhook: " + ("registered" if webhook.get("url") else "not registered (local polling is available)"))
        print("Allowed users: " + ("configured" if values.get("TELEGRAM_ALLOWED_USER_IDS") else "missing; access is denied until configured"))
        if args.discover_user_ids:
            if webhook.get("url"):
                raise RuntimeError("User discovery via polling requires a bot without a registered webhook")
            updates = telegram(values, "getUpdates", {"timeout": 0, "allowed_updates": ["message"]})
            ids = sorted({message["from"]["id"] for update in updates if (message := update.get("message", {})).get("chat", {}).get("type") == "private" and message.get("text", "").split()[0:1] == ["/start"] and "from" in message})
            print("Private /start sender IDs: " + (", ".join(map(str, ids)) if ids else "none yet; send /start to the bot first"))
    else:
        print("Telegram: token missing")
    if values.get("OPENROUTER_API_KEY"):
        request_json("https://openrouter.ai/api/v1/key", bearer=values["OPENROUTER_API_KEY"])
        print("OpenRouter: key verified")
        models = request_json("https://openrouter.ai/api/v1/models").get("data", [])
        available = {model["id"]: model for model in models}
        for name in ("OPENROUTER_MODEL", "OPENROUTER_FALLBACK_MODEL"):
            identifier = values.get(name, "")
            model = available.get(identifier)
            print(f"{name}: {identifier} — " + ("available" if model else "not listed; select an available model before testing"))
            if model and name == "OPENROUTER_MODEL":
                modalities = model.get("architecture", {}).get("input_modalities", [])
                print("Reference image input: " + ("supported" if "image" in modalities else "not advertised by this model"))
    else:
        print("OpenRouter: key missing")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError) as error:
        print(f"Verification failed: {error}", file=sys.stderr)
        sys.exit(1)
