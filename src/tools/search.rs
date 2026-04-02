use anyhow::Result;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::graph::client::{FalkorParam, GraphClient};
use crate::graph::queries;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchReposParams {
    /// Substring to match against repo names
    pub query: Option<String>,
    /// Filter by programming language
    pub language: Option<String>,
    /// Filter by topic
    pub topic: Option<String>,
    /// Filter by dependency name
    pub dependency: Option<String>,
    /// Maximum results (default 20)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRepoDetailsParams {
    /// Full name of the repo (e.g., "org/repo-name")
    pub repo: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindDependentsParams {
    /// Name of the dependency to search for
    pub dependency: String,
    /// Filter by ecosystem (e.g., "npm", "crates", "pypi")
    pub ecosystem: Option<String>,
    /// Maximum results (default 50)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FindRelatedParams {
    /// Full name of the repo to find related repos for
    pub repo: String,
    /// Type of relation: "dependencies", "team", "topics" (default: "dependencies")
    pub relation_type: Option<String>,
    /// Maximum results (default 10)
    pub limit: Option<i64>,
}

pub async fn search_repos(graph: &GraphClient, params: SearchReposParams) -> Result<String> {
    let limit = params.limit.unwrap_or(20);

    let (query_str, query_params): (&str, Vec<(&str, FalkorParam)>) =
        if let Some(ref dep) = params.dependency {
            (
                queries::SEARCH_REPOS_BY_DEPENDENCY,
                vec![
                    ("dependency", dep.clone().into()),
                    ("limit", FalkorParam::Int(limit)),
                ],
            )
        } else if let Some(ref lang) = params.language {
            (
                queries::SEARCH_REPOS_BY_LANGUAGE,
                vec![
                    ("language", lang.clone().into()),
                    ("limit", FalkorParam::Int(limit)),
                ],
            )
        } else if let Some(ref topic) = params.topic {
            (
                queries::SEARCH_REPOS_BY_TOPIC,
                vec![
                    ("topic", topic.clone().into()),
                    ("limit", FalkorParam::Int(limit)),
                ],
            )
        } else {
            let q = params.query.as_deref().unwrap_or("");
            (
                queries::SEARCH_REPOS,
                vec![
                    ("query", q.into()),
                    ("limit", FalkorParam::Int(limit)),
                ],
            )
        };

    graph.execute(query_str, &query_params).await
}

pub async fn get_repo_details(graph: &GraphClient, params: GetRepoDetailsParams) -> Result<String> {
    graph
        .execute(
            queries::GET_REPO_DETAILS,
            &[("full_name", params.repo.into())],
        )
        .await
}

pub async fn find_dependents(graph: &GraphClient, params: FindDependentsParams) -> Result<String> {
    let limit = params.limit.unwrap_or(50);
    graph
        .execute(
            queries::FIND_DEPENDENTS,
            &[
                ("dependency", params.dependency.into()),
                ("limit", FalkorParam::Int(limit)),
            ],
        )
        .await
}

pub async fn find_related(graph: &GraphClient, params: FindRelatedParams) -> Result<String> {
    let limit = params.limit.unwrap_or(10);
    let relation = params.relation_type.as_deref().unwrap_or("dependencies");

    let query = match relation {
        "team" => queries::FIND_RELATED_BY_TEAM,
        "topics" => queries::FIND_RELATED_BY_TOPIC,
        _ => queries::FIND_RELATED_BY_DEPS,
    };

    graph
        .execute(
            query,
            &[
                ("repo", params.repo.into()),
                ("limit", FalkorParam::Int(limit)),
            ],
        )
        .await
}
