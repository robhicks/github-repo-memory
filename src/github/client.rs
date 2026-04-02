use anyhow::{Context, Result};
use governor::{Quota, RateLimiter as GovRateLimiter};
use octocrab::Octocrab;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use tracing::debug;

use crate::config::AppConfig;
use crate::graph::models::Team;

type Limiter = GovRateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

/// Wrapper around octocrab with rate limiting.
pub struct GitHubClient {
    octocrab: Octocrab,
    org: String,
    rate_limiter: Arc<Limiter>,
}

impl GitHubClient {
    pub fn new(config: &AppConfig, token: &str) -> Result<Self> {
        let mut builder = Octocrab::builder().personal_token(token.to_string());

        if config.github_api_url != "https://api.github.com" {
            builder = builder.base_uri(&config.github_api_url)?;
        }

        let octocrab = builder.build().context("Failed to build GitHub client")?;

        // ~75 requests per minute (4500/hour with buffer)
        let quota = Quota::per_minute(NonZeroU32::new(75).unwrap());
        let rate_limiter = Arc::new(GovRateLimiter::direct(quota));

        Ok(Self {
            octocrab,
            org: config.github_org.clone(),
            rate_limiter,
        })
    }

    async fn rate_limit(&self) {
        self.rate_limiter.until_ready().await;
    }

    /// List repositories for the organization (or user), paginated.
    /// Tries the org endpoint first; falls back to the user endpoint if that fails.
    pub async fn list_repos(
        &self,
        page: u32,
        per_page: u8,
    ) -> Result<Vec<octocrab::models::Repository>> {
        self.rate_limit().await;
        debug!("Listing repos page={page} per_page={per_page}");

        // Try org endpoint first
        let org_result = self
            .octocrab
            .orgs(&self.org)
            .list_repos()
            .sort(octocrab::params::repos::Sort::Updated)
            .direction(octocrab::params::Direction::Descending)
            .per_page(per_page)
            .page(page)
            .send()
            .await;

        match org_result {
            Ok(p) => Ok(p.items),
            Err(_) => {
                debug!("Org endpoint failed, falling back to user repos for {}", self.org);
                self.rate_limit().await;
                let p: octocrab::Page<octocrab::models::Repository> = self
                    .octocrab
                    .get(
                        format!("/users/{}/repos?sort=updated&direction=desc&per_page={}&page={}", self.org, per_page, page),
                        None::<&()>,
                    )
                    .await
                    .context("Failed to list repos (tried both org and user endpoints)")?;
                Ok(p.items)
            }
        }
    }

    /// Get languages for a repository with byte counts.
    pub async fn get_repo_languages(&self, repo: &str) -> Result<HashMap<String, u64>> {
        self.rate_limit().await;
        debug!("Fetching languages for {repo}");

        let languages: HashMap<String, u64> = self
            .octocrab
            .get(format!("/repos/{}/{}/languages", self.org, repo), None::<&()>)
            .await
            .context("Failed to fetch repo languages")?;

        Ok(languages)
    }

    /// Get topics for a repository.
    pub async fn get_repo_topics(&self, repo: &str) -> Result<Vec<String>> {
        self.rate_limit().await;
        debug!("Fetching topics for {repo}");

        #[derive(serde::Deserialize)]
        struct TopicsResponse {
            names: Vec<String>,
        }

        let response: TopicsResponse = self
            .octocrab
            .get(
                format!("/repos/{}/{}/topics", self.org, repo),
                None::<&()>,
            )
            .await
            .context("Failed to fetch repo topics")?;

        Ok(response.names)
    }

    /// Get teams with access to a repository.
    pub async fn get_repo_teams(&self, repo: &str) -> Result<Vec<Team>> {
        self.rate_limit().await;
        debug!("Fetching teams for {repo}");

        #[derive(serde::Deserialize)]
        struct TeamResponse {
            slug: String,
            name: String,
            permission: Option<String>,
        }

        let teams: Vec<TeamResponse> = self
            .octocrab
            .get(
                format!("/repos/{}/{}/teams", self.org, repo),
                None::<&()>,
            )
            .await
            .unwrap_or_default();

        Ok(teams
            .into_iter()
            .map(|t| Team {
                slug: t.slug,
                name: t.name,
            })
            .collect())
    }

