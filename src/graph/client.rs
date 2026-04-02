use anyhow::{Context, Result};
use falkordb::FalkorClientBuilder;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use crate::config::AppConfig;

/// Wrapper around FalkorDB async graph connection.
///
/// FalkorDB's `AsyncGraph` is not thread-safe, so we wrap it in `Arc<Mutex<_>>`.
/// MCP tool calls from a single agent are sequential, so contention is minimal.
pub struct GraphClient {
    graph: Arc<Mutex<falkordb::AsyncGraph>>,
}

impl GraphClient {
    pub async fn connect(config: &AppConfig) -> Result<Self> {
        let builder = FalkorClientBuilder::new_async();

        let connection_info = if let Some(ref password) = config.falkordb_password {
            format!(
                "falkor://default:{}@{}:{}",
                password, config.falkordb_host, config.falkordb_port
            )
        } else {
            format!(
                "falkor://{}:{}",
                config.falkordb_host, config.falkordb_port
            )
        };

        let conn_info = connection_info
            .as_str()
            .try_into()
            .context("Invalid FalkorDB connection string")?;

        let client = builder
            .with_connection_info(conn_info)
            .build()
            .await
            .context("Failed to connect to FalkorDB")?;

        let graph = client.select_graph(&config.falkordb_graph_name);

        info!(
            "Connected to FalkorDB at {}:{}, graph: {}",
            config.falkordb_host, config.falkordb_port, config.falkordb_graph_name
        );

        Ok(Self {
            graph: Arc::new(Mutex::new(graph)),
        })
    }

    /// Execute a Cypher query with parameters.
    /// All parameter values are converted to strings since FalkorDB's
    /// with_params API accepts HashMap<String, String>.
    pub async fn execute(
        &self,
        query: &str,
        params: &[(&str, FalkorParam)],
    ) -> Result<String> {
        let mut graph = self.graph.lock().await;

        let param_map: HashMap<String, String> = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_cypher_string()))
            .collect();

        let result = if param_map.is_empty() {
            graph.query(query).execute().await
        } else {
            graph.query(query).with_params(&param_map).execute().await
        };

        let query_result = result.context("FalkorDB query failed")?;

        // Collect result data into a string representation
        let mut output = Vec::new();
        for row in query_result.data {
            output.push(format!("{:?}", row));
        }

        if !query_result.stats.is_empty() {
            output.push(format!("Stats: {:?}", query_result.stats));
        }

        Ok(output.join("\n"))
    }
}

/// Parameter types supported by FalkorDB queries.
/// FalkorDB's Rust client only accepts HashMap<String, String> for params,
/// so we serialize values into Cypher-compatible string representations.
#[derive(Debug, Clone)]
pub enum FalkorParam {
    String(String),
    Int(i64),
    Bool(bool),
}

impl FalkorParam {
    /// Convert to a Cypher-compatible string representation.
    pub fn to_cypher_string(&self) -> String {
        match self {
            FalkorParam::String(s) => s.clone(),
            FalkorParam::Int(i) => i.to_string(),
            FalkorParam::Bool(b) => b.to_string(),
        }
    }
}

impl From<&str> for FalkorParam {
    fn from(s: &str) -> Self {
        FalkorParam::String(s.to_string())
    }
}

impl From<String> for FalkorParam {
    fn from(s: String) -> Self {
        FalkorParam::String(s)
    }
}

impl From<i64> for FalkorParam {
    fn from(i: i64) -> Self {
        FalkorParam::Int(i)
    }
}

impl From<bool> for FalkorParam {
    fn from(b: bool) -> Self {
        FalkorParam::Bool(b)
    }
}
