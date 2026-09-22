use anyhow::{Context, Result};

use crate::domain::manifest::{McpServer, SecretRef};
use crate::ports::SecretStore;

/// Environment for a server, with every secret reference resolved.
pub fn resolve_env(server: &McpServer, secrets: &dyn SecretStore) -> Result<Vec<(String, String)>> {
    server
        .env
        .iter()
        .map(|(key, value)| {
            let resolved = match SecretRef::parse(value) {
                Some(reference) => secrets
                    .get(&reference)
                    .with_context(|| format!("resolving env `{key}`"))?,
                None => value.clone(),
            };
            Ok((key.clone(), resolved))
        })
        .collect()
}
