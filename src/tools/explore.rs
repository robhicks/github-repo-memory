use anyhow::Result;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::graph::client::{FalkorParam, GraphClient};
use crate::graph::queries;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExploreDepsParams {
    /// Full name of the repo (e.g., "org/repo-name")
    pub repo: String,
    /// Direction: "upstream" (what this repo depends on) or "downstream" (what depends on this repo)
    pub direction: Option<String>,
    /// Maximum results (default 20)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrgStatsParams {
    /// Organization login name (uses configured org if omitted)
    pub org: Option<String>,
}

pub async fn explore_dependency_graph(
    graph: &GraphClient,
    params: ExploreDepsParams,
) -> Result<String> {
    let limit = params.limit.unwrap_or(20);
    let direction = params.direction.as_deref().unwrap_or("upstream");

    let query = match direction {
        "downstream" => queries::EXPLORE_DEPS_DOWNSTREAM,
        _ => queries::EXPLORE_DEPS_UPSTREAM,
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

pub async fn list_languages(graph: &GraphClient) -> Result<String> {
    graph.execute(queries::LIST_LANGUAGES, &[]).await
}

pub async fn list_teams(graph: &GraphClient) -> Result<String> {
    graph.execute(queries::LIST_TEAMS, &[]).await
}

pub async fn get_org_stats(graph: &GraphClient, org: &str) -> Result<String> {
    graph
        .execute(queries::GET_ORG_STATS, &[("org", org.into())])
        .await
}
