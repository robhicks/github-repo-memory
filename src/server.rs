use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool_handler, tool_router, ErrorData, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

use crate::auth::token_store::TokenStore;
use crate::config::AppConfig;
use crate::github::client::GitHubClient;
use crate::github::ingest::{Ingester, SyncReport};
use crate::graph::client::GraphClient;
use crate::tools::{explore, search};

/// MCP server providing enterprise code memory tools.
#[derive(Clone)]
pub struct CodeMemoryServer {
    graph: Arc<GraphClient>,
    config: Arc<AppConfig>,
    token_store: TokenStore,
    tool_router: ToolRouter<Self>,
}

impl CodeMemoryServer {
    pub fn new(
        graph: Arc<GraphClient>,
        config: Arc<AppConfig>,
        token_store: TokenStore,
    ) -> Self {
        Self {
            graph,
            config,
            token_store,
            tool_router: Self::tool_router(),
        }
    }

    /// Create an Ingester for each configured org.
    fn ingesters(&self) -> Result<Vec<Ingester>, ErrorData> {
        let token_info = self.token_store.get_any_valid().ok_or_else(|| {
            let msg = if self.config.auth_mode == crate::config::AuthMode::GhCli {
                "GitHub CLI token expired. Restart the server to refresh."
            } else {
                "No valid GitHub token found. Please re-authenticate."
            };
            ErrorData::internal_error(msg.to_string(), None)
        })?;

        let mut ingesters = Vec::new();
        for org_config in &self.config.orgs {
            // In gh_cli mode with multiple hostnames, look up the per-hostname token
            let token = if self.config.auth_mode == crate::config::AuthMode::GhCli {
                self.token_store
                    .get_by_hostname(&org_config.hostname)
                    .map(|t| t.github_token.clone())
                    .unwrap_or_else(|| token_info.github_token.clone())
            } else {
                token_info.github_token.clone()
            };

            let github = Arc::new(
                GitHubClient::new(org_config, &token)
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            );
            ingesters.push(Ingester::new(
                github,
                self.graph.clone(),
                org_config.clone(),
                self.config.max_file_depth,
            ));
        }
        Ok(ingesters)
    }
}

// --- Tool parameter types ---

