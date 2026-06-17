//! Tree harvesting: walk AT-SPI2 application trees into a [`SlugDocument`].
//!
//! The harvester walks each application via `AccessibleProxy` (`GetChildren`,
//! `GetRole`, `GetState`, `GetAttributes`, plus the `Component`, `Action` and
//! `Value` interfaces) and builds a [`SlugDocument`] whose node refs are the
//! step-1 derived ULIDs (`{bus_name}:{path}`). It simultaneously records a
//! ref → [`ObjectRefOwned`] index so the action layer can re-acquire any node.

use std::collections::{HashMap, VecDeque};

use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{CoordType, ObjectRefOwned};
use slug_core::{
    derive_ref_from_atspi, Bounds, SlugAction, SlugDocument, SlugNode, SlugRole,
};
use tracing::{debug, trace, warn};

use crate::error::Result;
use crate::mapping::{map_role, map_states, refine_role};

/// Safety limits so a pathological tree can never hang the harvester.
const MAX_DEPTH: usize = 60;
const MAX_NODES: usize = 20_000;

/// The product of a harvest: the materialised document plus the index needed to
/// act on nodes and the per-app coverage reports.
pub struct Harvest {
    pub document: SlugDocument,
    /// ref → AT-SPI object handle, for action execution and live updates.
    pub index: HashMap<String, ObjectRefOwned>,
    /// Per-application coverage assessment (opaque-tree detection).
    pub coverage: Vec<crate::coverage::Coverage>,
}

/// Walk a set of application root object-refs into a single document.
pub async fn harvest_apps(
    conn: &zbus::Connection,
    apps: &[ObjectRefOwned],
) -> Result<Harvest> {
    let mut doc = SlugDocument::new();
    let mut index: HashMap<String, ObjectRefOwned> = HashMap::new();
    let mut coverage = Vec::new();

    for app in apps {
        if app.is_null() {
            continue;
        }
        match harvest_one_app(conn, app, &mut doc, &mut index).await {
            Ok(cov) => coverage.push(cov),
            Err(e) => warn!(error = %e, "failed to harvest application; skipping"),
        }
    }

    doc.recompute_roots();
    Ok(Harvest { document: doc, index, coverage })
}

/// Harvest a single application subtree (iterative DFS, bounded).
async fn harvest_one_app(
    conn: &zbus::Connection,
    app: &ObjectRefOwned,
    doc: &mut SlugDocument,
    index: &mut HashMap<String, ObjectRefOwned>,
) -> Result<crate::coverage::Coverage> {
    let app_proxy = app.as_accessible_proxy(conn).await?;
    let app_id = app_proxy.name().await.unwrap_or_default();
    let app_ref = obj_ref(app);
    debug!(app = %app_id, app_ref = %app_ref, "harvesting application");

    let mut node_count = 0usize;
    let mut max_depth = 0usize;

    // Stack entries: (object, parent_ref, window_id, depth).
    let mut stack: Vec<(ObjectRefOwned, Option<String>, String, usize)> =
        vec![(app.clone(), None, String::new(), 0)];

    while let Some((objref, parent_ref, window_id, depth)) = stack.pop() {
        if node_count >= MAX_NODES || depth > MAX_DEPTH {
            warn!(app = %app_id, "harvest limit hit; truncating subtree");
            break;
        }
        if objref.is_null() {
            continue;
        }

        let proxy = match objref.as_accessible_proxy(conn).await {
            Ok(p) => p,
            Err(e) => {
                trace!(error = %e, "skipping unreachable node");
                continue;
            }
        };

        let (node, children, window_id_for_children) =
            match build_node(&proxy, &objref, &parent_ref, &app_id, &window_id).await {
                Ok(v) => v,
                Err(e) => {
                    trace!(error = %e, "skipping node that failed to read");
                    continue;
                }
            };

        let this_ref = node.slug_ref.clone();
        index.insert(this_ref.clone(), objref.clone());
        doc.insert(node);
        node_count += 1;
        max_depth = max_depth.max(depth);

        for child in children.into_iter().rev() {
            stack.push((child, Some(this_ref.clone()), window_id_for_children.clone(), depth + 1));
        }
    }

    Ok(crate::coverage::assess(&app_id, &app_ref, node_count, max_depth))
}

