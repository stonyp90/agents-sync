use agents_sync::app::exec_mcp::resolve_env;
use agents_sync::app::watch::{plist, WatchSpec};
use agents_sync::domain::manifest::{Manifest, SecretRef};
use agents_sync::ports::SecretStore;

#[test]
fn a_manifest_lists_every_problem_at_once() {
    let err = Manifest::parse(
        r#"
version = 1
[[hooks]]
event = "OnLunch"
command = "x"
[mcp.both]
command = "a"
url = "https://b"
[mcp.leaky]
url = "https://c"
headers = { Authorization = "keychain:token" }
[targets.vim]
"#,
    )
    .unwrap_err()
    .to_string();

    for needle in ["OnLunch", "mcp.both", "mcp.leaky", "`vim`"] {
        assert!(err.contains(needle), "missing {needle} in:\n{err}");
    }
}

#[test]
fn secret_references_are_recognised_by_prefix() {
    assert_eq!(
        SecretRef::parse("keychain:svc"),
        Some(SecretRef::Keychain {
            service: "svc".into()
        })
    );
    assert_eq!(
        SecretRef::parse("env:TOKEN"),
        Some(SecretRef::Env {
            var: "TOKEN".into()
        })
    );
    assert_eq!(SecretRef::parse("plain"), None);
}

struct FakeSecrets;
impl SecretStore for FakeSecrets {
    fn get(&self, r: &SecretRef) -> anyhow::Result<String> {
        match r {
            SecretRef::Keychain { service } if service == "gh" => Ok("s3cret".into()),
            _ => anyhow::bail!("unknown"),
        }
    }
}

#[test]
fn the_launcher_resolves_secrets_and_passes_plain_values_through() {
    let m = Manifest::parse(
        r#"
version = 1
[mcp.github]
command = "npx"
env = { TOKEN = "keychain:gh", MODE = "fast" }
"#,
    )
    .unwrap();
    let env = resolve_env(&m.mcp["github"], &FakeSecrets).unwrap();
    assert_eq!(
        env,
        vec![
            ("MODE".into(), "fast".into()),
            ("TOKEN".into(), "s3cret".into())
        ]
    );
}

#[test]
fn a_missing_secret_names_the_variable() {
    let m = Manifest::parse(
        "version = 1\n[mcp.x]\ncommand = \"a\"\nenv = { TOKEN = \"keychain:nope\" }\n",
    )
    .unwrap();
    let err = format!("{:#}", resolve_env(&m.mcp["x"], &FakeSecrets).unwrap_err());
    assert!(err.contains("TOKEN"), "{err}");
}

#[test]
fn the_watch_agent_reruns_sync_when_a_shared_path_changes() {
    let xml = plist(&WatchSpec {
        label: "io.github.agents-sync".into(),
        exe: "/bin/agents-sync".into(),
        manifest: "/h/.agents/agents.toml".into(),
        watch_paths: vec!["/h/.agents/agents.toml".into(), "/h/.agents/skills".into()],
        log: "/h/.agents/logs/sync.log".into(),
        throttle_seconds: 2,
    });
    assert!(xml.contains("<string>io.github.agents-sync</string>"));
    assert!(xml.contains("<string>sync</string>"));
    assert!(xml.contains("<string>/h/.agents/skills</string>"));
    assert!(xml.contains("<integer>2</integer>"));
}
