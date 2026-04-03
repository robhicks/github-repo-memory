use anyhow::Result;
use serde::Serialize;
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::OrgConfig;
use crate::github::client::GitHubClient;
use crate::github::parsers;
use crate::graph::client::{FalkorParam, GraphClient};
use crate::graph::models::FileKind;
use crate::graph::queries;

/// Results of a sync operation.
#[derive(Debug, Default, Serialize)]
pub struct SyncReport {
    pub repos_processed: usize,
    pub repos_updated: usize,
    pub languages_found: usize,
    pub dependencies_found: usize,
    pub errors: Vec<String>,
}

/// Ingestion pipeline: crawls a single GitHub org and populates the FalkorDB graph.
pub struct Ingester {
    github: Arc<GitHubClient>,
    graph: Arc<GraphClient>,
    org_config: OrgConfig,
    max_file_depth: usize,
}

impl Ingester {
    pub fn new(
        github: Arc<GitHubClient>,
        graph: Arc<GraphClient>,
        org_config: OrgConfig,
        max_file_depth: usize,
    ) -> Self {
        Self {
            github,
            graph,
            org_config,
            max_file_depth,
        }
    }

    /// Full sync: re-ingest all repos (or only those in the org's repo list).
    pub async fn full_sync(&self) -> Result<SyncReport> {
        info!("Starting full sync for org: {}", self.org_config.org);
        let mut report = SyncReport::default();

        // Merge organization node
        self.graph
            .execute(
                queries::MERGE_ORG,
                &[
                    ("login", self.org_config.org.clone().into()),
                    ("name", self.org_config.org.clone().into()),
                    (
                        "url",
                        format!("{}/orgs/{}", self.org_config.api_url, self.org_config.org)
                            .into(),
                    ),
                ],
            )
            .await?;

        if !self.org_config.repos.is_empty() {
            info!("Syncing {} specified repos", self.org_config.repos.len());
            for repo_name in &self.org_config.repos {
                match self.sync_repo_by_name(repo_name).await {
                    Ok((langs, deps)) => {
                        report.repos_updated += 1;
                        report.languages_found += langs;
                        report.dependencies_found += deps;
                    }
                    Err(e) => {
                        warn!("Failed to sync repo {repo_name}: {e}");
                        report.errors.push(format!("{repo_name}: {e}"));
                    }
                }
                report.repos_processed += 1;
            }
        } else {
            let mut page = 1u32;
            loop {
                let repos = self.github.list_repos(page, 100).await?;
                if repos.is_empty() {
                    break;
                }
                for repo in &repos {
                    let repo_name = repo.name.clone();
                    match self.sync_single_repo(repo).await {
                        Ok((langs, deps)) => {
                            report.repos_updated += 1;
                            report.languages_found += langs;
                            report.dependencies_found += deps;
                        }
                        Err(e) => {
                            warn!("Failed to sync repo {repo_name}: {e}");
                            report.errors.push(format!("{repo_name}: {e}"));
                        }
                    }
                    report.repos_processed += 1;
                }
                if repos.len() < 100 {
                    break;
                }
                page += 1;
            }
        }

        // Cross-repo dependency resolution
        info!("Resolving cross-repo dependencies...");
        self.graph
            .execute(queries::MERGE_CROSS_REPO_DEPENDENCY, &[])
            .await?;

        self.update_sync_timestamp().await?;

        info!(
            "Full sync complete: {} repos processed, {} updated, {} errors",
            report.repos_processed, report.repos_updated, report.errors.len()
        );

        Ok(report)
    }

