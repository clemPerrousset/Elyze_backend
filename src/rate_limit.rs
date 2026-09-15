use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

use crate::state::{AppState, BanBuckets, BanState, RateLimitBuckets};

/// Paliers de ban progressif (secondes), déclenchés après `FAILURE_THRESHOLD`
/// échecs d'authentification (401) consécutifs depuis une même IP sur un
/// bucket donné. Le niveau grimpe à chaque nouveau ban (pas de décroissance) :
/// un récidiviste écope de bans de plus en plus longs.
const BAN_TIERS_SECS: [u64; 3] = [60, 300, 1800]; // 1 min, 5 min, 30 min
const FAILURE_THRESHOLD: u32 = 5;

/// Fenêtre fixe en mémoire, par IP + par bucket (route protégée). Suffisant
/// pour freiner un bourrage grossier depuis une IP donnée ; pas un budget de
/// résilience distribué (une seule instance, pas de reverse-proxy connu ici
/// — si un reverse-proxy est ajouté devant, s'assurer que ConnectInfo reflète
/// bien l'IP cliente, pas celle du proxy).
async fn check_rate(
    buckets: &RateLimitBuckets,
    bucket: &'static str,
    ip: IpAddr,
    max_requests: u32,
    window: Duration,
) -> bool {
    let now = Instant::now();
    let mut entry = buckets.entry((bucket, ip)).or_insert((0, now));
    if now.duration_since(entry.1) > window {
        *entry = (0, now);
    }
    entry.0 += 1;
    entry.0 <= max_requests
}

/// `Some(durée_restante)` si l'IP est actuellement bannie sur ce bucket.
fn currently_banned(bans: &BanBuckets, bucket: &'static str, ip: IpAddr) -> Option<Duration> {
    let now = Instant::now();
    bans.get(&(bucket, ip))
        .and_then(|state| state.banned_until)
        .and_then(|until| (until > now).then(|| until - now))
}

/// À appeler après la réponse du handler : met à jour le compteur d'échecs
/// et déclenche/prolonge un ban si le seuil est atteint. Un statut différent
/// de 401 réinitialise le compteur d'échecs (mais pas le niveau de ban déjà
/// atteint — un récidiviste reste traité plus sévèrement).
fn record_outcome(bans: &BanBuckets, bucket: &'static str, ip: IpAddr, status: StatusCode) {
    let now = Instant::now();
    let mut entry = bans.entry((bucket, ip)).or_insert(BanState {
        consecutive_failures: 0,
        ban_level: 0,
        banned_until: None,
    });

    if status == StatusCode::UNAUTHORIZED {
        entry.consecutive_failures += 1;
        if entry.consecutive_failures >= FAILURE_THRESHOLD {
            let tier = (entry.ban_level as usize).min(BAN_TIERS_SECS.len() - 1);
            let ban_duration = Duration::from_secs(BAN_TIERS_SECS[tier]);
            entry.banned_until = Some(now + ban_duration);
            entry.ban_level += 1;
            entry.consecutive_failures = 0;
            tracing::warn!(
                "IP {} bannie {}s sur \"{}\" après {} échecs d'authentification (niveau {})",
                ip,
                ban_duration.as_secs(),
                bucket,
                FAILURE_THRESHOLD,
                entry.ban_level
            );
        }
    } else {
        entry.consecutive_failures = 0;
    }
}

fn too_many_requests() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({"error": "too many requests, réessaie plus tard"})),
    )
        .into_response()
}

fn banned(retry_after: Duration) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({
            "error": "too many failed attempts, réessaie plus tard",
            "retry_after_secs": retry_after.as_secs(),
        })),
    )
        .into_response()
}

async fn guarded(
    state: Arc<AppState>,
    ip: IpAddr,
    bucket: &'static str,
    max_requests: u32,
    window: Duration,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Some(remaining) = currently_banned(&state.bans, bucket, ip) {
        return banned(remaining);
    }

    if !check_rate(&state.rate_limits, bucket, ip, max_requests, window).await {
        return too_many_requests();
    }

    let response = next.run(request).await;
    record_outcome(&state.bans, bucket, ip, response.status());
    response
}

/// 10 requêtes / 10s / IP sur POST /vote — large marge pour un humain qui
/// vote/annule plusieurs fois. Au-delà de 5 échecs d'authentification (401)
/// consécutifs, l'IP est bannie progressivement (1 min, 5 min, puis 30 min).
pub async fn limit_vote(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    guarded(
        state,
        addr.ip(),
        "vote",
        10,
        Duration::from_secs(10),
        request,
        next,
    )
    .await
}

/// 20 requêtes / 60s / IP sur les routes admin (ajout/suppression de
/// candidats). Même ban progressif après 5 échecs (401) consécutifs.
pub async fn limit_admin(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    guarded(
        state,
        addr.ip(),
        "admin",
        20,
        Duration::from_secs(60),
        request,
        next,
    )
    .await
}
