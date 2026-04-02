use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::token_store::TokenStore;
use crate::config::AppConfig;
use crate::AppState;

/// Shared state for all OAuth/auth endpoints.
#[derive(Clone)]
pub struct OAuthState {
    pub config: Arc<AppConfig>,
    pub token_store: TokenStore,
    pub http_client: reqwest::Client,
}

/// Helper to extract OAuthState from AppState
fn oauth(state: &AppState) -> &OAuthState {
    &state.oauth
}

// --- RFC 9728: Protected Resource Metadata ---

/// GET /.well-known/oauth-protected-resource
///
/// Tells MCP clients where to find the authorization server.
pub async fn resource_metadata(
    headers: HeaderMap,
    State(app): State<AppState>,
) -> impl IntoResponse {
    let base_url = server_base_url(&headers, &app.oauth.config);

    Json(serde_json::json!({
        "resource": base_url,
        "authorization_servers": [base_url],
        "scopes_supported": ["repo", "read:org"],
        "bearer_methods_supported": ["header"]
    }))
}

// --- RFC 8414: OAuth Authorization Server Metadata ---

/// GET /.well-known/oauth-authorization-server
///
/// Advertises OAuth endpoints so MCP clients can discover them.
pub async fn auth_server_metadata(
    headers: HeaderMap,
    State(app): State<AppState>,
) -> impl IntoResponse {
    let base_url = server_base_url(&headers, &app.oauth.config);

    Json(serde_json::json!({
        "issuer": base_url,
        "authorization_endpoint": format!("{}/authorize", base_url),
        "token_endpoint": format!("{}/token", base_url),
        "registration_endpoint": format!("{}/register", base_url),
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "grant_types_supported": ["authorization_code"],
        "response_types_supported": ["code"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["repo", "read:org"]
    }))
}

// --- Authorization Endpoint ---

#[derive(Deserialize)]
pub struct AuthorizeParams {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
}

/// GET /authorize
///
/// Proxies the authorization request to GitHub's OAuth endpoint.
/// The MCP client opens this URL; we redirect to GitHub with our
/// GitHub OAuth App credentials, preserving the client's state and
/// redirect_uri so we can relay the code back.
pub async fn authorize(
    State(app): State<AppState>,
    Query(params): Query<AuthorizeParams>,
) -> impl IntoResponse {
    // Store the client's redirect_uri and PKCE challenge so we can
    // use them when GitHub calls us back.
    let client_redirect = params
        .redirect_uri
        .unwrap_or_else(|| "http://localhost".to_string());
    let client_state = params.state.unwrap_or_default();

    // Build a compound state that encodes both the client's state
    // and redirect_uri so we can recover them in the callback.
    let compound_state = format!(
        "{}|{}",
        urlencoding::encode(&client_state),
        urlencoding::encode(&client_redirect)
    );

    // Store PKCE challenge if provided (we'll need it at /token time)
    if let (Some(challenge), Some(method)) = (&params.code_challenge, &params.code_challenge_method)
    {
        app.oauth.token_store.store_pkce(
            &client_state,
            challenge.clone(),
            method.clone(),
        );
    }

    let github_auth_url = format!(
        "{}?client_id={}&redirect_uri={}&scope={}&state={}",
        app.oauth.config.github_auth_url,
        app.oauth.config.github_client_id,
        urlencoding::encode(&app.oauth.config.github_redirect_uri),
        params.scope.as_deref().unwrap_or("repo,read:org"),
        urlencoding::encode(&compound_state),
    );

    Redirect::temporary(&github_auth_url)
}

// --- GitHub Callback ---

#[derive(Deserialize)]
pub struct CallbackParams {
    pub code: String,
    pub state: Option<String>,
}

/// GET /callback
///
/// GitHub redirects here after the user authorizes. We relay the
/// authorization code back to the MCP client's redirect_uri.
pub async fn callback(
    Query(params): Query<CallbackParams>,
) -> impl IntoResponse {
    let compound_state = params.state.unwrap_or_default();

    // Parse compound state: "client_state|client_redirect_uri"
    let parts: Vec<&str> = compound_state.splitn(2, '|').collect();
    let (client_state, client_redirect) = if parts.len() == 2 {
        (
            urlencoding::decode(parts[0])
                .unwrap_or_default()
                .to_string(),
            urlencoding::decode(parts[1])
                .unwrap_or_default()
                .to_string(),
        )
    } else {
        (String::new(), "http://localhost".to_string())
    };

    // Redirect back to the MCP client with the authorization code
    let redirect_url = format!(
        "{}?code={}&state={}",
        client_redirect,
        urlencoding::encode(&params.code),
        urlencoding::encode(&client_state),
    );

    Redirect::temporary(&redirect_url)
}

// --- Token Endpoint ---

#[derive(Deserialize)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code_verifier: Option<String>,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub scope: String,
}

