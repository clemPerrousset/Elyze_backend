use std::sync::Arc;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::state::AppState;

mod candidates;
mod vote;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/vote", post(vote::post_vote))
        .route("/votes", get(vote::get_votes))
        .route("/candidates", post(candidates::add_candidate))
        .route("/candidates/:id", delete(candidates::delete_candidate))
        .with_state(state)
}
