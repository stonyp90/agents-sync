use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;

use super::hooks::EVENTS;
use super::target::Target;

pub const SUPPORTED_VERSION: u32 = 1;

/// The single file every tool's configuration is derived from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub paths: Paths,
    pub instructions: Option<Instructions>,
    pub skills: Option<Skills>,
    pub agents: Option<Agents>,
    #[serde(default)]
    pub hooks: Vec<Hook>,
    #[serde(default)]
    pub mcp: BTreeMap<String, McpServer>,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
    #[serde(default)]
    pub watch: Watch,
}

/// Where agents-sync keeps its own files. Relative paths resolve against the
/// manifest's directory.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    pub state: Option<String>,
    pub backups: Option<String>,
    /// The skills folder Codex and Copilot read on their own.
    pub native_skills: Option<String>,
    /// Binary written into MCP entries that need secrets. Defaults to the
    /// running executable.
    pub launcher: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instructions {
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skills {
    pub dir: String,
    /// Glob patterns of skills that stay out of the tools that need links.
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agents {
    pub dir: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout: Option<u64>,
    #[serde(default)]
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Values may be `keychain:<service>` or `env:<VAR>` references.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub skip: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub home: Option<String>,
    pub mcp_file: Option<String>,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watch {
    #[serde(default = "default_label")]
    pub label: String,
    pub log: Option<String>,
    #[serde(default = "default_throttle")]
    pub throttle_seconds: u32,
}

fn default_label() -> String {
    "io.github.agents-sync".into()
}

fn default_throttle() -> u32 {
    2
}

impl Default for Watch {
    fn default() -> Self {
        Watch {
            label: default_label(),
            log: None,
            throttle_seconds: default_throttle(),
        }
    }
}

/// A pointer to a secret, resolved only at the moment an MCP server starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRef {
    Keychain { service: String },
    Env { var: String },
}

impl SecretRef {
    pub fn parse(value: &str) -> Option<SecretRef> {
        if let Some(service) = value.strip_prefix("keychain:") {
            Some(SecretRef::Keychain {
                service: service.into(),
            })
        } else {
            value
                .strip_prefix("env:")
                .map(|var| SecretRef::Env { var: var.into() })
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            SecretRef::Keychain { service } => service.is_empty(),
            SecretRef::Env { var } => var.is_empty(),
        }
    }
}

impl McpServer {
    pub fn uses_secrets(&self) -> bool {
        self.env.values().any(|v| SecretRef::parse(v).is_some())
    }

    pub fn applies_to(&self, target: Target) -> bool {
        !self.skip.iter().any(|s| s == target.name())
    }
}

impl Hook {
    pub fn applies_to(&self, target: Target) -> bool {
        !self.skip.iter().any(|s| s == target.name())
    }
}

impl Manifest {
    pub fn parse(source: &str) -> Result<Manifest> {
        let manifest: Manifest =
            toml::from_str(source).map_err(|e| anyhow!("invalid manifest: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Targets that have a `[targets.<name>]` table without `enabled = false`.
    pub fn enabled_targets(&self) -> Vec<Target> {
        Target::ALL
            .into_iter()
            .filter(|t| self.targets.get(t.name()).is_some_and(|c| c.enabled))
            .collect()
    }

    fn validate(&self) -> Result<()> {
        let mut errors = Vec::new();
        if self.version != SUPPORTED_VERSION {
            errors.push(format!(
                "version must be {SUPPORTED_VERSION}, got {}",
                self.version
            ));
        }
        for name in self.targets.keys() {
            if Target::from_name(name).is_none() {
                errors.push(format!(
                    "unknown target `{name}` (known: {})",
                    Target::names()
                ));
            }
        }
        for (i, hook) in self.hooks.iter().enumerate() {
            let ctx = format!("hooks[{i}]");
            if !EVENTS.contains(&hook.event.as_str()) {
                errors.push(format!(
                    "{ctx}: unknown event `{}` (known: {})",
                    hook.event,
                    EVENTS.join(", ")
                ));
            }
            if hook.command.trim().is_empty() {
                errors.push(format!("{ctx}: empty command"));
            }
            check_skip(&hook.skip, &ctx, &mut errors);
        }
        for (name, server) in &self.mcp {
            let ctx = format!("mcp.{name}");
            if server.command.is_some() == server.url.is_some() {
                errors.push(format!("{ctx}: set exactly one of `command` or `url`"));
            }
            if server.url.is_some() && (!server.args.is_empty() || !server.env.is_empty()) {
                errors.push(format!(
                    "{ctx}: `args` and `env` only apply to `command` servers"
                ));
            }
            if server.command.is_some() && !server.headers.is_empty() {
                errors.push(format!("{ctx}: `headers` only apply to `url` servers"));
            }
            if server
                .headers
                .values()
                .any(|v| SecretRef::parse(v).is_some())
            {
                errors.push(format!(
                    "{ctx}: secret references are only supported in `env` of command servers"
                ));
            }
            for (key, value) in &server.env {
                if SecretRef::parse(value).is_some_and(|r| r.is_empty()) {
                    errors.push(format!("{ctx}.env.{key}: empty secret reference"));
                }
            }
            check_skip(&server.skip, &ctx, &mut errors);
        }
        if let Some(skills) = &self.skills {
            for pattern in &skills.exclude {
                if glob::Pattern::new(pattern).is_err() {
                    errors.push(format!("skills.exclude: invalid pattern `{pattern}`"));
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            bail!("invalid manifest:\n  - {}", errors.join("\n  - "))
        }
    }
}

fn check_skip(skip: &[String], ctx: &str, errors: &mut Vec<String>) {
    for name in skip {
        if Target::from_name(name).is_none() {
            errors.push(format!("{ctx}: unknown target `{name}` in skip"));
        }
    }
}
