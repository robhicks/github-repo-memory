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

// Note: GhCli mode bypasses this middleware entirely (no auth layer is added
// to the MCP router in main.rs), so there is no GhCli match arm needed here.

/// Shared auth state injected into the middleware via axum State.
#[derive(Clone)]
pub struct AuthState {
    pub token_store: TokenStore,
    pub auth_mode: AuthMode,
    pub jwt_secret: Option<String>,
}

/// Axum middleware that validates OAuth 2 bearer tokens.
///
/// Returns MCP-spec compliant 401 responses with WWW-Authenticate headers
/// that include the resource_metadata URL so MCP clients can discover
/// the OAuth endpoints via RFC 9728.
pub async fn require_auth(
    State(auth): State<Arc<AuthState>>,
    request: Request,
    next: Next,
) -> Result<Response, Response> {
    // Derive the resource metadata URL from the request
    let resource_metadata_url = derive_resource_metadata_url(&request);

    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let auth_header = match auth_header {
        Some(h) => h,
        None => return Err(unauthorized_with_discovery(&resource_metadata_url, None)),
    };

    let token = match auth_header.strip_prefix("Bearer ") {
        Some(t) => t,
        None => {
            return Err(unauthorized_with_discovery(
                &resource_metadata_url,
                Some("invalid_request"),
            ))
        }
    };

    match auth.auth_mode {
        AuthMode::GitHub => {
            if auth.token_store.get(token).is_none() {
                return Err(unauthorized_with_discovery(
                    &resource_metadata_url,
                    Some("invalid_token"),
                ));
            }
        }
        AuthMode::Jwt => {
            validate_jwt(token, &auth).map_err(|_| {
                unauthorized_with_discovery(&resource_metadata_url, Some("invalid_token"))
            })?;
        }
        AuthMode::GhCli => {
            // GhCli mode should not use this middleware (no auth layer in main.rs).
            // If reached anyway, allow the request through.
        }
    }

    Ok(next.run(request).await)
}

fn validate_jwt(token: &str, auth: &AuthState) -> Result<(), ()> {
    let secret = auth.jwt_secret.as_ref().ok_or(())?;

    let validation = jsonwebtoken::Validation::default();
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());

    jsonwebtoken::decode::<serde_json::Value>(token, &key, &validation).map_err(|_| ())?;

    Ok(())
}

/// Build a 401 response with WWW-Authenticate header per MCP spec.
///
/// The header includes `resource_metadata` pointing to the
/// `/.well-known/oauth-protected-resource` endpoint so MCP clients
/// can discover OAuth endpoints via RFC 9728.
fn unauthorized_with_discovery(resource_metadata_url: &str, error: Option<&str>) -> Response {
    let mut www_auth = format!(
        "Bearer resource_metadata=\"{}\"",
        resource_metadata_url
    );

    if let Some(err) = error {
        www_auth.push_str(&format!(", error=\"{}\"", err));
    }

    let body = serde_json::json!({
        "error": error.unwrap_or("authorization_required"),
        "error_description": "Bearer token required. Use the resource_metadata URL to discover OAuth endpoints."
    });

    (
        StatusCode::UNAUTHORIZED,
        [("WWW-Authenticate", www_auth)],
        Json(body),
    )
        .into_response()
}

/// Derive the `/.well-known/oauth-protected-resource` URL from the request.
fn derive_resource_metadata_url(request: &Request) -> String {
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");

    let scheme = if host.starts_with("localhost") || host.starts_with("127.") {
        "http"
    } else {
        "https"
    };

    format!(
        "{scheme}://{host}/.well-known/oauth-protected-resource"
    )
}
