# La Chacra de Pando

Rust, Axum, Askama, HTMX and SQLite application for a small furniture shop. The production stack contains exactly two services: `woodshop` and Caddy. SQLite and photos persist in `./data:/data`; Caddy's certificates and configuration persist in named volumes.

## Local development

Rust development can use the repository's normal Cargo workflow. The isolated local Compose file needs no accounts, `.env`, or production settings. It uses its own `${LOCAL_DATA_DIR:-./.local-data}` directory so it cannot share the native app's `./data` accidentally.

Before starting, create the local bind directory and grant the container's non-root UID/GID 10001 write access:

```sh
LOCAL_DATA_DIR="${LOCAL_DATA_DIR:-$PWD/.local-data}"
mkdir -p "$LOCAL_DATA_DIR"
chmod 700 "$LOCAL_DATA_DIR"
sudo chown 10001:10001 "$LOCAL_DATA_DIR"
export LOCAL_DATA_DIR
docker compose -f docker-compose.local.yml up --build
```

Open `http://localhost:8088`. The app has no host port mapping. Local Caddy is HTTP-only and bound to loopback; its demo Basic Auth account is `demo` / `woodshop-local-demo`. Its health route is `/health`; the exact Telegram webhook path bypasses proxy auth because the application requires Telegram's secret header. For backup/restore on this stack, set `COMPOSE_FILE=docker-compose.local.yml` and `WOODSHOP_DATA_DIR` to the same absolute directory as `LOCAL_DATA_DIR`. Do not send real customer data to this development instance.

## Production deployment

Target is an Ubuntu DigitalOcean Droplet with Docker Engine and the Compose plugin, a domain A record pointing to the Droplet, and inbound TCP 80/443 enabled. Provisioning an account, Droplet, DNS or paid service is a separate user-controlled step.

1. Clone the repository on the host and enter its directory. Create `.env` from `.env.example`; fill the Telegram bot token, a strong webhook secret (at least 32 URL-safe characters), allowed Telegram user IDs, OpenRouter key, and real HTTPS `PUBLIC_BASE_URL`.
2. Generate Caddy credentials without putting the plaintext password in `.env` or a process argument: run `docker run --rm -it caddy:2.10.2-alpine caddy hash-password`. Enter the password at Caddy's interactive prompt, then set `CADDY_ADMIN_USER` and `CADDY_ADMIN_PASSWORD_HASH`.
3. Restrict the environment file and create the persistent directory: `chmod 600 .env && install -d -m 700 data`. The image runs as UID/GID 10001; ensure this user can write `data` (`sudo chown 10001:10001 data`). The image contains no application secrets.
4. Run `./scripts/deploy.sh`. It runs fmt, clippy, tests and release build before Compose build/start, then waits for database-backed health.
5. Register the Telegram webhook with URL `https://your-domain/telegram/webhook` and `secret_token` equal to `TELEGRAM_WEBHOOK_SECRET`. Visit `https://your-domain/health` for the minimal health status, then verify protected dashboard access and the bot. Caddy obtains and renews HTTPS certificates automatically. Do not deploy until DNS resolves to the host and ports 80/443 are reachable.

Production Compose publishes only Caddy on 80 and 443. The Rust listener is internal to Compose on port 8080. Caddy leaves only `/health` and the exact `/telegram/webhook` path without Basic Auth; webhook authenticity is enforced by Axum. Every dashboard route uses Caddy Basic Auth. Set `APP_ENV=production`; startup requires Telegram and OpenRouter configuration and an HTTPS base URL.

### Provisioned deployment

The approved target is `https://lachacradepando.magnati.dev`, on DigitalOcean Droplet
`606182744` (`104.131.98.158`, `nyc3`, Ubuntu 24.04, 2 GB Basic, $12/month).
The app resides at `/opt/lachacradepando`. Dedicated SSH key, known-hosts and provisioning
state live only in ignored `.deploy/`; never commit or upload them to the Docker build.
SSH is restricted by the cloud firewall to the provisioning connection's public IP.
If that IP changes, update the firewall through the DigitalOcean console before SSH.

`python3 scripts/prepare_release.py` creates a private **first-deployment-only** bundle,
with a verified SQLite snapshot, photos and an explicit production environment allowlist.
It does not copy the DigitalOcean admin token. Do not extract such a bundle over an
existing production data directory: later releases must preserve the live database,
photos and `.env`, replacing application code only after a verified backup.
Stop local Telegram polling before running `python3 scripts/activate_telegram_webhook.py`;
activation requires working production HTTPS and preserves queued updates.

