//! Cross-platform live smoke test.
//!
//! Gated behind the `live-tests` feature so it only runs on a real desktop with
//! the platform accessibility service available (and, on macOS, Accessibility
//! permission granted). It exercises the active backend end-to-end: connect →
//! enumerate apps → snapshot → render the agent-facing YAML.
//!
//! ```text
//! # Linux:   gsettings set org.gnome.desktop.interface toolkit-accessibility true
//! # macOS:   grant Accessibility permission to your terminal first
//! # Windows: no special permission needed
//! cargo test -p slug-bridge --features live-tests --test live_smoke -- --ignored --nocapture
//! ```
#![cfg(feature = "live-tests")]

use slug_bridge::Bridge;
use slug_core::AliasTable;

#[tokio::test]
#[ignore = "requires a live desktop accessibility service"]
async fn connect_enumerate_snapshot() {
    let bridge = Bridge::connect().await.expect("connect to accessibility backend");
    eprintln!("backend: {}", bridge.backend_label());

    let apps = bridge.list_apps().await.expect("list apps");
    eprintln!("apps: {}", apps.len());
    assert!(!apps.is_empty(), "expected at least one accessible application");

    let snap = bridge.snapshot_desktop().await.expect("snapshot desktop");
    assert!(snap.document.len() > 1, "snapshot should contain nodes");

    let mut aliases = AliasTable::new();
    let yaml = snap.document.to_yaml_assigning(&mut aliases);
    println!("--- snapshot ({} nodes) ---\n{}", snap.document.len(), yaml);
    assert!(yaml.contains("[ref="), "snapshot should carry ref aliases");
}
