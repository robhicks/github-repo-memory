use anyhow::{Context, Result};

/// Configuration for a single GitHub organization/enterprise instance.
#[derive(Debug, Clone)]
pub struct OrgConfig {
    /// Organization name (e.g., "my-org")
    pub org: String,
    /// GitHub API base URL (e.g., "https://api.github.com" or "https://ghes.corp.com/api/v3")
    pub api_url: String,
    /// Hostname for `gh auth token --hostname` (e.g., "github.com" or "ghes.corp.com")
    pub hostname: String,
    /// Optional list of specific repos to ingest for this org.
    pub repos: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    // GitHub OAuth (used in github/jwt auth modes)
    pub github_client_id: String,
    pub github_client_secret: String,
    pub github_redirect_uri: String,
    pub github_auth_url: String,
    pub github_token_url: String,

    /// All configured GitHub organizations.
    pub orgs: Vec<OrgConfig>,

    // FalkorDB
    pub falkordb_host: String,
    pub falkordb_port: u16,
    pub falkordb_password: Option<String>,
    pub falkordb_graph_name: String,

    // Server
    pub server_host: String,
    pub server_port: u16,

    // Auth mode for MCP clients: "github", "jwt", or "gh_cli"
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
    GhCli,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let auth_mode = match env_or("ECMEM_AUTH_MODE", "github").as_str() {
            "jwt" => AuthMode::Jwt,
            "gh_cli" | "gh-cli" | "ghcli" => AuthMode::GhCli,
            _ => AuthMode::GitHub,
        };

        let jwt_secret = if auth_mode == AuthMode::Jwt {
            Some(env_required("ECMEM_JWT_SECRET")?)
        } else {
            std::env::var("ECMEM_JWT_SECRET").ok()
        };

        // OAuth credentials are only required for GitHub OAuth mode
        let (github_client_id, github_client_secret, github_redirect_uri) =
            if auth_mode == AuthMode::GhCli {
                (
                    std::env::var("ECMEM_GITHUB_CLIENT_ID").unwrap_or_default(),
                    std::env::var("ECMEM_GITHUB_CLIENT_SECRET").unwrap_or_default(),
                    std::env::var("ECMEM_GITHUB_REDIRECT_URI").unwrap_or_default(),
                )
            } else {
                (
                    env_required("ECMEM_GITHUB_CLIENT_ID")?,
                    env_required("ECMEM_GITHUB_CLIENT_SECRET")?,
                    env_required("ECMEM_GITHUB_REDIRECT_URI")?,
                )
            };

        // Parse org configurations.
        // ECMEM_GITHUB_ORGS takes precedence (multi-org).
        // Falls back to ECMEM_GITHUB_ORG + ECMEM_GITHUB_API_URL (single org, backward compat).
        //
        // Format: "org1@hostname1,org2@hostname2,org3"
        //   - org@hostname → GHE instance at that hostname
        //   - org alone    → github.com
        let orgs = if let Ok(orgs_str) = std::env::var("ECMEM_GITHUB_ORGS") {
            let repos_map = parse_per_org_repos()?;
            orgs_str
                .split(',')
                .map(|entry| {
                    let entry = entry.trim();
                    let (org, hostname) = if let Some((o, h)) = entry.split_once('@') {
                        (o.to_string(), h.to_string())
                    } else {
                        (entry.to_string(), "github.com".to_string())
                    };
                    let api_url = api_url_for_hostname(&hostname);
                    let repos = repos_map.get(&org).cloned().unwrap_or_default();
                    OrgConfig { org, api_url, hostname, repos }
                })
                .collect()
        } else {
            let org = env_required("ECMEM_GITHUB_ORG")?;
            let api_url = env_or("ECMEM_GITHUB_API_URL", "https://api.github.com");
            let hostname = hostname_from_api_url(&api_url);
            let repos = std::env::var("ECMEM_REPOS")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|s| s.split(',').map(|r| r.trim().to_string()).collect())
                .unwrap_or_default();
            vec![OrgConfig { org, api_url, hostname, repos }]
        };

        Ok(Self {
            github_client_id,
            github_client_secret,
            github_redirect_uri,
            github_auth_url: env_or(
                "ECMEM_GITHUB_AUTH_URL",
                "https://github.com/login/oauth/authorize",
            ),
            github_token_url: env_or(
                "ECMEM_GITHUB_TOKEN_URL",
                "https://github.com/login/oauth/access_token",
            ),

            orgs,

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

/// Parse ECMEM_REPOS for per-org repo lists.
/// Format: "org1:repo-a,repo-b;org2:repo-c" or just "repo-a,repo-b" (applies to all orgs).
fn parse_per_org_repos() -> Result<std::collections::HashMap<String, Vec<String>>> {
    let mut map = std::collections::HashMap::new();
    if let Ok(val) = std::env::var("ECMEM_REPOS") {
        if val.is_empty() {
            return Ok(map);
        }
        for segment in val.split(';') {
            let segment = segment.trim();
            if let Some((org, repos_str)) = segment.split_once(':') {
                let repos: Vec<String> = repos_str.split(',').map(|r| r.trim().to_string()).collect();
                map.insert(org.trim().to_string(), repos);
            }
            // If no colon, ignore — ambiguous which org it belongs to in multi-org mode
        }
    }
    Ok(map)
}

/// Derive the GitHub API URL from a hostname.
fn api_url_for_hostname(hostname: &str) -> String {
    if hostname == "github.com" {
        "https://api.github.com".to_string()
    } else {
        format!("https://{hostname}/api/v3")
    }
}

/// Extract the hostname from a GitHub API URL.
fn hostname_from_api_url(api_url: &str) -> String {
    if api_url == "https://api.github.com" {
        "github.com".to_string()
    } else {
        // "https://ghes.corp.com/api/v3" → "ghes.corp.com"
        api_url
            .strip_prefix("https://")
            .or_else(|| api_url.strip_prefix("http://"))
            .unwrap_or(api_url)
            .split('/')
            .next()
            .unwrap_or("github.com")
            .to_string()
    }
}

fn env_required(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("{key} environment variable is required"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
