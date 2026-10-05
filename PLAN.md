# Woodshop execution plan

- [x] Read specification and inspect empty repository.
- [x] Establish dependencies, module boundaries, and delegated ownership.
- [x] Database/domain implementation: migrations, transaction-safe repositories, focused tests.
- [x] Telegram/AI: authenticated webhook, pedido/entrega, validated extraction and photos.
- [x] Web: dashboard, order/customer/delivery pages and management; five focused tests pass.
- [x] Core integration compiles; database/domain and web acceptance tests pass.
- [x] Infrastructure: Docker/Caddy, backup/restore, CI and setup documentation.
- [x] Security review and acceptance tests.
- [x] Local Compose, authentication, persistence and online backup/restore verification.
- [x] DigitalOcean deployment, HTTPS and production Telegram webhook registration.
- [ ] New user-originated production pedido/entrega acceptance smoke test.

## Deployment status

- DigitalOcean Droplet created with explicit $12/month approval: 606182744,
  lachacradepando, nyc3, Ubuntu 24.04, s-1vcpu-2gb; public IPv4 104.131.98.158.
  Cloud firewall attached: HTTP/HTTPS public, SSH restricted to provisioning IP.
  Dedicated deployment key/state are private under ignored .deploy and excluded
  from Docker builds. Paid backups are disabled. User will configure DNS A record
  lachacradepando.magnati.dev -> 104.131.98.158; DNS has been verified. Production
  login hash is valid bcrypt format. Production image built and both containers are healthy.
- Initial release snapshot preserves two existing orders and photos. Server-side database
  integrity passes, data is UID/GID 10001 mode 700, and environment is root-owned mode 600.
- Cloud admin token stays local. Telegram and OpenRouter credentials have been verified;
  Telegram whitelist contains only user 8708668772. Production webhook is registered at
  https://lachacradepando.magnati.dev/telegram/webhook with no pending updates or reported
  delivery errors. Local polling is not running.
- First verified production backup succeeded and nightly backup timer is enabled.
  Paid DigitalOcean backups and off-host storage have not been enabled.

## Environment observations

- Rust toolchain is installed.
- Docker Desktop started successfully; Docker engine 29.7.2 is available.
- Dependency download requires network permission in this sandbox.
- Initialization commit created. Rust dependencies downloaded and locked.

## Integration decisions

- Confirmed: /orders exposes all order details via an expandable, lazily loaded
  full order card. Native disclosure and a normal full-page link preserve fallback
  navigation; filters remain in place. Multiple expanded orders use unique form IDs.
- Confirmed UI requirement: all site interface text is Spanish; brand and page titles
  use La Chacra de Pando. Internal status values and existing customer/order data remain
  unchanged. Spanish labels also cover history, photos, missing fields and errors.
- Development server binds loopback; production app is reachable only on Compose network.
- Telegram's photo sizes represent one image; albums require scoped aggregation.
- Order/delivery mutation and webhook result IDs commit atomically for crash-safe retries.
- Dashboard mutation forms enforce same-origin requests and record order events.
- Root took ownership of template integration after Askama syntax failures; web agent
  retains handlers and focused tests.
- Customer name fallback does not override conflicting phone/email; ambiguous same-name
  matches require contact details rather than silently merging different customers.
- Local Telegram polling bridge forwards to the authenticated webhook handler so live
  integration testing does not require a public domain; production remains webhook based.

## Verification evidence

- Database/domain: 13 order tests and 5 delivery tests pass.
- Web: 5 tests pass; persistence/photo/history acceptance test passes.
- Telegram: 15 mock-only tests pass, including restart/late-album recovery, retries,
  duplicate photos, authorization and user/chat/session-expiry scoping.
- Full Rust suite: 39 tests pass. Formatting, strict Clippy and release build pass.
- Read-only final security/Telegram review found no remaining defects in reviewed scope.
- Python: 6 tests pass (backup archive validation, retention, private release snapshot,
  cloud-secret exclusion and successful empty API responses).
- Docker image builds successfully. Isolated Compose has exactly two healthy containers.
- Caddy local health 200, unauthenticated dashboard 401, authenticated dashboard 200,
  webhook without secret 401.
- Actual online backup succeeded; restore recovered one saved order from a two-order
  test database, integrity check returned ok, and the saved order survived restart.
- Final image and revised backup/restore scripts verified with fresh isolated data;
  a pre-release migration checksum mismatch in earlier test data was detected and
  that earlier database was preserved rather than modified or discarded.
- Native UI running http://localhost:8097; orders page inspected in Brave.
- Production HTTPS health returns 200; dashboard without login returns 401; HTTP redirects
  to HTTPS with 308; webhook without its secret returns 401. Telegram webhook registration
  is verified. A new user-originated production pedido/entrega smoke test remains available.
- The first production backup initially failed because Docker cp could not access a tmpfs
  snapshot; backup.sh now snapshots on the data bind mount and copies the host path.
  Its systemd rerun succeeded; retention and private snapshot tests pass.
- First production backup restored into a separate disposable directory: two orders,
  integrity_check ok and no foreign-key violations. Live data was not replaced.
- All four production pages render Spanish language metadata and La Chacra de Pando.
- Telegram token and OpenRouter key are configured and verified; secrets remain private.

## Shared integration contract

`AppState { pool: SqlitePool, config: Arc<Config>, http: reqwest::Client }`.
Config: app_env, app_port, database_url, telegram_bot_token, telegram_webhook_secret,
telegram_allowed_user_ids: Vec<i64>, telegram_allowed_chat_ids: Vec<i64>, openrouter_api_key,
openrouter_model, openrouter_fallback_model, public_base_url, data_dir (all String except port u16).
`web::router() -> Router<AppState>` and `telegram::router() -> Router<AppState>`.
`db::connect(url: &str) -> anyhow::Result<SqlitePool>` migrates at startup.
`AppError` supports From<anyhow::Error> and From<sqlx::Error> with safe HTTP responses.
Database/AI extraction contract types live in domain and are re-exported there.
Agents may use runtime sqlx::query/query_as (no compile-time database requirement).
