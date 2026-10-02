# Woodshop execution plan

- [x] Read specification and inspect empty repository.
- [x] Establish dependencies, module boundaries, and delegated ownership.
- [ ] Database/domain: migrations, transaction-safe repositories, focused tests.
- [ ] Telegram/AI: authenticated webhook, pedido/entrega, validated extraction and photos.
- [ ] Web: dashboard, order/customer/delivery pages and management.
- [ ] Integrate application and run Rust checks.
- [ ] Infrastructure: Docker/Caddy, backup/restore, CI and setup documentation.
- [ ] Security review and acceptance tests.
- [ ] Local Compose, persistence and backup/restore verification.
- [ ] DigitalOcean deployment, HTTPS and live Telegram acceptance.

## Pending deployment inputs

Domain/DNS access; DigitalOcean account or Droplet IP and SSH access; Telegram bot token
and allowed user IDs; OpenRouter API key; dashboard username/password hash.
Secrets belong in ignored .env, never chat or committed files. User confirmed none of
these accounts/settings is ready and explicitly deferred setup to later. Do not create
accounts, provision paid infrastructure, or attempt live API acceptance this run.

## Environment observations

- Rust toolchain is installed.
- Docker CLI exists, but Docker daemon is not running; container verification needs it.
- Dependency download requires network permission in this sandbox.

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
