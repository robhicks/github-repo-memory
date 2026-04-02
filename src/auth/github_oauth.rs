use anyhow::Result;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::token_store::TokenStore;
use crate::config::AppConfig;

/// Shared state for OAuth endpoints.
#[derive(Clone)]
pub struct OAuthState {
    pub config: Arc<AppConfig>,
    pub token_store: TokenStore,
    pub http_client: reqwest::Client,
}

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: String,
    #[allow(dead_code)]
    pub state: Option<String>,
}

#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
    token_type: String,
    #[allow(dead_code)]
    scope: String,
}

#[derive(Deserialize)]
struct GitHubUser {
    login: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub login: String,
}

/// GET /auth/github/login — redirects to GitHub OAuth authorization page.
pub async fn login(State(state): State<OAuthState>) -> Response {
    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&scope=repo,read:org",
        state.config.github_auth_url, state.config.github_client_id, state.config.github_redirect_uri
    );
    Redirect::temporary(&auth_url).into_response()
}

/// GET /auth/github/callback — exchanges authorization code for access token.
pub async fn callback(
    State(state): State<OAuthState>,
    Query(params): Query<CallbackParams>,
) -> Result<Json<TokenResponse>, AppError> {
    // Exchange code for token
    let token_response: GitHubTokenResponse = state
        .http_client
        .post(&state.config.github_token_url)
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "client_id": state.config.github_client_id,
            "client_secret": state.config.github_client_secret,
            "code": params.code,
            "redirect_uri": state.config.github_redirect_uri,
        }))
        .send()
        .await
        .map_err(|e| AppError(anyhow::anyhow!("Failed to exchange code: {e}")))?
        .json()
        .await
        .map_err(|e| AppError(anyhow::anyhow!("Failed to parse token response: {e}")))?;

    // Validate token by fetching user info
    let api_url = format!("{}/user", state.config.github_api_url);
    let user: GitHubUser = state
        .http_client
        .get(&api_url)
        .header("Authorization", format!("Bearer {}", token_response.access_token))
        .header("User-Agent", "enterprise-code-memory")
        .send()
        .await
        .map_err(|e| AppError(anyhow::anyhow!("Failed to fetch user info: {e}")))?
        .json()
        .await
        .map_err(|e| AppError(anyhow::anyhow!("Failed to parse user info: {e}")))?;

    // Store token — the GitHub access token itself serves as the bearer token
    state.token_store.insert(
        &token_response.access_token,
        user.login.clone(),
        token_response.access_token.clone(),
    );

    Ok(Json(TokenResponse {
        access_token: token_response.access_token,
        token_type: token_response.token_type,
        login: user.login,
    }))
}

/// Simple error wrapper for axum responses.
pub struct AppError(pub anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.0.to_string() });
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(body),
        )
            .into_response()
    }
}
