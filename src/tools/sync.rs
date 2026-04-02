use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SyncOrgParams {
    /// Sync mode: "full" or "incremental" (default: "incremental")
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SyncRepoParams {
    /// Name of the repository to sync
    pub repo: String,
}
