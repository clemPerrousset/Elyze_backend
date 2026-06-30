use std::sync::Arc;

use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::{auth, state::AppState};

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

    if !state.counts.contains_key(&req.candidate_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "unknown candidate"})),
        )
            .into_response();
    }

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT candidate_id FROM votes WHERE phone_id = ?")
            .bind(&req.phone_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);

    match existing {
        None => {
            if let Err(e) =
                sqlx::query("INSERT INTO votes (phone_id, candidate_id) VALUES (?, ?)")
                    .bind(&req.phone_id)
                    .bind(&req.candidate_id)
                    .execute(&state.db)
                    .await
            {
                tracing::error!("DB insert error: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "db error"})),
                )
                    .into_response();
            }
            state
                .counts
                .entry(req.candidate_id)
                .and_modify(|v| *v += 1);
            (StatusCode::OK, Json(serde_json::json!({"status": "voted"}))).into_response()
        }

        Some((ref existing_id,)) if existing_id == &req.candidate_id => {
            if let Err(e) = sqlx::query("DELETE FROM votes WHERE phone_id = ?")
                .bind(&req.phone_id)
                .execute(&state.db)
                .await
            {
                tracing::error!("DB delete error: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "db error"})),
                )
                    .into_response();
            }
            state
                .counts
                .entry(req.candidate_id)
                .and_modify(|v| *v -= 1);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "unvoted"})),
            )
                .into_response()
        }

        Some((old_candidate,)) => {
            if let Err(e) = sqlx::query(
                "UPDATE votes SET candidate_id = ?, voted_at = unixepoch() WHERE phone_id = ?",
            )
            .bind(&req.candidate_id)
            .bind(&req.phone_id)
            .execute(&state.db)
            .await
            {
                tracing::error!("DB update error: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "db error"})),
                )
                    .into_response();
            }
            state
                .counts
                .entry(old_candidate)
                .and_modify(|v| *v -= 1);
            state
                .counts
                .entry(req.candidate_id)
                .and_modify(|v| *v += 1);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "changed"})),
            )
                .into_response()
        }
    }
}

/// Served entirely from RAM — zero DB hit.
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
