use std::path::PathBuf;

/// A launchd agent that reruns `sync` whenever a shared path changes.
pub struct WatchSpec {
    pub label: String,
    pub exe: PathBuf,
    pub manifest: PathBuf,
    pub watch_paths: Vec<PathBuf>,
    pub log: PathBuf,
    pub throttle_seconds: u32,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn string(value: impl AsRef<str>) -> String {
    format!("<string>{}</string>", escape(value.as_ref()))
}

pub fn plist(spec: &WatchSpec) -> String {
    let path = |p: &PathBuf| string(p.display().to_string());
    let program = [
        path(&spec.exe),
        string("sync"),
        string("--manifest"),
        path(&spec.manifest),
    ]
    .join("\n      ");
    let watched = spec
        .watch_paths
        .iter()
        .map(path)
        .collect::<Vec<_>>()
        .join("\n      ");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    {label}
    <key>ProgramArguments</key>
    <array>
      {program}
    </array>
    <key>WatchPaths</key>
    <array>
      {watched}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>{throttle}</integer>
    <key>StandardOutPath</key>
    {log}
    <key>StandardErrorPath</key>
    {log}
  </dict>
</plist>
"#,
        label = string(&spec.label),
        throttle = spec.throttle_seconds,
        log = path(&spec.log),
    )
}
