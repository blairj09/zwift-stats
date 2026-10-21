//! Runs `zwift-hub serve` in the background as a macOS LaunchAgent: it starts at login, restarts
//! if it exits, and keeps the database current while the explorer stays at 127.0.0.1.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const LABEL: &str = "com.github.blairj09.zwift-hub";

fn plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home folder")?;
    Ok(home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

fn domain() -> Result<String> {
    let out = Command::new("id").arg("-u").output()?;
    Ok(format!("gui/{}", String::from_utf8_lossy(&out.stdout).trim()))
}

fn ensure_macos() -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("the background service uses launchd and is only available on macOS; run `zwift-hub serve` instead");
    }
    Ok(())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn plist(args: &[String], log: &Path) -> String {
    let args: String = args.iter().map(|a| format!("\n        <string>{}</string>", xml_escape(a))).collect();
    let log = xml_escape(&log.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>{args}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>30</integer>
    <key>ProcessType</key>
    <string>Background</string>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    )
}

/// `serve_args` are the arguments after the executable, e.g. `["serve", "--port", "7878"]`.
pub fn install(serve_args: Vec<String>, data_dir: &Path) -> Result<()> {
    ensure_macos()?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let mut args = vec![exe.display().to_string()];
    args.extend(serve_args);
    fs::create_dir_all(data_dir)?;
    let log = data_dir.join("zwift-hub.log");

    let path = plist_path()?;
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, plist(&args, &log))?;

    let domain = domain()?;
    // Replace a running copy so a reinstall picks up a new binary or new options
    let _ = Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]).output();
    let out = Command::new("launchctl").args(["bootstrap", &domain]).arg(&path).output()?;
    if !out.status.success() {
        bail!("launchctl bootstrap failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    println!("Installed {}", path.display());
    println!("Zwift Stats now runs in the background and starts at login. Log: {}", log.display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    ensure_macos()?;
    let domain = domain()?;
    let _ = Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]).output();
    let path = plist_path()?;
    if path.exists() {
        fs::remove_file(&path)?;
        println!("Removed {}", path.display());
    } else {
        println!("The background service was not installed");
    }
    Ok(())
}

pub fn status() -> Result<()> {
    ensure_macos()?;
    let out = Command::new("launchctl").args(["print", &format!("{}/{LABEL}", domain()?)]).output()?;
    if !out.status.success() {
        println!("Not running (install it with `zwift-hub service install`)");
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines().map(str::trim) {
        if line.starts_with("state =") || line.starts_with("pid =") || line.starts_with("last exit code =") {
            println!("{line}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_escapes_arguments() {
        let p = plist(&["/a & b/zwift-hub".into(), "serve".into()], Path::new("/tmp/x.log"));
        assert!(p.contains("<string>/a &amp; b/zwift-hub</string>"));
        assert!(p.contains("<string>serve</string>"));
        assert!(p.contains(LABEL));
    }
}
