use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

use crate::auth::token_store::TokenStore;
use crate::config::AuthMode;

/// Axum middleware that validates OAuth 2 bearer tokens.
///
/// In `GitHub` mode, checks the token against the in-memory TokenStore
/// (tokens were validated against GitHub API during the OAuth callback).
///
/// In `JWT` mode, validates the JWT signature and claims.
pub async fn require_auth(
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    let token_store = request
        .extensions()
        .get::<TokenStore>()
        .cloned()
        .ok_or_else(|| unauthorized("Server misconfigured: missing token store"))?;

    let auth_mode = request
        .extensions()
        .get::<AuthMode>()
        .cloned()
        .ok_or_else(|| unauthorized("Server misconfigured: missing auth mode"))?;

    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| unauthorized("Authorization header must use Bearer scheme"))?;

    match auth_mode {
        AuthMode::GitHub => {
            token_store
                .get(token)
                .ok_or_else(|| unauthorized("Invalid or expired token"))?;
        }
        AuthMode::Jwt => {
            validate_jwt(token, &request)?;
        }
    }

    Ok(next.run(request).await)
}

fn validate_jwt(token: &str, request: &Request) -> Result<(), Response> {
    let secret = request
        .extensions()
        .get::<JwtSecret>()
        .ok_or_else(|| unauthorized("Server misconfigured: missing JWT secret"))?;

    let validation = jsonwebtoken::Validation::default();
    let key = jsonwebtoken::DecodingKey::from_secret(secret.0.as_bytes());

    jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation)
        .map_err(|e| unauthorized(&format!("Invalid JWT: {e}")))?;

    Ok(())
}

fn unauthorized(message: &str) -> Response {
    let body = serde_json::json!({ "error": message });
    (StatusCode::UNAUTHORIZED, Json(body)).into_response()
}

/// Wrapper to store JWT secret in request extensions.
#[derive(Clone)]
pub struct JwtSecret(pub String);
