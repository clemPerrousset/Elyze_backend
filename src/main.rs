use std::sync::Arc;

use tokio::sync::Mutex;

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

    let votes = db::load_votes(&pool)
        .await
        .expect("Failed to load votes");

    let counts = db::load_counts(&pool)
        .await
        .expect("Failed to load vote counts");

    tracing::info!(
        "Loaded {} votes, {} candidates from DB",
        votes.len(),
        counts.len()
    );

    let state = Arc::new(AppState {
        db: pool.clone(),
        votes: Arc::new(votes),
        counts: Arc::new(counts),
        pending: Arc::new(Mutex::new(Vec::new())),
        hmac_secret: std::env::var("VOTE_HMAC_SECRET")
            .expect("VOTE_HMAC_SECRET must be set"),
        admin_token: std::env::var("ADMIN_TOKEN")
            .expect("ADMIN_TOKEN must be set"),
    });

    // Tâche background : flush toutes les 5 secondes
    let flush_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let ops = {
                let mut pending = flush_state.pending.lock().await;
                if pending.is_empty() {
                    continue;
                }
                std::mem::take(&mut *pending)
            };
            let count = ops.len();
            if let Err(e) = db::flush(&flush_state.db, ops).await {
                tracing::error!("Flush error: {}", e);
            } else {
                tracing::debug!("Flushed {} ops to SQLite", count);
            }
        }
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
