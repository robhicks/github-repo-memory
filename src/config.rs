use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct AppConfig {
    // GitHub OAuth
    pub github_client_id: String,
    pub github_client_secret: String,
    pub github_redirect_uri: String,
    pub github_org: String,
    pub github_api_url: String,
    pub github_auth_url: String,
    pub github_token_url: String,

    // FalkorDB
    pub falkordb_host: String,
    pub falkordb_port: u16,
    pub falkordb_password: Option<String>,
    pub falkordb_graph_name: String,

    // Server
    pub server_host: String,
    pub server_port: u16,

    // Auth mode for MCP clients: "github" or "jwt"
    pub auth_mode: AuthMode,
    pub jwt_secret: Option<String>,

    // Sync settings
    pub sync_batch_size: usize,
    pub max_file_depth: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthMode {
    GitHub,
    Jwt,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let auth_mode = match env_or("ECMEM_AUTH_MODE", "github").as_str() {
            "jwt" => AuthMode::Jwt,
            _ => AuthMode::GitHub,
        };

        let jwt_secret = if auth_mode == AuthMode::Jwt {
            Some(env_required("ECMEM_JWT_SECRET")?)
        } else {
            std::env::var("ECMEM_JWT_SECRET").ok()
        };

        Ok(Self {
            github_client_id: env_required("ECMEM_GITHUB_CLIENT_ID")?,
            github_client_secret: env_required("ECMEM_GITHUB_CLIENT_SECRET")?,
            github_redirect_uri: env_required("ECMEM_GITHUB_REDIRECT_URI")?,
            github_org: env_required("ECMEM_GITHUB_ORG")?,
            github_api_url: env_or("ECMEM_GITHUB_API_URL", "https://api.github.com"),
            github_auth_url: env_or(
                "ECMEM_GITHUB_AUTH_URL",
                "https://github.com/login/oauth/authorize",
            ),
            github_token_url: env_or(
                "ECMEM_GITHUB_TOKEN_URL",
                "https://github.com/login/oauth/access_token",
            ),

            falkordb_host: env_or("ECMEM_FALKORDB_HOST", "127.0.0.1"),
            falkordb_port: env_or("ECMEM_FALKORDB_PORT", "6379")
                .parse()
                .context("ECMEM_FALKORDB_PORT must be a valid port number")?,
            falkordb_password: std::env::var("ECMEM_FALKORDB_PASSWORD").ok(),
            falkordb_graph_name: env_or("ECMEM_FALKORDB_GRAPH", "enterprise_code"),

            server_host: env_or("ECMEM_SERVER_HOST", "127.0.0.1"),
            server_port: env_or("ECMEM_SERVER_PORT", "8080")
                .parse()
                .context("ECMEM_SERVER_PORT must be a valid port number")?,

            auth_mode,
            jwt_secret,

            sync_batch_size: env_or("ECMEM_SYNC_BATCH_SIZE", "50")
                .parse()
                .context("ECMEM_SYNC_BATCH_SIZE must be a number")?,
            max_file_depth: env_or("ECMEM_MAX_FILE_DEPTH", "2")
                .parse()
                .context("ECMEM_MAX_FILE_DEPTH must be a number")?,
        })
    }

    pub fn server_addr(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
    }
}

fn env_required(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("{key} environment variable is required"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
