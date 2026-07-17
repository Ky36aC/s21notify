//! s21notify — точка входа.

use s21_server::{alarm, config::AppConfig, db, http, poll, state::AppState, watcher};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let cfg = AppConfig::from_env()?;
    let pool = db::connect(&cfg.database_url).await?;
    let (poll_tx, poll_rx) = poll::channel();
    let bind_addr = cfg.bind_addr.clone();
    let state = AppState::build(cfg, pool, poll_tx)?;

    // вебхуки: TG с секрет-заголовком, MAX с секретом в URL;
    // MAX снимает подписку после ~8 ч недоступности — переустановка при каждом старте
    for (name, adapter) in &state.adapters {
        let url = state.cfg.webhook_url(name);
        match adapter.set_webhook(&url).await {
            Ok(()) => tracing::info!("вебхук {name} установлен"),
            Err(e) => tracing::error!("вебхук {name}: {e}"),
        }
    }

    let manager = watcher::PollManager::new(state.clone());
    tokio::spawn(manager.run(poll_rx));
    tokio::spawn(alarm::run(state.clone()));
    tokio::spawn(watcher::housekeeping(state.clone()));

    tracing::info!(
        "s21notify v{} слушает {} ({} мессенджеров)",
        env!("CARGO_PKG_VERSION"),
        bind_addr,
        state.adapters.len()
    );
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, http::router(state)).await?;
    Ok(())
}
