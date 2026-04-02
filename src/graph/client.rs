use anyhow::{Context, Result};
use falkordb::FalkorClientBuilder;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

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
    ///
    /// FalkorDB's `with_params` uses the CYPHER parameter prefix format:
    /// `CYPHER key=value key2='string value' <query>`
    ///
    /// String values are single-quoted, integers and booleans are bare.
    pub async fn execute(
        &self,
        query: &str,
        params: &[(&str, FalkorParam)],
    ) -> Result<String> {
        let mut graph = self.graph.lock().await;

        let param_map: HashMap<String, String> = params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_cypher_param()))
            .collect();

        debug!(query = query, params = ?param_map, "Executing Cypher query");

        let result = if param_map.is_empty() {
            graph.query(query).execute().await
        } else {
            graph.query(query).with_params(&param_map).execute().await
        };

        let query_result = result.context("FalkorDB query failed")?;

        // Format result rows
        let mut output = Vec::new();
        for row in query_result.data {
            output.push(format!("{:?}", row));
        }

        if output.is_empty() {
            // Still include stats for write operations
            if !query_result.stats.is_empty() {
                let stats: Vec<String> = query_result
                    .stats
                    .iter()
                    .filter(|s| !s.contains("0 ")) // skip zero stats
                    .cloned()
                    .collect();
                if stats.is_empty() {
                    return Ok("Operation completed (no changes).".to_string());
                }
                return Ok(format!("Operation completed. {}", stats.join(", ")));
            }
            return Ok("No results found.".to_string());
        }

        Ok(output.join("\n"))
    }
}

/// Parameter types supported by FalkorDB queries.
///
/// FalkorDB's CYPHER prefix format requires:
/// - Strings: single-quoted with escaped single quotes
/// - Integers: bare numeric values
/// - Booleans: `true` or `false`
#[derive(Debug, Clone)]
pub enum FalkorParam {
    String(String),
    Int(i64),
    Bool(bool),
}

impl FalkorParam {
    /// Convert to FalkorDB CYPHER parameter format.
    ///
    /// Strings are single-quoted: `'hello'`
    /// Strings with single quotes are escaped: `'it\\'s'`
    /// Integers and booleans are bare: `42`, `true`
    pub fn to_cypher_param(&self) -> String {
        match self {
            FalkorParam::String(s) => {
                let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
                format!("'{escaped}'")
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_param_quoting() {
        let param = FalkorParam::String("hello world".to_string());
        assert_eq!(param.to_cypher_param(), "'hello world'");
    }

    #[test]
    fn test_string_param_escaping() {
        let param = FalkorParam::String("it's a test".to_string());
        assert_eq!(param.to_cypher_param(), "'it\\'s a test'");
    }

    #[test]
    fn test_int_param() {
        let param = FalkorParam::Int(42);
        assert_eq!(param.to_cypher_param(), "42");
    }

    #[test]
    fn test_bool_param() {
        let param = FalkorParam::Bool(true);
        assert_eq!(param.to_cypher_param(), "true");

        let param = FalkorParam::Bool(false);
        assert_eq!(param.to_cypher_param(), "false");
    }

    #[test]
    fn test_from_conversions() {
        let p: FalkorParam = "test".into();
        assert!(matches!(p, FalkorParam::String(s) if s == "test"));

        let p: FalkorParam = String::from("test").into();
        assert!(matches!(p, FalkorParam::String(s) if s == "test"));

        let p: FalkorParam = 42i64.into();
        assert!(matches!(p, FalkorParam::Int(42)));

        let p: FalkorParam = true.into();
        assert!(matches!(p, FalkorParam::Bool(true)));
    }
}
