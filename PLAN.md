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
- [ ] DigitalOcean deployment, HTTPS and live Telegram acceptance.

## Pending deployment inputs

Domain/DNS access; DigitalOcean account or Droplet IP and SSH access; Telegram bot token
and allowed user IDs; OpenRouter API key; dashboard username/password hash.
Secrets belong in ignored .env, never chat or committed files. User confirmed none of
these accounts/settings is ready. Initially setup was deferred; user subsequently asked
to create Telegram and OpenRouter accounts and configure real integrations now. Guided
BotFather creation is in progress; keys are entered privately with configure_integrations.py.
DigitalOcean/domain setup remains deferred. No paid infrastructure is provisioned.

## Environment observations

- Rust toolchain is installed.
- Docker Desktop started successfully; Docker engine 29.7.2 is available.
- Dependency download requires network permission in this sandbox.
- Initialization commit created. Rust dependencies downloaded and locked.

## Integration decisions

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
- Backup archives: 4 Python tests pass (round-trip, unsafe entries, corruption, retention).
- Docker image builds successfully. Isolated Compose has exactly two healthy containers.
- Caddy local health 200, unauthenticated dashboard 401, authenticated dashboard 200,
  webhook without secret 401.
- Actual online backup succeeded; restore recovered one saved order from a two-order
  test database, integrity check returned ok, and the saved order survived restart.
- Final image and revised backup/restore scripts verified with fresh isolated data;
  a pre-release migration checksum mismatch in earlier test data was detected and
  that earlier database was preserved rather than modified or discarded.
- Native UI running http://localhost:8097; orders page inspected in Brave.
- Production HTTPS/public Telegram/DigitalOcean acceptance remains pending credentials/DNS.
- Private integration checker confirms Telegram token and OpenRouter key still missing.

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
