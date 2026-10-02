use std::env;

#[derive(Clone)]
pub struct Config {
    pub app_env: String,
    pub app_port: u16,
    pub database_url: String,
    pub telegram_bot_token: String,
    pub telegram_webhook_secret: String,
    pub telegram_allowed_user_ids: Vec<i64>,
    pub telegram_allowed_chat_ids: Vec<i64>,
    pub openrouter_api_key: String,
    pub openrouter_model: String,
    pub openrouter_fallback_model: String,
    pub public_base_url: String,
    pub data_dir: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let get = |key: &str, default: &str| env::var(key).unwrap_or_else(|_| default.into());
        let ids = |key: &str| -> anyhow::Result<Vec<i64>> {
            get(key, "")
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    s.trim()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("Invalid ID in {key}"))
                })
                .collect()
        };
        let config = Self {
            app_env: get("APP_ENV", "development"),
            app_port: get("APP_PORT", "8080").parse()?,
            database_url: get("DATABASE_URL", "sqlite:data/woodshop.db"),
            telegram_bot_token: get("TELEGRAM_BOT_TOKEN", ""),
            telegram_webhook_secret: get("TELEGRAM_WEBHOOK_SECRET", ""),
            telegram_allowed_user_ids: ids("TELEGRAM_ALLOWED_USER_IDS")?,
            telegram_allowed_chat_ids: ids("TELEGRAM_ALLOWED_CHAT_IDS")?,
            openrouter_api_key: get("OPENROUTER_API_KEY", ""),
            openrouter_model: get("OPENROUTER_MODEL", "google/gemma-4-26b-a4b-it"),
            openrouter_fallback_model: get("OPENROUTER_FALLBACK_MODEL", "qwen/qwen3.5-35b-a3b"),
            public_base_url: get("PUBLIC_BASE_URL", "http://localhost:8080"),
            data_dir: get("DATA_DIR", "data"),
        };
        let public_url = reqwest::Url::parse(&config.public_base_url)
            .map_err(|_| anyhow::anyhow!("PUBLIC_BASE_URL must be a valid HTTP origin"))?;
        anyhow::ensure!(
            matches!(public_url.scheme(), "http" | "https")
                && public_url.host_str().is_some()
                && public_url.username().is_empty()
                && public_url.password().is_none()
                && public_url.query().is_none()
                && public_url.fragment().is_none()
                && public_url.path() == "/",
            "PUBLIC_BASE_URL must be an HTTP origin without credentials, path, query, or fragment"
        );
        if config.app_env == "production" {
            anyhow::ensure!(
                !config.telegram_bot_token.is_empty(),
                "TELEGRAM_BOT_TOKEN required"
            );
            anyhow::ensure!(
                config.telegram_webhook_secret.len() >= 32
                    && config
                        .telegram_webhook_secret
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
                "Valid webhook secret of at least 32 characters required"
            );
            anyhow::ensure!(
                !config.telegram_allowed_user_ids.is_empty(),
                "Allowed Telegram users required"
            );
            anyhow::ensure!(
                !config.openrouter_api_key.is_empty(),
                "OPENROUTER_API_KEY required"
            );
            anyhow::ensure!(public_url.scheme() == "https", "HTTPS public URL required");
        }
        Ok(config)
    }
}
