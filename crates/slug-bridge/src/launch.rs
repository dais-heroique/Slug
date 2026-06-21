//! Launching applications by name or URI — the "open Spotify" primitive.
//!
//! Slug otherwise drives apps that are already running; this lets the agent start
//! one first. It shells out to the platform's standard launcher (no extra deps),
//! and can also open a URI / deep link (e.g. `spotify:playlist:…`) so flows like
//! "open Spotify and play my playlist" work end to end.

use std::process::{Command, Stdio};

use crate::error::{BridgeError, Result};

/// Launch an application by `name` (e.g. `Spotify`), optionally opening `uri`
/// (a deep link or file path) with it. Returns once the launcher is spawned.
pub fn launch(name: &str, uri: Option<&str>) -> Result<()> {
    let name = name.trim();
    let uri = uri.map(str::trim).filter(|s| !s.is_empty());
    if name.is_empty() && uri.is_none() {
        return Err(BridgeError::InvalidArgs {
            action: "launch".into(),
            detail: "provide an app name or a uri".into(),
        });
    }
    let mut cmd = build_command(name, uri);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map_err(|e| BridgeError::Backend(format!("failed to launch '{name}': {e}")))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn build_command(name: &str, uri: Option<&str>) -> Command {
    // `open -a <App> [uri]` launches by display name; `open <uri>` uses the URI's
    // registered handler when no app name is given.
    let mut cmd = Command::new("open");
    if !name.is_empty() {
        cmd.arg("-a").arg(name);
    }
    if let Some(u) = uri {
        cmd.arg(u);
    }
    cmd
}

#[cfg(target_os = "windows")]
fn build_command(name: &str, uri: Option<&str>) -> Command {
    // `cmd /C start "" <target>` launches an app by name (if resolvable) or opens
    // a URI via its protocol handler.
    let target = uri.unwrap_or(name);
    let mut cmd = Command::new("cmd");
    cmd.args(["/C", "start", "", target]);
    cmd
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn build_command(name: &str, uri: Option<&str>) -> Command {
    // Linux: open a URI with the desktop handler, else launch the .desktop app by
    // name (gtk-launch), else fall back to running the binary directly.
    if let Some(u) = uri {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(u);
        cmd
    } else if which("gtk-launch") {
        let mut cmd = Command::new("gtk-launch");
        cmd.arg(name);
        cmd
    } else {
        Command::new(name.to_ascii_lowercase())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_request_is_an_error() {
        assert!(launch("", None).is_err());
        assert!(launch("   ", Some("")).is_err());
    }

    // On non-macOS/Windows the name path runs the binary directly, so a harmless
    // always-present command exercises the spawn logic end to end.
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn launches_a_harmless_binary() {
        assert!(launch("true", None).is_ok());
    }
}