/// POST /token
///
/// Exchanges an authorization code for an access token by proxying
/// to GitHub's token endpoint. This is the standard OAuth 2 token
/// exchange that MCP clients call after receiving the auth code.
pub async fn token(
    State(app): State<AppState>,
    axum::Form(params): axum::Form<TokenRequest>,
) -> Result<Json<serde_json::Value>, TokenError> {
    let grant_type = params.grant_type.as_deref().unwrap_or("");
    if grant_type != "authorization_code" {
        return Err(TokenError::unsupported_grant_type());
    }

    let code = params
        .code
        .ok_or_else(|| TokenError::invalid_request("missing code parameter"))?;

    // Exchange the code with GitHub
    #[derive(Deserialize)]
    struct GitHubTokenResponse {
        access_token: Option<String>,
        token_type: Option<String>,
        scope: Option<String>,
        error: Option<String>,
        error_description: Option<String>,
    }

    let github_response: GitHubTokenResponse = app
        .oauth
        .http_client
        .post(&app.oauth.config.github_token_url)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", app.oauth.config.github_client_id.as_str()),
            ("client_secret", app.oauth.config.github_client_secret.as_str()),
            ("code", &code),
        ])
        .send()
        .await
        .map_err(|e| TokenError::server_error(&format!("GitHub request failed: {e}")))?
        .json()
        .await
        .map_err(|e| TokenError::server_error(&format!("Failed to parse GitHub response: {e}")))?;

    if let Some(error) = github_response.error {
        let desc = github_response
            .error_description
            .unwrap_or_else(|| error.clone());
        return Err(TokenError::new("invalid_grant", &desc));
    }

    let access_token = github_response
        .access_token
        .ok_or_else(|| TokenError::server_error("No access_token in GitHub response"))?;

    // Validate the token and get user info
    #[derive(Deserialize)]
    struct GitHubUser {
        login: String,
    }

    let user: GitHubUser = app
        .oauth
        .http_client
        .get(&format!("{}/user", app.oauth.config.orgs.first().map(|o| o.api_url.as_str()).unwrap_or("https://api.github.com")))
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "enterprise-code-memory")
        .send()
        .await
        .map_err(|e| TokenError::server_error(&format!("Failed to validate token: {e}")))?
        .json()
        .await
        .map_err(|e| TokenError::server_error(&format!("Failed to parse user: {e}")))?;

    // Store the validated token
    app.oauth.token_store.insert(
        &access_token,
        user.login,
        access_token.clone(),
    );

    let scope = github_response
        .scope
        .unwrap_or_else(|| "repo,read:org".to_string());

    Ok(Json(serde_json::json!({
        "access_token": access_token,
        "token_type": github_response.token_type.unwrap_or_else(|| "Bearer".to_string()),
        "scope": scope
    })))
}

// --- Dynamic Client Registration (RFC 7591) ---

/// POST /register
///
/// Simplified dynamic client registration. MCP clients call this
/// to register themselves before starting the OAuth flow.
/// Since we proxy to GitHub, we just acknowledge the registration
/// and return our GitHub OAuth App's client_id.
pub async fn register(
    State(app): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let client_name = body
        .get("client_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let redirect_uris = body
        .get("redirect_uris")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    tracing::info!(
        client_name = client_name,
        redirect_uris = ?redirect_uris,
        "Dynamic client registration"
    );

    Json(serde_json::json!({
        "client_id": app.oauth.config.github_client_id,
        "client_name": client_name,
        "redirect_uris": redirect_uris,
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    }))
}

// --- Error types ---

pub struct TokenError {
    status: StatusCode,
    error: String,
    description: String,
}

impl TokenError {
    fn new(error: &str, description: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: error.to_string(),
            description: description.to_string(),
        }
    }

    fn unsupported_grant_type() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: "unsupported_grant_type".to_string(),
            description: "Only authorization_code grant type is supported".to_string(),
        }
    }

    fn invalid_request(desc: &str) -> Self {
        Self::new("invalid_request", desc)
    }

    fn server_error(desc: &str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "server_error".to_string(),
            description: desc.to_string(),
        }
    }
}

impl IntoResponse for TokenError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::json!({
            "error": self.error,
            "error_description": self.description
        });
        (self.status, Json(body)).into_response()
    }
}

// --- Helpers ---

/// Derive the server's base URL from the request Host header.
fn server_base_url(headers: &HeaderMap, config: &AppConfig) -> String {
    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        let scheme = if host.starts_with("localhost") || host.starts_with("127.") {
            "http"
        } else {
            "https"
        };
        format!("{scheme}://{host}")
    } else {
        server_base_url_from_config(config)
    }
}

fn server_base_url_from_config(config: &AppConfig) -> String {
    format!("http://{}:{}", config.server_host, config.server_port)
}
