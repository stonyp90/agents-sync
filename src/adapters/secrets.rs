use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::domain::manifest::SecretRef;
use crate::ports::SecretStore;

/// Reads `keychain:<service>` from the macOS login Keychain and `env:<VAR>`
/// from the process environment.
pub struct SystemSecrets;

impl SecretStore for SystemSecrets {
    fn get(&self, reference: &SecretRef) -> Result<String> {
        match reference {
            SecretRef::Keychain { service } => {
                let out = Command::new("security")
                    .args(["find-generic-password", "-s", service, "-w"])
                    .output()
                    .context("cannot run `security` (macOS Keychain CLI)")?;
                if !out.status.success() {
                    bail!(
                        "no Keychain item for service `{service}`; add it with: \
                         security add-generic-password -s {service} -a \"$USER\" -w"
                    );
                }
                let value = String::from_utf8(out.stdout).context("Keychain value is not UTF-8")?;
                Ok(value.trim_end_matches('\n').to_string())
            }
            SecretRef::Env { var } => std::env::var(var)
                .with_context(|| format!("environment variable `{var}` is not set")),
        }
    }
}
