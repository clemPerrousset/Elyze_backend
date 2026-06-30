use std::sync::Arc;

mod auth;
mod db;
mod routes;
mod state;

use state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://votes.db".to_string());

    let pool = db::create_pool(&database_url)
        .await
        .expect("Failed to setup database");

    db::run_migrations(&pool)
        .await
        .expect("Failed to run migrations");

    let counts = db::load_counts(&pool)
        .await
        .expect("Failed to load vote counts");

    let hmac_secret = std::env::var("VOTE_HMAC_SECRET")
        .expect("VOTE_HMAC_SECRET must be set");
    let admin_token = std::env::var("ADMIN_TOKEN")
        .expect("ADMIN_TOKEN must be set");

    let state = Arc::new(AppState {
        db: pool,
        counts: Arc::new(counts),
        hmac_secret,
        admin_token,
    });

    let app = routes::create_router(state);

    let addr = std::env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string());

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");

    tracing::info!("Listening on {}", addr);
    axum::serve(listener, app).await.unwrap();
}
