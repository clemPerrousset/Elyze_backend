use std::sync::Arc;

use dashmap::DashMap;
use sqlx::SqlitePool;

pub struct AppState {
    pub db: SqlitePool,
    /// In-memory vote counts per candidate — served directly on GET /votes.
    /// Source of truth is the DB; this cache is rebuilt on startup.
    pub counts: Arc<DashMap<String, i64>>,
    pub hmac_secret: String,
    pub admin_token: String,
}
