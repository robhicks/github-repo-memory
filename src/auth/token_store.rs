use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub github_login: String,
    pub github_token: String,
    pub validated_at: DateTime<Utc>,
    pub ttl_seconds: i64,
}

impl TokenInfo {
    pub fn is_expired(&self) -> bool {
        let elapsed = Utc::now() - self.validated_at;
        elapsed.num_seconds() > self.ttl_seconds
    }
}

#[derive(Debug, Clone)]
struct PkceInfo {
    challenge: String,
    method: String,
}

/// In-memory store for validated tokens and PKCE challenges.
#[derive(Debug, Clone)]
pub struct TokenStore {
    tokens: Arc<DashMap<String, TokenInfo>>,
    pkce: Arc<DashMap<String, PkceInfo>>,
    default_ttl: i64,
}

impl TokenStore {
    pub fn new(default_ttl_seconds: i64) -> Self {
        Self {
            tokens: Arc::new(DashMap::new()),
            pkce: Arc::new(DashMap::new()),
            default_ttl: default_ttl_seconds,
        }
    }

    pub fn insert(&self, bearer_token: &str, github_login: String, github_token: String) {
        self.tokens.insert(
            bearer_token.to_string(),
            TokenInfo {
                github_login,
                github_token,
                validated_at: Utc::now(),
                ttl_seconds: self.default_ttl,
            },
        );
    }

    pub fn get(&self, bearer_token: &str) -> Option<TokenInfo> {
        let entry = self.tokens.get(bearer_token)?;
        if entry.is_expired() {
            drop(entry);
            self.tokens.remove(bearer_token);
            return None;
        }
        Some(entry.clone())
    }

    pub fn remove(&self, bearer_token: &str) {
        self.tokens.remove(bearer_token);
    }

    /// Store a PKCE challenge for later verification.
    pub fn store_pkce(&self, state: &str, challenge: String, method: String) {
        self.pkce
            .insert(state.to_string(), PkceInfo { challenge, method });
    }

    /// Remove all expired entries.
    pub fn cleanup_expired(&self) {
        self.tokens.retain(|_, v| !v.is_expired());
    }
}
