#![allow(dead_code)]

mod auth;
mod config;
mod github;
mod graph;
mod server;
mod tools;

use anyhow::Result;
use axum::{middleware, routing::get, Router};
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
use github::client::GitHubClient;
use graph::client::GraphClient;
use server::CodeMemoryServer;

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

    // OAuth state for GitHub auth endpoints
    let oauth_state = OAuthState {
        config: config.clone(),
        token_store: token_store.clone(),
        http_client: reqwest::Client::new(),
    };

    // Auth state for MCP middleware
    let auth_state = Arc::new(AuthState {
        token_store: token_store.clone(),
        auth_mode: config.auth_mode.clone(),
        jwt_secret: config.jwt_secret.clone(),
    });

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
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            require_auth,
        ));

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
