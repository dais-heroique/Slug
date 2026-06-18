//! End-to-end test against a real GTK4 app (gnome-text-editor) over a live
//! AT-SPI2 bus.
//!
//! Gated behind the `live-tests` feature (and Linux) so default `cargo test` and
//! the CI matrix never build/run it without a desktop. Run on a real session:
//! `cargo test -p slug-bridge --features live-tests --test e2e_gnome_text_editor -- --ignored --nocapture`
#![cfg(all(target_os = "linux", feature = "live-tests"))]
//!
//! This test is `#[ignore]`d by default because it needs a running graphical
//! session with the accessibility bus enabled and `gnome-text-editor` installed.
//! Run it explicitly on such a machine:
//!
//! ```text
//! # ensure a11y is on: gsettings set org.gnome.desktop.interface toolkit-accessibility true
//! cargo test -p slug-bridge --test e2e_gnome_text_editor -- --ignored --nocapture
//! ```
//!
//! It launches the editor, harvests its tree, asserts the tree is non-trivial and
//! not flagged opaque, finds an interactive control, and focuses it.

use std::process::{Child, Command};
use std::time::Duration;

use slug_bridge::Bridge;
use slug_core::{AliasTable, SlugRole};

/// Spawn gnome-text-editor and return the child so the test can kill it.
fn launch_editor() -> Option<Child> {
    Command::new("gnome-text-editor").arg("--new-window").spawn().ok()
}

#[tokio::test]
#[ignore = "requires a graphical session with AT-SPI + gnome-text-editor"]
async fn harvest_and_drive_gnome_text_editor() {
    // Bring the app up and give it a moment to register on the a11y bus.
    let mut child = launch_editor().expect("failed to launch gnome-text-editor");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let result = run_assertions().await;

    // Always clean up the editor process.
    let _ = child.kill();
    let _ = child.wait();

    result.expect("e2e assertions failed");
}

async fn run_assertions() -> anyhow::Result<()> {
    let bridge = Bridge::connect().await?;

    // The editor must be visible in the app list.
    let apps = bridge.list_apps().await?;
    assert!(
        apps.iter().any(|a| a.app_id.to_lowercase().contains("text editor")
            || a.app_id.to_lowercase().contains("gnome-text-editor")),
        "gnome-text-editor not found among apps: {:?}",
        apps.iter().map(|a| &a.app_id).collect::<Vec<_>>()
    );

    // Harvest the whole desktop and locate the editor's window subtree.
    let snap = bridge.snapshot_desktop().await?;
    assert!(snap.document.len() > 10, "editor tree suspiciously small");

    // The editor itself must not be flagged opaque.
    assert!(
        !snap.opaque.iter().any(|c| c.app_id.to_lowercase().contains("text editor")),
        "editor was flagged opaque: {:?}",
        snap.opaque
    );

    // Render the YAML the agent would see (aliases only).
    let mut aliases = AliasTable::new();
    let yaml = snap.document.to_yaml_assigning(&mut aliases);
    println!("--- snapshot ---\n{yaml}");
    assert!(yaml.contains("[ref="), "snapshot should carry ref aliases");

    // Find an interactive node (a button) and focus it.
    let button = snap
        .document
        .iter()
        .find(|n| n.role == SlugRole::Button)
        .expect("expected at least one button in the editor UI");
    let ok = bridge.invoke(&button.slug_ref, "focus", None, Some("e2e: focus a button")).await?;
    println!("focus button {:?} -> {ok}", button.name);

    Ok(())
}
