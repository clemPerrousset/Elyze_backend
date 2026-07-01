use std::sync::Arc;

use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

pub enum PendingOp {
    Upsert(String, String), // phone_id, candidate_id
    Delete(String),         // phone_id
}

pub struct AppState {
    pub db: SqlitePool,
    /// Votes en RAM — source de vérité pour les lectures et la logique de vote.
    /// Flushé en SQLite toutes les 5 secondes en arrière-plan.
    pub votes: Arc<DashMap<String, String>>,
    /// Compteurs par candidat — servis directement sur GET /votes, zéro DB.
    pub counts: Arc<DashMap<String, i64>>,
    /// File d'attente des opérations à persister en SQLite.
    pub pending: Arc<Mutex<Vec<PendingOp>>>,
    pub hmac_secret: String,
    pub admin_token: String,
}