#[derive(Debug, Deserialize, JsonSchema)]
struct SearchReposInput {
    /// Substring to match against repo names
    query: Option<String>,
    /// Filter by programming language
    language: Option<String>,
    /// Filter by topic
    topic: Option<String>,
    /// Filter by dependency name
    dependency: Option<String>,
    /// Maximum results (default 20)
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RepoInput {
    /// Full name of the repo (e.g., "org/repo-name")
    repo: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindDependentsInput {
    /// Name of the dependency to search for
    dependency: String,
    /// Filter by ecosystem (e.g., "npm", "crates", "pypi")
    ecosystem: Option<String>,
    /// Maximum results (default 50)
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FindRelatedInput {
    /// Full name of the repo to find related repos for
    repo: String,
    /// Type of relation: "dependencies", "team", "topics" (default: "dependencies")
    relation_type: Option<String>,
    /// Maximum results (default 10)
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExploreDepsInput {
    /// Full name of the repo (e.g., "org/repo-name")
    repo: String,
    /// Direction: "upstream" (what this repo depends on) or "downstream" (what depends on this repo)
    direction: Option<String>,
    /// Maximum results (default 20)
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SyncOrgInput {
    /// Sync mode: "full" (re-index everything) or "incremental" (only changed repos, default)
    mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SyncRepoInput {
    /// Name of the repository to sync
    repo: String,
}

// --- Tool implementations ---

#[tool_router]
impl CodeMemoryServer {
    #[rmcp::tool(description = "Search repositories by name, language, topic, or dependency")]
    async fn search_repos(
        &self,
        Parameters(input): Parameters<SearchReposInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = search::SearchReposParams {
            query: input.query,
            language: input.language,
            topic: input.topic,
            dependency: input.dependency,
            limit: input.limit,
        };
        let result = search::search_repos(&self.graph, params)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "Get detailed metadata for a specific repository including languages, topics, teams, and dependency counts")]
    async fn get_repo_details(
        &self,
        Parameters(input): Parameters<RepoInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = search::GetRepoDetailsParams { repo: input.repo };
        let result = search::get_repo_details(&self.graph, params)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "Find all repos that depend on a given library or internal repo")]
    async fn find_dependents(
        &self,
        Parameters(input): Parameters<FindDependentsInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = search::FindDependentsParams {
            dependency: input.dependency,
            ecosystem: input.ecosystem,
            limit: input.limit,
        };
        let result = search::find_dependents(&self.graph, params)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "Find repos related to a given repo by shared dependencies, same team, or similar topics")]
    async fn find_related_repos(
        &self,
        Parameters(input): Parameters<FindRelatedInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = search::FindRelatedParams {
            repo: input.repo,
            relation_type: input.relation_type,
            limit: input.limit,
        };
        let result = search::find_related(&self.graph, params)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "Explore the dependency graph for a repo - find what it depends on (upstream) or what depends on it (downstream)")]
    async fn explore_dependency_graph(
        &self,
        Parameters(input): Parameters<ExploreDepsInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = explore::ExploreDepsParams {
            repo: input.repo,
            direction: input.direction,
            limit: input.limit,
        };
        let result = explore::explore_dependency_graph(&self.graph, params)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "List all programming languages used across all organizations with repo counts")]
    async fn list_languages(&self) -> Result<CallToolResult, ErrorData> {
        let result = explore::list_languages(&self.graph)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "List all teams with their repo counts")]
    async fn list_teams(&self) -> Result<CallToolResult, ErrorData> {
        let result = explore::list_teams(&self.graph)
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[rmcp::tool(description = "Get organization summary statistics: total repos, languages, dependencies, teams. Reports across all configured orgs.")]
    async fn get_org_stats(&self) -> Result<CallToolResult, ErrorData> {
        let mut results = Vec::new();
        for org_config in &self.config.orgs {
            let result = explore::get_org_stats(&self.graph, &org_config.org)
                .await
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            results.push(result);
        }
        Ok(CallToolResult::success(vec![Content::text(results.join("\n---\n"))]))
    }

    #[rmcp::tool(description = "Sync repos from GitHub into the knowledge graph. Syncs all configured organizations. Mode: 'full' (re-index everything) or 'incremental' (only changed repos, default)")]
    async fn sync_org(
        &self,
        Parameters(input): Parameters<SyncOrgInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let ingesters = self.ingesters()?;
        let mut combined = SyncReport::default();

        for ingester in &ingesters {
            let report = match input.mode.as_deref().unwrap_or("incremental") {
                "full" => ingester.full_sync().await,
                _ => ingester.incremental_sync().await,
            }
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            combined.repos_processed += report.repos_processed;
            combined.repos_updated += report.repos_updated;
            combined.languages_found += report.languages_found;
            combined.dependencies_found += report.dependencies_found;
            combined.errors.extend(report.errors);
        }

        let json = serde_json::to_string_pretty(&combined)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[rmcp::tool(description = "Re-sync a single repository from GitHub. Searches across all configured organizations.")]
    async fn sync_repo(
        &self,
        Parameters(input): Parameters<SyncRepoInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let ingesters = self.ingesters()?;

        for ingester in &ingesters {
            match ingester.sync_repo_by_name(&input.repo).await {
                Ok(_) => {
                    return Ok(CallToolResult::success(vec![Content::text(format!(
                        "Successfully synced repo: {}",
                        input.repo
                    ))]));
                }
                Err(_) => continue,
            }
        }

        Err(ErrorData::internal_error(
            format!("Repository {} not found in any configured organization", input.repo),
            None,
        ))
    }
}

#[tool_handler]
impl ServerHandler for CodeMemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Enterprise code memory graph. Search repos, explore dependencies, \
                 find related projects across your GitHub organizations.",
            )
    }
}
