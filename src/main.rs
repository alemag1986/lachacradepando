use std::{sync::Arc, time::Duration};
use woodshop::{AppState, config::Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = Config::from_env()?;
    tokio::fs::create_dir_all(format!("{}/photos", config.data_dir)).await?;
    let pool = woodshop::db::connect(&config.database_url).await?;
    let default_host = if config.app_env == "production" {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let host: std::net::IpAddr = std::env::var("APP_BIND_HOST")
        .unwrap_or_else(|_| default_host.into())
        .parse()?;
    let listen = format!("{host}:{}", config.app_port);
    let state = AppState {
        pool,
        config: Arc::new(config),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
    };
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!("Woodshop listening on {listen}");
    axum::serve(listener, woodshop::router(state))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
