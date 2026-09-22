use std::collections::BTreeMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use anyhow::{bail, ensure, Context, Result};
use clap::{Parser, Subcommand};

use agents_sync::adapters::real_fs::RealFs;
use agents_sync::adapters::secrets::SystemSecrets;
use agents_sync::app::plan::{Change, Plan, Replacing};
use agents_sync::app::watch::{plist, WatchSpec};
use agents_sync::app::{self, exec_mcp, Env, Loaded};
use agents_sync::domain::paths::display;
use agents_sync::domain::target::Target;
use agents_sync::ports::FileSystem;

const DEFAULT_MANIFEST: &str = ".agents/agents.toml";

#[derive(Parser)]
#[command(
    name = "agents-sync",
    version,
    about = "One manifest for every coding agent"
)]
struct Cli {
    /// Manifest path (default: ~/.agents/agents.toml).
    #[arg(long, global = true, env = "AGENTS_SYNC_MANIFEST")]
    manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show what `sync` would change, without touching anything.
    Plan {
        /// Plan replacing files that differ from the shared version.
        #[arg(long)]
        adopt: bool,
        /// Print a unified diff for every file write.
        #[arg(long)]
        diff: bool,
    },
    /// Apply the manifest to every enabled tool (backups first).
    Sync {
        /// Replace files that differ from the shared version (after backup).
        #[arg(long)]
        adopt: bool,
    },
    /// Exit 1 when a tool has drifted from the manifest.
    Check {
        /// Session-start hook mode: one line on drift, always exit 0.
        #[arg(long)]
        hook: bool,
    },
    /// Start an MCP server with its secrets resolved (used by generated configs).
    ExecMcp { name: String },
    /// Rerun `sync` automatically when a shared file changes (launchd).
    Watch {
        #[command(subcommand)]
        action: WatchCmd,
    },
}

#[derive(Subcommand)]
enum WatchCmd {
    /// Print the launchd agent.
    Print,
    /// Install and load the launchd agent.
    Install,
    /// Unload and remove the launchd agent.
    Uninstall,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let home = PathBuf::from(std::env::var("HOME").context("HOME is not set")?);
    let manifest = cli.manifest.unwrap_or_else(|| home.join(DEFAULT_MANIFEST));
    let vars: BTreeMap<String, String> = Target::ALL
        .into_iter()
        .filter_map(Target::home_env_var)
        .filter_map(|key| std::env::var(key).ok().map(|v| (key.to_string(), v)))
        .collect();
    let launcher = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("cannot locate the agents-sync executable")?;
    let env = Env {
        home: home.clone(),
        vars,
        launcher,
    };
    let fs = RealFs;