/// Read a single node's fields and its child object-refs.
async fn build_node(
    proxy: &AccessibleProxy<'_>,
    objref: &ObjectRefOwned,
    parent_ref: &Option<String>,
    app_id: &str,
    window_id: &str,
) -> Result<(SlugNode, Vec<ObjectRefOwned>, String)> {
    let slug_ref = obj_ref(objref);

    let atspi_role = proxy.get_role().await?;
    let states = map_states(proxy.get_state().await?);
    let attributes = proxy.get_attributes().await.unwrap_or_default();

    let base_role = map_role(atspi_role);
    let role = refine_role(base_role, &states, &attributes);

    let mut node = SlugNode::new(&slug_ref, role);
    node.parent_ref = parent_ref.clone();
    node.states = states;
    node.app_id = app_id.to_string();

    // A Window/Frame node starts (or replaces) the window context for its subtree.
    let window_id_for_children = if matches!(role, SlugRole::Window | SlugRole::Dialog) {
        slug_ref.clone()
    } else {
        window_id.to_string()
    };
    node.window_id =
        if window_id.is_empty() { window_id_for_children.clone() } else { window_id.to_string() };

    // Labels.
    let name = proxy.name().await.unwrap_or_default();
    if !name.is_empty() {
        node.name = Some(name);
    }
    if let Ok(desc) = proxy.description().await {
        if !desc.is_empty() {
            node.description = Some(desc);
        }
    }

    // Heading level from attributes (e.g. "level" => 1..6).
    if role == SlugRole::Heading {
        if let Some(level) = attributes.get("level").and_then(|l| l.parse::<u8>().ok()) {
            node.heading_level = Some(level.clamp(1, 6));
        }
    }

    // Toolkit metadata → extensions (Slug core ignores these; §2.1).
    if !attributes.is_empty() {
        node.extensions = Some(attributes.into_iter().collect());
    }

    // Interface-derived data: geometry, actions, value.
    if let Ok(proxies) = proxy.proxies().await {
        if let Ok(component) = proxies.component().await {
            if let Ok((x, y, w, h)) = component.get_extents(CoordType::Screen).await {
                node.bounds =
                    Some(Bounds { x: x as f64, y: y as f64, width: w as f64, height: h as f64 });
            }
        }
        if let Ok(action) = proxies.action().await {
            if let Ok(actions) = action.get_actions().await {
                node.actions = actions
                    .into_iter()
                    .map(|a| SlugAction {
                        id: normalize_action_id(&a.name),
                        label: if a.description.is_empty() { a.name } else { a.description },
                    })
                    .collect();
            }
        }
        if let Ok(value) = proxies.value().await {
            if let Ok(cur) = value.current_value().await {
                node.value = Some(format_number(cur));
                node.value_min = value.minimum_value().await.ok();
                node.value_max = value.maximum_value().await.ok();
                node.value_step = value.minimum_increment().await.ok();
            }
        }
    }

    // Children.
    let children = proxy.get_children().await.unwrap_or_default();
    node.child_refs = children.iter().filter(|c| !c.is_null()).map(obj_ref).collect();

    Ok((node, children, window_id_for_children))
}

/// Read a single node (no app/window context) — used by the live event path to
/// materialise a freshly-created widget for a `NodeCreated` event.
pub async fn read_node(
    conn: &zbus::Connection,
    objref: &ObjectRefOwned,
    parent_ref: Option<String>,
) -> Result<SlugNode> {
    let proxy = objref.as_accessible_proxy(conn).await?;
    let (node, _children, _win) = build_node(&proxy, objref, &parent_ref, "", "").await?;
    Ok(node)
}

/// Derive the stable ref for an object reference (`{bus_name}:{path}`).
pub(crate) fn obj_ref(objref: &ObjectRefOwned) -> String {
    let bus = objref.name_as_str().unwrap_or("");
    let path = objref.path_as_str();
    derive_ref_from_atspi(bus, path)
}

/// Normalise an AT-SPI action name into a stable Slug action id.
///
/// AT-SPI actions are commonly named `click`, `press`, `activate`, `toggle`,
/// `SetFocus`, etc. We lowercase and map the most common synonyms onto the
/// `activate` family the agent expects (see [`crate::actions`]).
fn normalize_action_id(name: &str) -> String {
    match name.trim().to_ascii_lowercase().as_str() {
        "click" | "press" | "activate" | "do default" | "jump" => "activate".to_string(),
        "toggle" => "toggle".to_string(),
        "expand or contract" | "expand" => "expand".to_string(),
        other => other.replace(' ', "_"),
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Convenience: harvest from the registry's desktop root (all applications).
pub async fn harvest_desktop(conn_handle: &atspi::AccessibilityConnection) -> Result<Harvest> {
    let root = conn_handle.root_accessible_on_registry().await?;
    let apps = root.get_children().await?;
    let mut queue: VecDeque<ObjectRefOwned> = VecDeque::new();
    for a in apps {
        queue.push_back(a);
    }
    let apps: Vec<ObjectRefOwned> = queue.into_iter().collect();
    harvest_apps(conn_handle.connection(), &apps).await
}
