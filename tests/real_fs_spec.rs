//! The same use case against the real filesystem, to prove the symlink,
//! backup and permission handling of the adapter.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use agents_sync::adapters::real_fs::RealFs;
use agents_sync::app::{self, Env};

#[test]
fn syncing_on_disk_links_backs_up_and_preserves_file_permissions() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().canonicalize().unwrap();
    let store = home.join(".agents");
    fs::create_dir_all(store.join("skills/alpha")).unwrap();
    fs::write(
        store.join("skills/alpha/SKILL.md"),
        "---\nname: alpha\n---\n",
    )
    .unwrap();
    fs::write(store.join("AGENTS.md"), "# rules\n").unwrap();
    fs::write(
        store.join("agents.toml"),
        "version = 1\n[instructions]\nsource = \"AGENTS.md\"\n[skills]\ndir = \"skills\"\n\
         [[hooks]]\nevent = \"SessionStart\"\ncommand = \"echo hi\"\n[targets.claude]\n",
    )
    .unwrap();
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/CLAUDE.md"), "# old rules\n").unwrap();
    let settings = home.join(".claude/settings.json");
    fs::write(&settings, "{\"model\":\"opus\"}").unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();

    let env = Env {
        home: home.clone(),
        vars: Default::default(),
        launcher: "/bin/agents-sync".into(),
    };
    let loaded = app::load(&RealFs, &store.join("agents.toml"), &env).unwrap();
    let (plan, report) = app::sync(&RealFs, &loaded, true, "T").unwrap();
    assert_eq!(report.conflicts, 0, "{:#?}", plan.changes);

    assert_eq!(
        fs::read_link(home.join(".claude/CLAUDE.md")).unwrap(),
        store.join("AGENTS.md")
    );
    assert_eq!(
        fs::read_to_string(store.join("backups/T/.claude/CLAUDE.md")).unwrap(),
        "# old rules\n"
    );
    assert_eq!(
        fs::read_link(home.join(".claude/skills/alpha")).unwrap(),
        store.join("skills/alpha")
    );
    let mode = fs::metadata(&settings).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(fs::read_to_string(&settings).unwrap().contains("echo hi"));
}
