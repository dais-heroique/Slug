//! Bringing an application to the foreground — the "give the game keyboard focus"
//! primitive.
//!
//! This exists to solve a real problem when Slug is driven by a client that lives
//! in another window (e.g. Claude Code in a terminal): each separate tool call
//! returns control to that client, and the OS keyboard focus follows the frontmost
//! window — so synthetic keys sent on the *next* call land in the terminal, not the
//! target app. The fix is to (re)activate the target app **in the same daemon-side
//! call** right before sending input, exactly like `osascript … activate` does, so
//! the client never gets the focus back mid-sequence.
//!
//! Dependency-free: shells out to the platform's standard activator (no extra
//! crates), mirroring [`crate::launch`].

use std::process::{Command, Stdio};

use crate::error::{BridgeError, Result};

/// Bring the application named `name` to the foreground (and give it keyboard
/// focus). Best-effort: returns `Ok(())` once the activator is spawned. The app is
/// expected to be running already; on macOS `open -a` will also launch it if not.
pub fn activate(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(BridgeError::InvalidArgs {
            action: "activate".into(),
            detail: "provide an app name to bring to the front".into(),
        });
    }
    let mut cmd = build_command(name);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.status().map_err(|e| BridgeError::Backend(format!("failed to activate '{name}': {e}")))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn build_command(name: &str) -> Command {
    // `open -a <App>` activates a running app (brings it to the front); if it is
    // not running it launches it. This is the reliable, scriptable equivalent of
    // an AppleScript `activate`.
    let mut cmd = Command::new("open");
    cmd.arg("-a").arg(name);
    cmd
}

#[cfg(target_os = "windows")]
fn build_command(name: &str) -> Command {
    // WScript.Shell's AppActivate focuses an existing window by (partial) title or
    // process name — best effort, no launch.
    let script = format!(
        "$ErrorActionPreference='SilentlyContinue';(New-Object -ComObject WScript.Shell).AppActivate('{}')|Out-Null",
        name.replace('\'', "''")
    );
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
    cmd
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn build_command(name: &str) -> Command {
    // Linux: wmctrl raises a window by (partial) title if available.
    let mut cmd = Command::new("wmctrl");
    cmd.args(["-a", name]);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_name_is_an_error() {
        assert!(activate("").is_err());
        assert!(activate("   ").is_err());
    }
}
