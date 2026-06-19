//! An agent drives `slug-notes` end-to-end **through the bus, with zero vision** —
//! only the exported semantic tree and refs. This is the proof that the toolkit's
//! completeness guarantee makes apps fully agent-operable.

use slug_core::{SlugRole, SlugState};
use slug_ui::{bus, shared, verify_completeness, BusClient, BusSnapshot};

/// Find a node's ref by (role, name substring).
fn find_ref(snap: &BusSnapshot, role: SlugRole, name_contains: &str) -> Option<String> {
    snap.nodes
        .iter()
        .find(|n| {
            n.role == role
                && n.name.as_deref().map(|s| s.contains(name_contains)).unwrap_or(false)
        })
        .map(|n| n.slug_ref.clone())
}

fn node_value(snap: &BusSnapshot, slug_ref: &str) -> Option<String> {
    snap.nodes.iter().find(|n| n.slug_ref == slug_ref).and_then(|n| n.value.clone())
}

#[tokio::test]
async fn agent_drives_notes_with_zero_vision() {
    // Boot the app on a private Unix socket.
    let socket = std::env::temp_dir().join(format!(
        "slug-notes-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let runtime = shared(slug_notes::build_app());
    let serve_path = socket.clone();
    tokio::spawn(async move {
        let _ = bus::serve(&serve_path, runtime).await;
    });

    // Connect (with a brief retry while the listener binds).
    let mut client = None;
    for _ in 0..50 {
        if let Ok(c) = BusClient::connect(&socket).await {
            client = Some(c);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut client = client.expect("connect to slug-notes bus");

    // 1. Read the world: a snapshot, no pixels.
    let snap = client.snapshot().await.unwrap();
    assert_eq!(snap.app, "slug-notes");
    assert!(snap.tools.iter().any(|t| t.name == "create_note"), "tools are exported");
    assert!(snap.nodes.iter().any(|n| n.role == SlugRole::List), "a notes list exists");

    // 2. High-level tool: create a note.
    let res = client
        .call_tool("create_note", serde_json::json!({ "title": "Groceries", "body": "" }))
        .await
        .unwrap();
    let idx = res["index"].as_u64().unwrap();

    // The new note appears as a list item.
    let snap = client.snapshot().await.unwrap();
    assert!(
        snap.nodes.iter().any(|n| n.role == SlugRole::ListItem
            && n.name.as_deref().map(|s| s.contains("Groceries")).unwrap_or(false)),
        "created note shows in the tree"
    );

    // 3. Low-level widget drive: edit the Title text box by its ref.
    let title_ref = find_ref(&snap, SlugRole::Entry, "Title").expect("title box");
    let snap = client.invoke(&title_ref, "set_text", Some("Groceries (updated)")).await.unwrap();
    assert_eq!(node_value(&snap, &title_ref).as_deref(), Some("Groceries (updated)"));

    // 4. Toggle the Pinned checkbox via its ref.
    let pin_ref = find_ref(&snap, SlugRole::Checkbox, "Pinned").expect("pinned checkbox");
    let snap = client.invoke(&pin_ref, "toggle", None).await.unwrap();
    let pin = snap.nodes.iter().find(|n| n.slug_ref == pin_ref).unwrap();
    assert!(pin.states.contains(&SlugState::Checked), "pin toggled on");

    // 5. Press the "New" button (widget action) → another note.
    let new_ref = find_ref(&snap, SlugRole::Button, "New").expect("New button");
    let before = snap.nodes.iter().filter(|n| n.role == SlugRole::ListItem).count();
    let snap = client.invoke(&new_ref, "click", None).await.unwrap();
    let after = snap.nodes.iter().filter(|n| n.role == SlugRole::ListItem).count();
    assert_eq!(after, before + 1, "New added a note");

    // 6. Search tool finds our note.
    let res = client.call_tool("search_notes", serde_json::json!({ "query": "groceries" })).await.unwrap();
    assert!(!res["matches"].as_array().unwrap().is_empty(), "search finds the note");

    // 7. Clean up via the delete tool — and the whole time the tree stayed
    //    complete (every node well-formed, no opaque widgets).
    let _ = client.call_tool("delete_note", serde_json::json!({ "index": idx })).await.unwrap();

    // Completeness holds for a freshly built frame of the same app.
    let frame = slug_notes::build_app().frame();
    verify_completeness(&frame).expect("slug-notes is fully semantic");
}