    /// Get the file tree for a repository (recursive, default branch).
    pub async fn get_tree(
        &self,
        repo: &str,
        sha: &str,
    ) -> Result<Vec<TreeEntry>> {
        self.rate_limit().await;
        debug!("Fetching tree for {repo}@{sha}");

        #[derive(serde::Deserialize)]
        struct TreeResponse {
            tree: Vec<TreeEntryRaw>,
            truncated: bool,
        }

        #[derive(serde::Deserialize)]
        struct TreeEntryRaw {
            path: String,
            #[serde(rename = "type")]
            entry_type: String,
            size: Option<u64>,
        }

        let response: TreeResponse = self
            .octocrab
            .get(
                format!(
                    "/repos/{}/{}/git/trees/{}?recursive=1",
                    self.org, repo, sha
                ),
                None::<&()>,
            )
            .await
            .context("Failed to fetch repo tree")?;

        if response.truncated {
            tracing::warn!("Tree for {repo} was truncated (>100k entries)");
        }

        Ok(response
            .tree
            .into_iter()
            .filter(|e| e.entry_type == "blob")
            .map(|e| TreeEntry {
                path: e.path,
                size: e.size.unwrap_or(0),
            })
            .collect())
    }

    /// Get raw file content from a repository.
    pub async fn get_file_content(&self, repo: &str, path: &str) -> Result<String> {
        self.rate_limit().await;
        debug!("Fetching file content {repo}/{path}");

        #[derive(serde::Deserialize)]
        struct ContentResponse {
            content: Option<String>,
            encoding: Option<String>,
        }

        let response: ContentResponse = self
            .octocrab
            .get(
                format!("/repos/{}/{}/contents/{}", self.org, repo, path),
                None::<&()>,
            )
            .await
            .context("Failed to fetch file content")?;

        match (response.content, response.encoding) {
            (Some(content), Some(encoding)) if encoding == "base64" => {
                let cleaned = content.replace('\n', "");
                let bytes = base64_decode(&cleaned)?;
                String::from_utf8(bytes).context("File content is not valid UTF-8")
            }
            (Some(content), _) => Ok(content),
            _ => Ok(String::new()),
        }
    }

    pub fn org(&self) -> &str {
        &self.org
    }
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: String,
    pub size: u64,
}

fn base64_decode(input: &str) -> Result<Vec<u8>> {
    // Simple base64 decoder (standard alphabet)
    use std::io::Read;
    let mut decoder = base64_reader(input.as_bytes());
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf)?;
    Ok(buf)
}

fn base64_reader(input: &[u8]) -> impl std::io::Read + '_ {
    struct Base64Reader<'a> {
        input: &'a [u8],
        pos: usize,
        buf: [u8; 3],
        buf_len: usize,
        buf_pos: usize,
    }

    impl<'a> std::io::Read for Base64Reader<'a> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let mut written = 0;
            while written < out.len() {
                if self.buf_pos < self.buf_len {
                    out[written] = self.buf[self.buf_pos];
                    self.buf_pos += 1;
                    written += 1;
                    continue;
                }
                // Decode next 4-char block
                let mut block = [0u8; 4];
                let mut block_len = 0;
                while block_len < 4 && self.pos < self.input.len() {
                    let c = self.input[self.pos];
                    self.pos += 1;
                    if let Some(v) = decode_char(c) {
                        block[block_len] = v;
                        block_len += 1;
                    }
                }
                if block_len == 0 {
                    break;
                }
                self.buf[0] = (block[0] << 2) | (block[1] >> 4);
                self.buf_len = 1;
                if block_len > 2 {
                    self.buf[1] = (block[1] << 4) | (block[2] >> 2);
                    self.buf_len = 2;
                }
                if block_len > 3 {
                    self.buf[2] = (block[2] << 6) | block[3];
                    self.buf_len = 3;
                }
                self.buf_pos = 0;
            }
            Ok(written)
        }
    }

    fn decode_char(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    Base64Reader {
        input,
        pos: 0,
        buf: [0; 3],
        buf_len: 0,
        buf_pos: 0,
    }
}
