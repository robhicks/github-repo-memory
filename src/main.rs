#![allow(dead_code)]

mod auth;
mod config;
mod github;
mod graph;
mod server;
mod tools;

use anyhow::Result;
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use auth::github_oauth::OAuthState;
use auth::middleware::{require_auth, AuthState};
use auth::token_store::TokenStore;
use graph::client::GraphClient;
use server::CodeMemoryServer;

/// Combined application state shared across all routes.
#[derive(Clone)]
pub struct AppState {
    pub oauth: OAuthState,
    pub auth: Arc<AuthState>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("enterprise_code_memory=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = config::AppConfig::from_env()?;
    let config = Arc::new(config);
    info!("Starting enterprise-code-memory server");

    // Connect to FalkorDB
    let graph = Arc::new(GraphClient::connect(&config).await?);
    graph::schema::ensure_schema(&graph).await?;

    // Token store for OAuth (5 minute TTL for validated tokens)
    let token_store = TokenStore::new(300);

    let oauth_state = OAuthState {
        config: config.clone(),
        token_store: token_store.clone(),
        http_client: reqwest::Client::new(),
    };

    let auth_state = Arc::new(AuthState {
        token_store: token_store.clone(),
        auth_mode: config.auth_mode.clone(),
        jwt_secret: config.jwt_secret.clone(),
    });

    let app_state = AppState {
        oauth: oauth_state,
        auth: auth_state.clone(),
    };

    // --- MCP service ---
    let graph_for_mcp = graph.clone();
    let config_for_mcp = config.clone();
    let token_store_for_mcp = token_store.clone();

    let mcp_service = StreamableHttpService::new(
        move || {
            let graph = graph_for_mcp.clone();
            let config = config_for_mcp.clone();
            let token_store = token_store_for_mcp.clone();
            Ok(CodeMemoryServer::new(graph, config, token_store))
        },
        LocalSessionManager::default().into(),
        Default::default(),
    );

    // --- Build routers separately, then combine ---

    // 1. OAuth routes (unauthenticated, need AppState)
    let oauth_router = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(auth::github_oauth::resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(auth::github_oauth::auth_server_metadata),
        )
        .route("/authorize", get(auth::github_oauth::authorize))
        .route("/auth/github/callback", get(auth::github_oauth::callback))
        .route("/token", post(auth::github_oauth::token))
        .route("/register", post(auth::github_oauth::register))
        .with_state(app_state);

    // 2. MCP route (authenticated, no AppState needed)
    let mcp_router = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            auth_state.clone(),
            require_auth,
        ));

    // 3. Combine everything
    let app = oauth_router
        .route("/health", get(|| async { "ok" }))
        .merge(mcp_router);

    let addr = config.server_addr();
    info!("Listening on {addr}");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