## Backup and restore

Run `./scripts/backup.sh` on the Docker host. It uses SQLite's online `.backup` API, snapshots photos, verifies SQLite integrity and SHA-256 hashes for every database/photo file, then writes a mode-600 archive under `backups/`. Retention keeps seven distinct calendar days and four distinct Sunday weeks. The portable archive helper uses Python 3's standard library, available by default on Ubuntu. Optional S3-compatible upload is available by setting `S3_DESTINATION` and installing/configuring the AWS CLI in the host environment. For accountless local Compose backup/restore, set `COMPOSE_FILE=docker-compose.local.yml` and `WOODSHOP_DATA_DIR` to the absolute directory used by `LOCAL_DATA_DIR`; this keeps operations isolated from native `./data`.

The `ops/woodshop-backup.service` and `.timer` units schedule backups at 03:00 UTC daily
with up to five minutes of randomized delay. Inspect `systemctl list-timers woodshop-backup.timer`
and `journalctl -u woodshop-backup.service`. These are host-local backups, not paid
DigitalOcean snapshots or off-host disaster recovery.

Restore with `./scripts/restore.sh /absolute/path/to/backups/woodshop-YYYYMMDDTHHMMSSZ.tar.gz`. Python validates archive member types and paths before extraction; symlinks, hardlinks, devices, traversal and unexpected files are rejected. The target must be a data directory containing `woodshop.db` and `photos`, and cannot be a symlink, filesystem root, workspace root or home directory. After checksum and SQLite integrity checks, restore stops the writer and moves the complete data directory to a uniquely named sibling backup, preventing old SQLite WAL/SHM files from being reused. It preserves host UID/GID, recreates the app container against the replacement directory, verifies `/health`, and rolls back the data directory and app if startup fails. Keep off-host copies for disaster recovery; host-local retention alone does not protect against Droplet loss.

## System setup and future credentials

- Telegram: create a bot with [BotFather](https://core.telegram.org/bots/features#botfather), then use `python3 scripts/configure_integrations.py --telegram` to enter credentials without echoing the token. Allowed user IDs are numeric sender IDs, not Telegram `api_id` or `api_hash`. Run `python3 scripts/check_integrations.py`; it checks `getMe`, pending webhook status and allowed users without exposing secrets. Start the local app on port 8097, restart it after configuration changes, then run `python3 scripts/telegram_local.py` in another terminal and send the bot `pedido` or `entrega`. The bridge uses long polling and forwards real updates to the app's secret-protected webhook handler at loopback. Keep it running during local tests. Production continues to use Telegram's public HTTPS [webhook API](https://core.telegram.org/bots/api#setwebhook); local polling does not register or change the production webhook.
- OpenRouter: create an API key in the account console and run `python3 scripts/configure_integrations.py --openrouter`; the key is entered privately and saved in `.env` with mode 600. Run `python3 scripts/check_integrations.py` to verify key validity and model listing without inference. Model IDs from the product spec may require selection based on current [OpenRouter model availability](https://openrouter.ai/docs/api_reference/overview). Actual inference occurs only when exercising `pedido`; checks and CI do not make inference calls.
- Caddy: the configuration follows the [Caddyfile Basic Auth directive](https://caddyserver.com/docs/caddyfile/directives/basic_auth); generate a password hash using the same Caddy image version.
- DigitalOcean: create an Ubuntu Droplet, configure DNS and firewall, install Docker Engine plus Compose plugin, then follow the production steps above. Supply the real domain, host access, and credentials later; this repository setup does not create accounts or provision infrastructure.

## CI

GitHub Actions runs `cargo fmt --check`, `cargo clippy --locked -- -D warnings`, `cargo test --locked`, `cargo build --locked --release`, and a Docker image build. Test AI inputs use fixtures; CI does not call OpenRouter.

## Public repository safety

Keep credentials in the ignored `.env`, never in source, examples, screenshots,
issues or transcripts. `.env.example` must contain empty credential fields.
Deployment keys and release bundles in `.deploy/`, databases, customer photos and
backups are private and must not be attached to public releases. Environment files,
private key files and exported release/backup bundles are also excluded from Docker
build contexts. The documented local demo password is intentionally public and must
never be reused for production.

Before publishing, scan staged files and all Git history for secrets; ignore rules
do not remove files already committed. If a credential is exposed, revoke/rotate it
before cleaning the history. Do not publish raw setup conversations: these can
contain credentials even when the repository does not.
