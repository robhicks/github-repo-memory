use anyhow::Result;
use tracing::info;

use super::client::GraphClient;
use super::queries::SCHEMA_INDICES;

/// Create all graph indices idempotently.
pub async fn ensure_schema(client: &GraphClient) -> Result<()> {
    info!("Ensuring graph schema indices...");

    for query in SCHEMA_INDICES {
        match client.execute(query, &[]).await {
            Ok(_) => {}
            Err(e) => {
                // Index may already exist — that's fine
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("Already indexed") {
                    tracing::debug!("Index already exists, skipping: {query}");
                } else {
                    tracing::warn!("Failed to create index (continuing): {e}");
                }
            }
        }
    }

    info!("Graph schema indices ensured");
    Ok(())
}
