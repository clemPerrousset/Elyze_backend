use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

pub enum PendingOp {
    Upsert(String, String), // phone_id, candidate_id
    Delete(String),         // phone_id
}

/// Clé = (nom du bucket protégé, IP appelante). Valeur = (nb requêtes dans la
/// fenêtre courante, début de la fenêtre). Voir src/rate_limit.rs.
pub type RateLimitBuckets = DashMap<(&'static str, IpAddr), (u32, Instant)>;

/// État de ban progressif pour une IP sur un bucket donné : nb d'échecs
/// d'authentification (401) consécutifs, niveau de ban déjà atteint (fixe la
/// durée du prochain ban), et instant jusqu'auquel l'IP est bannie. Voir
/// src/rate_limit.rs.
pub struct BanState {
    pub consecutive_failures: u32,
    pub ban_level: u32,
    pub banned_until: Option<Instant>,
}

pub type BanBuckets = DashMap<(&'static str, IpAddr), BanState>;

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
    /// État du rate limiting par IP (POST /vote, routes admin).
    pub rate_limits: Arc<RateLimitBuckets>,
    /// État du ban progressif par IP (POST /vote, routes admin).
    pub bans: Arc<BanBuckets>,
}
