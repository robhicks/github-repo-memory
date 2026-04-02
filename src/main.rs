#![allow(dead_code)]

mod auth;
mod config;
mod github;
mod graph;
mod server;
mod tools;

use anyhow::{Context, Result};
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use rmcp::ServiceExt;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use auth::github_oauth::OAuthState;
use auth::middleware::{require_auth, AuthState};
use auth::token_store::TokenStore;
use config::AuthMode;
use graph::client::GraphClient;
use server::CodeMemoryServer;

/// Combined application state shared across all routes.
#[derive(Clone)]
pub struct AppState {
    pub oauth: OAuthState,
    pub auth: Arc<AuthState>,
}

/// Retrieve a GitHub token from the `gh` CLI for a specific hostname.
fn gh_cli_token(hostname: &str) -> Result<String> {
    let mut cmd = std::process::Command::new("gh");
    cmd.args(["auth", "token"]);
    if hostname != "github.com" {
        cmd.args(["--hostname", hostname]);
    }
    let output = cmd
        .output()
        .context("Failed to run `gh auth token`. Is the GitHub CLI installed?")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`gh auth token` failed for {hostname} (exit {}): {stderr}",
            output.status.code().unwrap_or(-1)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Fetch the GitHub login for a token via the GitHub API.
async fn gh_cli_login(token: &str, api_url: &str) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct User {
        login: String,
    }
    let user: User = reqwest::Client::new()
        .get(format!("{api_url}/user"))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "enterprise-code-memory")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(user.login)
}

/// Check if `--stdio` was passed on the command line.
fn use_stdio() -> bool {
    std::env::args().any(|a| a == "--stdio")
}

#[tokio::main]
async fn main() -> Result<()> {
    // In stdio mode, logs must go to stderr (stdout is the MCP transport)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("enterprise_code_memory=info".parse()?),
        )
        .with_writer(std::io::stderr)
        .init();

    let config = config::AppConfig::from_env()?;
    let config = Arc::new(config);

    // Connect to FalkorDB
    let graph = Arc::new(GraphClient::connect(&config).await?);
    graph::schema::ensure_schema(&graph).await?;

    // Token store — long TTL for gh_cli since the token doesn't expire via OAuth flow
    let token_ttl = if config.auth_mode == AuthMode::GhCli {
        86400 // 24 hours
    } else {
        300 // 5 minutes
    };
    let token_store = TokenStore::new(token_ttl);

    // If using gh_cli mode, seed the token store with a token per unique hostname
    if config.auth_mode == AuthMode::GhCli {
        let mut seen_hostnames = std::collections::HashSet::new();
        for org in &config.orgs {
            if !seen_hostnames.insert(org.hostname.clone()) {
                continue; // already authenticated this hostname
            }
            let gh_token = gh_cli_token(&org.hostname)?;
            let login = gh_cli_login(&gh_token, &org.api_url).await?;
            info!(login = %login, hostname = %org.hostname, "Authenticated via GitHub CLI");
            token_store.insert_with_hostname(
                &gh_token,
                login,
                gh_token.clone(),
                org.hostname.clone(),
            );
        }
    }

    if use_stdio() {
        info!("Starting in stdio transport mode");
        run_stdio(graph, config, token_store).await
    } else {
        info!("Starting in HTTP transport mode (auth_mode={:?})", config.auth_mode);
        run_http(graph, config, token_store).await
    }
}

/// Run the MCP server over stdio (for local MCP clients like Claude Desktop).
async fn run_stdio(
    graph: Arc<GraphClient>,
    config: Arc<config::AppConfig>,
    token_store: TokenStore,
) -> Result<()> {
    let server = CodeMemoryServer::new(graph, config, token_store);
    let (stdin, stdout) = rmcp::transport::io::stdio();
    let service = server.serve((stdin, stdout)).await?;
    service.waiting().await?;
    Ok(())
}

/// Run the MCP server over Streamable HTTP with optional OAuth.
async fn run_http(
    graph: Arc<GraphClient>,
    config: Arc<config::AppConfig>,
    token_store: TokenStore,
) -> Result<()> {
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

    let app = if config.auth_mode == AuthMode::GhCli {
        Router::new()
            .nest_service("/mcp", mcp_service)
            .route("/health", get(|| async { "ok" }))
    } else {
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

        let mcp_router = Router::new()
            .nest_service("/mcp", mcp_service)
            .layer(middleware::from_fn_with_state(
                auth_state.clone(),
                require_auth,
            ));

        oauth_router
            .route("/health", get(|| async { "ok" }))
            .merge(mcp_router)
    };

    let addr = config.server_addr();
    info!("Listening on {addr}");
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