    match cli.command {
        Cmd::Plan { adopt, diff } => {
            let loaded = app::load(&fs, &manifest, &env)?;
            print_plan(&app::compute_plan(&fs, &loaded, adopt)?, &home, diff);
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Sync { adopt } => {
            let loaded = app::load(&fs, &manifest, &env)?;
            let stamp = humantime::format_rfc3339_seconds(SystemTime::now())
                .to_string()
                .replace(':', "-");
            let (plan, report) = app::sync(&fs, &loaded, adopt, &stamp)?;
            print_plan(&plan, &home, false);
            if report.backed_up {
                println!(
                    "Backups: {}",
                    display(&loaded.backups_dir.join(&stamp), &home)
                );
            }
            Ok(if report.conflicts > 0 {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::Check { hook } => check(&fs, &manifest, &env, hook),
        Cmd::ExecMcp { name } => {
            let loaded = app::load(&fs, &manifest, &env)?;
            let server = loaded
                .manifest
                .mcp
                .get(&name)
                .with_context(|| format!("no MCP server `{name}` in the manifest"))?;
            let command = server
                .command
                .as_ref()
                .with_context(|| format!("`{name}` is a URL server; nothing to execute"))?;
            let vars = exec_mcp::resolve_env(server, &SystemSecrets)?;
            let error = Command::new(command).args(&server.args).envs(vars).exec();
            Err(error).with_context(|| format!("cannot start `{command}`"))
        }
        Cmd::Watch { action } => {
            let loaded = app::load(&fs, &manifest, &env)?;
            watch(&fs, &loaded, &home, action)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn check(fs: &RealFs, manifest: &Path, env: &Env, hook: bool) -> Result<ExitCode> {
    let result = app::load(fs, manifest, env).and_then(|l| app::compute_plan(fs, &l, false));
    match (result, hook) {
        (Ok(plan), _) if plan.changes.is_empty() => {
            if !hook {
                println!("Everything is in sync.");
            }
            Ok(ExitCode::SUCCESS)
        }
        (Ok(plan), true) => {
            println!(
                "agents-sync: {} pending change(s) in the shared agent config; run `agents-sync plan`.",
                plan.changes.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        (Ok(plan), false) => {
            print_plan(&plan, &env.home, false);
            Ok(ExitCode::from(1))
        }
        (Err(e), true) => {
            println!("agents-sync: {e:#}");
            Ok(ExitCode::SUCCESS)
        }
        (Err(e), false) => Err(e),
    }
}

fn watch(fs: &RealFs, loaded: &Loaded, home: &Path, action: WatchCmd) -> Result<()> {
    let config = &loaded.manifest.watch;
    let spec = WatchSpec {
        label: config.label.clone(),
        exe: loaded.layout.launcher.clone(),
        manifest: loaded.layout.manifest_path.clone(),
        watch_paths: app::watch_paths(loaded),
        log: loaded.log_path.clone(),
        throttle_seconds: config.throttle_seconds,
    };
    let agent = home
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", spec.label));
    let uid = String::from_utf8(Command::new("id").arg("-u").output()?.stdout)?
        .trim()
        .to_string();
    let domain = format!("gui/{uid}");
    let service = format!("{domain}/{}", spec.label);
    match action {
        WatchCmd::Print => print!("{}", plist(&spec)),
        WatchCmd::Install => {
            fs.write(&agent, &plist(&spec))?;
            // Reload cleanly if an older version is already loaded.
            let _ = Command::new("launchctl")
                .args(["bootout", &service])
                .status();
            let status = Command::new("launchctl")
                .args(["bootstrap", &domain])
                .arg(&agent)
                .status()?;
            ensure!(status.success(), "launchctl bootstrap failed");
            println!(
                "Watching {} path(s); log: {}",
                spec.watch_paths.len(),
                display(&spec.log, home)
            );
        }
        WatchCmd::Uninstall => {
            let _ = Command::new("launchctl")
                .args(["bootout", &service])
                .status();
            if agent.exists() {
                std::fs::remove_file(&agent)?;
            } else {
                bail!("{} is not installed", display(&agent, home));
            }
        }
    }
    Ok(())
}

fn describe(change: &Change, home: &Path) -> String {
    let d = |p: &Path| display(p, home);
    let body = match change {
        Change::Link {
            path,
            to,
            replacing,
            ..
        } => {
            let note = match replacing {
                Replacing::Nothing => String::new(),
                Replacing::Link(old) => format!("  (was -> {})", d(old)),
                Replacing::Existing { identical: true } => {
                    "  (replaces an identical copy, backed up)".into()
                }
                Replacing::Existing { identical: false } => {
                    "  (replaces DIFFERENT content, backed up)".into()
                }
            };
            format!("link     {} -> {}{note}", d(path), d(to))
        }
        Change::RemoveLink { path, was, .. } => {
            format!("unlink   {}  (stale -> {})", d(path), d(was))
        }
        Change::Write {
            path, before: None, ..
        } => format!("create   {}", d(path)),
        Change::Write {
            path,
            before: Some(_),
            ..
        } => format!("update   {}", d(path)),
        Change::RemoveFile { path, .. } => {
            format!("remove   {}  (stale generated file, backed up)", d(path))
        }
        Change::Conflict { path, reason, .. } => format!("CONFLICT {}: {reason}", d(path)),
    };
    format!("{:<8} {body}", change.target().name())
}

fn print_plan(plan: &Plan, home: &Path, diff: bool) {
    for warning in &plan.warnings {
        eprintln!("warning: {warning}");
    }
    if plan.changes.is_empty() {
        println!("Everything is in sync.");
        return;
    }
    for change in &plan.changes {
        println!("{}", describe(change, home));
        if let (
            true,
            Change::Write {
                path,
                before,
                after,
                ..
            },
        ) = (diff, change)
        {
            let old = before.as_deref().unwrap_or_default();
            let label = display(path, home);
            let text = similar::TextDiff::from_lines(old, after.as_str())
                .unified_diff()
                .context_radius(2)
                .header(&label, &label)
                .to_string();
            print!("{text}");
        }
    }
    let conflicts = plan.changes.iter().filter(|c| c.is_conflict()).count();
    println!(
        "\n{} change(s), {conflicts} conflict(s).",
        plan.changes.len() - conflicts
    );
}
