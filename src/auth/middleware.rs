use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use crate::auth::token_store::TokenStore;
use crate::config::AuthMode;

/// Shared auth state injected into the middleware via axum State.
#[derive(Clone)]
pub struct AuthState {
    pub token_store: TokenStore,
    pub auth_mode: AuthMode,
    pub jwt_secret: Option<String>,
}

/// Axum middleware that validates OAuth 2 bearer tokens.
///
/// In `GitHub` mode, checks the token against the in-memory TokenStore
/// (tokens were validated against GitHub API during the OAuth callback).
///
/// In `JWT` mode, validates the JWT signature and claims.
pub async fn require_auth(
    State(auth): State<Arc<AuthState>>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| unauthorized("Authorization header must use Bearer scheme"))?;

    match auth.auth_mode {
        AuthMode::GitHub => {
            auth.token_store
                .get(token)
                .ok_or_else(|| unauthorized("Invalid or expired token"))?;
        }
        AuthMode::Jwt => {
            validate_jwt(token, &auth)?;
        }
    }

    Ok(next.run(request).await)
}

fn validate_jwt(token: &str, auth: &AuthState) -> Result<(), Response> {
    let secret = auth
        .jwt_secret
        .as_ref()
        .ok_or_else(|| unauthorized("Server misconfigured: missing JWT secret"))?;

    let validation = jsonwebtoken::Validation::default();
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());

    jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
        .map_err(|e| unauthorized(&format!("Invalid JWT: {e}")))?;

    Ok(())
}

fn unauthorized(message: &str) -> Response {
    let body = serde_json::json!({ "error": message });
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}
