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
    routing::get,
    Extension, Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use auth::github_oauth::OAuthState;
use auth::middleware::{require_auth, JwtSecret};
use auth::token_store::TokenStore;
use config::{AppConfig, AuthMode};
use github::client::GitHubClient;
use graph::client::GraphClient;
use server::CodeMemoryServer;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging — logs to stderr (stdout reserved for MCP protocol if using stdio)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("enterprise_code_memory=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    let config = AppConfig::from_env()?;
    let config = Arc::new(config);
    info!("Starting enterprise-code-memory server");

    // Connect to FalkorDB
    let graph = Arc::new(GraphClient::connect(&config).await?);
    graph::schema::ensure_schema(&graph).await?;

    // Token store for OAuth (5 minute TTL for validated tokens)
    let token_store = TokenStore::new(300);

    // OAuth state for GitHub auth endpoints
    let oauth_state = OAuthState {
        config: config.clone(),
        token_store: token_store.clone(),
        http_client: reqwest::Client::new(),
    };

    // Auth routes (unauthenticated)
    let auth_routes = Router::new()
        .route("/auth/github/login", get(auth::github_oauth::login))
        .route(
            "/auth/github/callback",
            get(auth::github_oauth::callback),
        )
        .with_state(oauth_state);

    // Health check (unauthenticated)
    let health_route = Router::new().route("/health", get(|| async { "ok" }));

    // MCP service factory — creates a CodeMemoryServer per session.
    let graph_for_mcp = graph.clone();
    let config_for_mcp = config.clone();

    let mcp_service = StreamableHttpService::new(
        move || {
            let graph = graph_for_mcp.clone();
            let config = config_for_mcp.clone();

            // Create a GitHub client with a placeholder token.
            // The sync tools will eventually use the session's auth token.
            let github = Arc::new(
                GitHubClient::new(&config, "placeholder")
                    .expect("Failed to create GitHub client"),
            );

            Ok(CodeMemoryServer::new(graph, github, config))
        },
        LocalSessionManager::default().into(),
        Default::default(),
    );

    // Protected MCP route with auth middleware
    let mcp_route = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(Extension(token_store.clone()))
        .layer(Extension(config.auth_mode.clone()))
        .layer(middleware::from_fn(require_auth));

    // Add JWT secret extension if in JWT mode
    let mcp_route = if config.auth_mode == AuthMode::Jwt {
        if let Some(ref secret) = config.jwt_secret {
            mcp_route.layer(Extension(JwtSecret(secret.clone())))
        } else {
            mcp_route
        }
    } else {
        mcp_route
    };

    // Combine all routes
    let app = Router::new()
        .merge(health_route)
        .merge(auth_routes)
        .merge(mcp_route);

    let addr = config.server_addr();
    info!("Listening on {addr}");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
