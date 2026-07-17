//! s21notify v3 — точка входа. Пока каркас: конфиг из .env + логирование.

fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    tracing::info!(
        "s21notify v{} — каркас, сервисной логики ещё нет",
        env!("CARGO_PKG_VERSION")
    );
}
