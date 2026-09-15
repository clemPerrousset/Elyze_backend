use std::sync::Arc;

use axum::{
    middleware::from_fn_with_state,
    routing::{delete, get, post},
    Router,
};

use crate::{rate_limit, state::AppState};

mod candidates;
mod vote;

pub fn create_router(state: Arc<AppState>) -> Router {
    let vote_routes = Router::new()
        .route("/vote", post(vote::post_vote))
        .route_layer(from_fn_with_state(state.clone(), rate_limit::limit_vote));

    let admin_routes = Router::new()
        .route("/candidates", post(candidates::add_candidate))
        .route("/candidates/:id", delete(candidates::delete_candidate))
        .route_layer(from_fn_with_state(state.clone(), rate_limit::limit_admin));

    let public_routes = Router::new()
        .route("/votes", get(vote::get_votes))
        .route("/votes/history", get(vote::get_history))
        .route("/votes/:phone_id", get(vote::get_my_vote));

    vote_routes
        .merge(admin_routes)
        .merge(public_routes)
        .with_state(state)
}
