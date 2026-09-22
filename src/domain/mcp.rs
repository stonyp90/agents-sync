use std::path::Path;

use serde_json::{json, Map, Value};

use super::manifest::McpServer;
use super::target::Target;

/// Command that starts a secret-bearing server through agents-sync, so the
/// secret is read from the store at launch and never written to disk.
pub struct Launcher<'a> {
    pub exe: &'a Path,
    pub manifest: &'a Path,
}

/// One server in the dialect of `target`.
pub fn render(target: Target, name: &str, server: &McpServer, launcher: &Launcher) -> Value {
    let mut entry = Map::new();
    if let Some(url) = &server.url {
        if matches!(target, Target::Claude | Target::Copilot | Target::Qoder) {
            entry.insert("type".into(), json!("http"));
        }
        entry.insert("url".into(), json!(url));
        if !server.headers.is_empty() {
            let key = if target == Target::Codex {
                "http_headers"
            } else {
                "headers"
            };
            entry.insert(key.into(), json!(server.headers));
        }
    } else {
        let (command, args, env) = if server.uses_secrets() {
            let args = vec![
                "exec-mcp".to_string(),
                name.to_string(),
                "--manifest".to_string(),
                launcher.manifest.display().to_string(),
            ];
            (launcher.exe.display().to_string(), args, Default::default())
        } else {
            (
                server.command.clone().unwrap_or_default(),
                server.args.clone(),
                server.env.clone(),
            )
        };
        match target {
            Target::Claude => {
                entry.insert("type".into(), json!("stdio"));
            }
            Target::Copilot => {
                entry.insert("type".into(), json!("local"));
            }
            _ => {}
        }
        entry.insert("command".into(), json!(command));
        if !args.is_empty() {
            entry.insert("args".into(), json!(args));
        }
        if !env.is_empty() {
            entry.insert("env".into(), json!(env));
        }
    }
    if target == Target::Copilot {
        entry.insert("tools".into(), json!(["*"]));
    }
    Value::Object(entry)
}
