use std::sync::Arc;

use axum::{
    extract::{Json, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::{auth, state::{AppState, PendingOp}};

#[derive(Deserialize)]
pub struct VoteRequest {
    pub phone_id: String,
    pub candidate_id: String,
    pub token: String,
}

#[derive(Serialize)]
struct VotesResponse {
    candidates: Vec<CandidateCount>,
}

#[derive(Serialize)]
struct CandidateCount {
    id: String,
    count: i64,
}

pub async fn post_vote(
    State(state): State<Arc<AppState>>,
    Json(req): Json<VoteRequest>,
) -> impl IntoResponse {
    if req.phone_id.is_empty() || req.candidate_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "missing fields"})),
        )
            .into_response();
    }

    if !auth::verify_phone_token(&req.phone_id, &req.token, &state.hmac_secret) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid token"})),
        )
            .into_response();
    }

    // Toute la logique est en RAM — zéro accès DB dans le hot path
    let existing = state.votes.get(&req.phone_id).map(|v| v.clone());

    let (status, op) = match existing {
        None => {
            // Nouveau vote
            state.counts.entry(req.candidate_id.clone()).and_modify(|v| *v += 1).or_insert(1);
            state.votes.insert(req.phone_id.clone(), req.candidate_id.clone());
            ("voted", PendingOp::Upsert(req.phone_id, req.candidate_id))
        }
        Some(ref old) if old == &req.candidate_id => {
            // Toggle off — même candidat
            state.counts.entry(req.candidate_id.clone()).and_modify(|v| *v -= 1);
            state.votes.remove(&req.phone_id);
            ("unvoted", PendingOp::Delete(req.phone_id))
        }
        Some(old_candidate) => {
            // Changement de candidat
            state.counts.entry(old_candidate).and_modify(|v| *v -= 1);
            state.counts.entry(req.candidate_id.clone()).and_modify(|v| *v += 1).or_insert(1);
            state.votes.insert(req.phone_id.clone(), req.candidate_id.clone());
            ("changed", PendingOp::Upsert(req.phone_id, req.candidate_id))
        }
    };

    state.pending.lock().await.push(op);

    (StatusCode::OK, Json(serde_json::json!({"status": status}))).into_response()
}

/// Servi entièrement depuis la RAM — zéro DB.
pub async fn get_votes(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let candidates: Vec<CandidateCount> = state
        .counts
        .iter()
        .map(|e| CandidateCount {
            id: e.key().clone(),
            count: *e.value(),
        })
        .collect();

    Json(VotesResponse { candidates })
}

/// Retourne le candidat pour lequel ce téléphone a voté, ou null si aucun vote.
pub async fn get_my_vote(
    State(state): State<Arc<AppState>>,
    Path(phone_id): Path<String>,
) -> impl IntoResponse {
    match state.votes.get(&phone_id) {
        Some(candidate_id) => {
            Json(serde_json::json!({ "candidate_id": candidate_id.clone() })).into_response()
        }
        None => Json(serde_json::json!({ "candidate_id": null })).into_response(),
    }
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// Liste d'ids de candidats séparés par des virgules, ex: "melenchon_jeanluc,lepen_marine".
    /// Si absent/vide, l'historique de tous les candidats est renvoyé.
    candidate_ids: Option<String>,
}

#[derive(Serialize)]
struct HistoryPoint {
    date: i64,
    candidate_id: String,
    votes: i64,
}

#[derive(Serialize)]
struct HistoryResponse {
    history: Vec<HistoryPoint>,
}

/// Historique des voix dans le temps, pour les candidats dont les ids sont envoyés par le front.
pub async fn get_history(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HistoryQuery>,
) -> impl IntoResponse {
    let ids: Vec<String> = params
        .candidate_ids
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    match crate::db::get_history(&state.db, &ids).await {
        Ok(rows) => {
            let history = rows
                .into_iter()
                .map(|(candidate_id, votes, date)| HistoryPoint {
                    date,
                    candidate_id,
                    votes,
                })
                .collect();
            (StatusCode::OK, Json(HistoryResponse { history })).into_response()
        }
        Err(e) => {
            tracing::error!("history query error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
                .into_response()
        }
    }
}