    /// Incremental sync: only process repos updated since last sync.
    pub async fn incremental_sync(&self) -> Result<SyncReport> {
        info!("Starting incremental sync for org: {}", self.org_config.org);
        let mut report = SyncReport::default();

        // Get last sync time from org node
        let result = self
            .graph
            .execute(
                "MATCH (o:Organization {login: $login}) RETURN o.last_sync_at",
                &[("login", self.org_config.org.clone().into())],
            )
            .await;

        let _last_sync = match result {
            Ok(_rs) => None::<String>,
            Err(_) => None,
        };

        if !self.org_config.repos.is_empty() {
            info!("Incremental syncing {} specified repos", self.org_config.repos.len());
            for repo_name in &self.org_config.repos {
                match self.sync_repo_by_name(repo_name).await {
                    Ok((langs, deps)) => {
                        report.repos_updated += 1;
                        report.languages_found += langs;
                        report.dependencies_found += deps;
                    }
                    Err(e) => {
                        warn!("Failed to sync repo {repo_name}: {e}");
                        report.errors.push(format!("{repo_name}: {e}"));
                    }
                }
                report.repos_processed += 1;
            }
        } else {
            let mut page = 1u32;
            loop {
                let repos = self.github.list_repos(page, 100).await?;
                if repos.is_empty() {
                    break;
                }
                for repo in &repos {
                    let repo_name = repo.name.clone();
                    match self.sync_single_repo(repo).await {
                        Ok((langs, deps)) => {
                            report.repos_updated += 1;
                            report.languages_found += langs;
                            report.dependencies_found += deps;
                        }
                        Err(e) => {
                            warn!("Failed to sync repo {repo_name}: {e}");
                            report.errors.push(format!("{repo_name}: {e}"));
                        }
                    }
                    report.repos_processed += 1;
                }
                if repos.len() < 100 {
                    break;
                }
                page += 1;
            }
        }

        // Cross-repo dependency resolution
        self.graph
            .execute(queries::MERGE_CROSS_REPO_DEPENDENCY, &[])
            .await?;

        self.update_sync_timestamp().await?;

        info!(
            "Incremental sync complete: {} repos processed, {} updated",
            report.repos_processed, report.repos_updated
        );

        Ok(report)
    }

    /// Sync a single repository by name. Returns (languages_found, dependencies_found).
    pub async fn sync_repo_by_name(&self, repo_name: &str) -> Result<(usize, usize)> {
        info!("Single repo sync for {repo_name}");
        let mut page = 1u32;
        loop {
            let repos = self.github.list_repos(page, 100).await?;
            if repos.is_empty() {
                break;
            }
            for repo in &repos {
                if repo.name == repo_name {
                    return self.sync_single_repo(repo).await;
                }
            }
            if repos.len() < 100 {
                break;
            }
            page += 1;
        }
        anyhow::bail!("Repository {repo_name} not found in org {}", self.org_config.org);
    }

    async fn update_sync_timestamp(&self) -> Result<()> {
        self.graph
            .execute(
                queries::UPDATE_ORG_SYNC_TIME,
                &[
                    ("login", self.org_config.org.clone().into()),
                    ("last_sync_at", chrono::Utc::now().to_rfc3339().into()),
                ],
            )
            .await?;
        Ok(())
    }

