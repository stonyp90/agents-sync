use serde_json::{json, Map, Value};

use super::manifest::Hook;

/// Events the manifest accepts. Names follow Claude Code, which Codex and
/// Qoder copy and Copilot accepts in its compatible (PascalCase) mode.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
    "PostCompact",
    "Notification",
];

/// Claude Code, Codex and Qoder shape:
/// `{Event: [{matcher?, hooks: [{type, command, timeout?}]}]}`.
pub fn claude_shape(hooks: &[&Hook]) -> Value {
    let mut events = Map::new();
    for hook in hooks {
        let groups = events
            .entry(hook.event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("event groups are arrays");
        let mut entry = Map::new();
        entry.insert("type".into(), json!("command"));
        entry.insert("command".into(), json!(hook.command));
        if let Some(timeout) = hook.timeout {
            entry.insert("timeout".into(), json!(timeout));
        }
        let same_matcher =
            |g: &&mut Value| g.get("matcher").and_then(Value::as_str) == hook.matcher.as_deref();
        match groups.iter_mut().find(same_matcher) {
            Some(group) => group["hooks"]
                .as_array_mut()
                .expect("group hooks are arrays")
                .push(Value::Object(entry)),
            None => {
                let mut group = Map::new();
                if let Some(matcher) = &hook.matcher {
                    group.insert("matcher".into(), json!(matcher));
                }
                group.insert("hooks".into(), json!([entry]));
                groups.push(Value::Object(group));
            }
        }
    }
    Value::Object(events)
}

/// Copilot hook file: `{version: 1, hooks: {Event: [{type, bash, matcher?, timeoutSec?}]}}`.
/// PascalCase event names make Copilot use Claude payloads and tool names,
/// so the same hook scripts work unchanged.
pub fn copilot_shape(hooks: &[&Hook]) -> Value {
    let mut events = Map::new();
    for hook in hooks {
        let mut entry = Map::new();
        entry.insert("type".into(), json!("command"));
        entry.insert("bash".into(), json!(hook.command));
        if let Some(matcher) = &hook.matcher {
            entry.insert("matcher".into(), json!(matcher));
        }
        if let Some(timeout) = hook.timeout {
            entry.insert("timeoutSec".into(), json!(timeout));
        }
        events
            .entry(hook.event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("event entries are arrays")
            .push(Value::Object(entry));
    }
    json!({ "version": 1, "hooks": events })
}
