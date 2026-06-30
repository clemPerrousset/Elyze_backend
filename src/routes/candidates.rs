use std::sync::Arc;

use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct AddCandidateRequest {
    pub id: String,
}

fn check_admin(headers: &HeaderMap, admin_token: &str) -> bool {
    headers
        .get("X-Admin-Token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == admin_token)
        .unwrap_or(false)
}

pub async fn add_candidate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<AddCandidateRequest>,
) -> impl IntoResponse {
    if !check_admin(&headers, &state.admin_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    if req.id.is_empty() || req.id.len() > 64 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "id must be 1-64 chars"})),
        )
            .into_response();
    }

    match sqlx::query("INSERT OR IGNORE INTO candidates (id) VALUES (?)")
        .bind(&req.id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            state.counts.insert(req.id, 0);
            (
                StatusCode::CREATED,
                Json(serde_json::json!({"status": "created"})),
            )
                .into_response()
        }
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "already_exists"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("DB error adding candidate: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "db error"})),
            )
                .into_response()
        }
    }
}

pub async fn delete_candidate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if !check_admin(&headers, &state.admin_token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    match sqlx::query("DELETE FROM candidates WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
    {
        Ok(result) if result.rows_affected() > 0 => {
            state.counts.remove(&id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "deleted"})),
            )
                .into_response()
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "candidate not found"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("DB error deleting candidate: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "db error"})),
            )
                .into_response()
        }
    }
}