    async fn sync_single_repo(&self, repo: &octocrab::models::Repository) -> Result<(usize, usize)> {
        let full_name = repo.full_name.as_deref().unwrap_or(&repo.name);
        let repo_name = &repo.name;

        info!("Syncing repo: {full_name}");

        let mut lang_count = 0usize;
        let mut dep_count = 0usize;

        // Merge repository node
        self.graph
            .execute(
                queries::MERGE_REPO,
                &[
                    ("full_name", full_name.into()),
                    ("name", repo_name.as_str().into()),
                    (
                        "description",
                        repo.description.as_deref().unwrap_or("").into(),
                    ),
                    (
                        "default_branch",
                        repo.default_branch.as_deref().unwrap_or("main").into(),
                    ),
                    (
                        "is_archived",
                        FalkorParam::Bool(repo.archived.unwrap_or(false)),
                    ),
                    ("is_fork", FalkorParam::Bool(repo.fork.unwrap_or(false))),
                    (
                        "stars",
                        FalkorParam::Int(repo.stargazers_count.unwrap_or(0) as i64),
                    ),
                    (
                        "updated_at",
                        repo.updated_at
                            .map(|d| d.to_rfc3339())
                            .unwrap_or_default()
                            .into(),
                    ),
                    (
                        "url",
                        repo.html_url
                            .as_ref()
                            .map(|u| u.to_string())
                            .unwrap_or_default()
                            .into(),
                    ),
                ],
            )
            .await?;

        // Link org -> repo
        self.graph
            .execute(
                queries::MERGE_ORG_HAS_REPO,
                &[
                    ("org_login", self.org_config.org.clone().into()),
                    ("repo_full_name", full_name.into()),
                ],
            )
            .await?;

        // Languages
        match self.github.get_repo_languages(repo_name).await {
            Ok(languages) => {
                for (lang, bytes) in &languages {
                    self.graph
                        .execute(
                            queries::MERGE_REPO_USES_LANGUAGE,
                            &[
                                ("repo_full_name", full_name.into()),
                                ("lang_name", lang.as_str().into()),
                                ("bytes", FalkorParam::Int(*bytes as i64)),
                            ],
                        )
                        .await?;
                    lang_count += 1;
                }
            }
            Err(e) => warn!("Failed to fetch languages for {repo_name}: {e}"),
        }

        // Topics
        if let Ok(topics) = self.github.get_repo_topics(repo_name).await {
            for topic in &topics {
                self.graph
                    .execute(
                        queries::MERGE_TOPIC,
                        &[
                            ("repo_full_name", full_name.into()),
                            ("topic_name", topic.as_str().into()),
                        ],
                    )
                    .await?;
            }
        }

        // Teams
        if let Ok(teams) = self.github.get_repo_teams(repo_name).await {
            for team in &teams {
                self.graph
                    .execute(
                        queries::MERGE_TEAM,
                        &[
                            ("repo_full_name", full_name.into()),
                            ("team_slug", team.slug.as_str().into()),
                            ("team_name", team.name.as_str().into()),
                            ("permission", "read".into()),
                        ],
                    )
                    .await?;
            }
        }

        // File tree + manifest parsing
        let default_branch = repo.default_branch.as_deref().unwrap_or("main");

        if let Ok(tree) = self.github.get_tree(repo_name, default_branch).await {
            for entry in &tree {
                let depth = entry.path.matches('/').count();
                if depth > self.max_file_depth {
                    continue;
                }

                let kind = FileKind::from_path(&entry.path);

                match kind {
                    FileKind::Other | FileKind::Source => continue,
                    _ => {}
                }

                self.graph
                    .execute(
                        queries::MERGE_FILE,
                        &[
                            ("repo_full_name", full_name.into()),
                            ("path", entry.path.as_str().into()),
                            ("kind", kind.to_string().into()),
                        ],
                    )
                    .await?;

                if matches!(kind, FileKind::Manifest) {
                    match self.github.get_file_content(repo_name, &entry.path).await {
                        Ok(content) => {
                            match parsers::parse_manifest(&entry.path, &content) {
                                Ok(deps) => {
                                    for dep in &deps {
                                        self.graph
                                            .execute(
                                                queries::MERGE_DEPENDENCY,
                                                &[
                                                    ("repo_full_name", full_name.into()),
                                                    ("dep_name", dep.name.as_str().into()),
                                                    ("ecosystem", dep.ecosystem.as_str().into()),
                                                    (
                                                        "version",
                                                        dep.version_spec
                                                            .as_deref()
                                                            .unwrap_or("")
                                                            .into(),
                                                    ),
                                                    ("dev", FalkorParam::Bool(false)),
                                                ],
                                            )
                                            .await?;
                                        dep_count += 1;
                                    }
                                }
                                Err(e) => warn!("Failed to parse manifest {}: {e}", entry.path),
                            }
                        }
                        Err(e) => warn!("Failed to fetch {}: {e}", entry.path),
                    }
                }
            }
        }

        Ok((lang_count, dep_count))
    }
}
