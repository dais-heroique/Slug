//! Automatic, complete semantic derivation.
//!
//! [`build_frame`] walks the widget tree **once** and, for every widget it
//! visits, produces a draw-command list *and* a semantic node together via
//! [`lower`]. The two are computed from the same `Sem`, so there is no code path
//! that renders a widget without contributing a node — opaque widgets are
//! structurally impossible (the *completeness guarantee*).
//!
//! Each frame yields a full [`accesskit::TreeUpdate`] (the canonical semantics)
//! and a parallel [`BusNode`] tree (the Slug wire form). Push-based: a full tree
//! every frame; callers may diff successive snapshots for deltas.

use accesskit::{Action, Node, NodeId, Rect, Role, Toggled, Tree, TreeId, TreeUpdate};
use slug_core::{derive_ref, SlugRole, SlugState};

use crate::draw::DrawCmd;
use crate::id::WidgetId;
use crate::protocol::BusNode;
use crate::widget::Widget;

const ROW_H: f64 = 24.0;
const WIDTH: f64 = 360.0;
const INDENT: f64 = 16.0;

/// The intrinsic semantics of one widget (bounds filled in by the walk).
struct Sem {
    role: SlugRole,
    ak_role: Role,
    name: Option<String>,
    value: Option<String>,
    numeric: Option<(f64, f64, f64)>,
    toggled: Option<bool>,
    states: Vec<SlugState>,
    actions: Vec<String>,
}

/// One frame: draws, the Slug wire nodes, and the canonical AccessKit tree.
pub struct Frame {
    /// `(ref, draw commands)` for every visited widget — always non-empty.
    pub draws: Vec<(String, Vec<DrawCmd>)>,
    /// One [`BusNode`] per visited widget, parent-linked by ref.
    pub nodes: Vec<BusNode>,
    /// The canonical AccessKit tree update for this frame.
    pub ak: TreeUpdate,
    /// The root node's ref.
    pub root_ref: String,
}

/// Build a complete frame from a widget tree.
pub fn build_frame<Msg: Clone>(app: &str, root: &Widget<Msg>) -> Frame {
    let mut out = Walk { app, draws: Vec::new(), nodes: Vec::new(), ak: Vec::new(), y: 0.0 };
    let (root_ref, root_id) = out.visit(root, "root", 0);
    let ak = TreeUpdate {
        nodes: out.ak,
        tree: Some(Tree::new(root_id)),
        tree_id: TreeId::ROOT,
        focus: root_id,
    };
    Frame { draws: out.draws, nodes: out.nodes, ak, root_ref }
}

/// Verify the completeness guarantee for a built frame: every rendered widget
/// (each `draws` group) maps 1:1 to a semantic node, every group actually paints
/// something, and the AccessKit tree matches node-for-node. Returns `Err` naming
/// the first violation (an "opaque" widget) — which, by construction, can't
/// happen.
pub fn verify_completeness(frame: &Frame) -> Result<(), String> {
    if frame.draws.len() != frame.nodes.len() {
        return Err(format!(
            "{} draw groups but {} nodes",
            frame.draws.len(),
            frame.nodes.len()
        ));
    }
    if frame.ak.nodes.len() != frame.nodes.len() {
        return Err(format!(
            "{} AccessKit nodes but {} wire nodes",
            frame.ak.nodes.len(),
            frame.nodes.len()
        ));
    }
    let node_refs: std::collections::HashSet<&str> =
        frame.nodes.iter().map(|n| n.slug_ref.as_str()).collect();
    for (slug_ref, cmds) in &frame.draws {
        if cmds.is_empty() {
            return Err(format!("widget {slug_ref} rendered nothing"));
        }
        if !node_refs.contains(slug_ref.as_str()) {
            return Err(format!("rendered widget {slug_ref} has no semantic node (opaque!)"));
        }
    }
    Ok(())
}

struct Walk<'a> {
    app: &'a str,
    draws: Vec<(String, Vec<DrawCmd>)>,
    nodes: Vec<BusNode>,
    ak: Vec<(NodeId, Node)>,
    y: f64,
}

impl Walk<'_> {
    /// Visit one widget; returns its `(ref, NodeId)`.
    fn visit<Msg: Clone>(&mut self, w: &Widget<Msg>, path: &str, depth: usize) -> (String, NodeId) {
        let key = w.key().map(str::to_string).unwrap_or_else(|| path.to_string());
        let wid = WidgetId::from_key(&key);
        let slug_ref = derive_ref(&format!("slug-ui:{}:{}", self.app, key));

        let bounds = [INDENT * depth as f64, self.y, WIDTH - INDENT * depth as f64, ROW_H];
        self.y += ROW_H;

        let sem = semantics_of(w);
        let draws = draw_of(w, bounds, &sem);
        self.draws.push((slug_ref.clone(), draws));

        // Recurse into children (containers, list items, menu items) — each is a
        // node too, so everything that paints is represented.
        let mut child_refs: Vec<String> = Vec::new();
        let mut child_ids: Vec<NodeId> = Vec::new();
        match w {
            Widget::Container { children, .. } => {
                for (i, c) in children.iter().enumerate() {
                    let cp = format!("{key}/{i}");
                    let (cr, ci) = self.visit(c, &cp, depth + 1);
                    child_refs.push(cr);
                    child_ids.push(ci);
                }
            }
            Widget::List { items, .. } => {
                for (i, item) in items.iter().enumerate() {
                    let (cr, ci) = self.leaf(
                        &format!("{key}/item/{i}"),
                        depth + 1,
                        SlugRole::ListItem,
                        Role::ListItem,
                        Some(item.text.clone()),
                        None,
                        None,
                        item.selected.then_some(false),
                        {
                            let mut s = vec![SlugState::Enabled, SlugState::Focusable];
                            if item.selected {
                                s.push(SlugState::Selected);
                            }
                            s
                        },
                        vec!["click".into()],
                    );
                    child_refs.push(cr);
                    child_ids.push(ci);
                }
            }
            Widget::Menu { items, .. } => {
                for (i, item) in items.iter().enumerate() {
                    let (cr, ci) = self.leaf(
                        &format!("{key}/item/{i}"),
                        depth + 1,
                        SlugRole::MenuItem,
                        Role::MenuItem,
                        Some(item.text.clone()),
                        None,
                        None,
                        None,
                        vec![SlugState::Enabled, SlugState::Focusable],
                        vec!["click".into()],
                    );
                    child_refs.push(cr);
                    child_ids.push(ci);
                }
            }
            _ => {}
        }

        self.emit(&slug_ref, wid.node_id(), &sem, bounds, &child_refs, &child_ids);
        (slug_ref, wid.node_id())
    }

    /// Emit a leaf node (list/menu item) that has no widget of its own.
    #[allow(clippy::too_many_arguments)]
    fn leaf(
        &mut self,
        path: &str,
        depth: usize,
        role: SlugRole,
        ak_role: Role,
        name: Option<String>,
        value: Option<String>,
        numeric: Option<(f64, f64, f64)>,
        toggled: Option<bool>,
        states: Vec<SlugState>,
        actions: Vec<String>,
    ) -> (String, NodeId) {
        let wid = WidgetId::from_key(path);
        let slug_ref = derive_ref(&format!("slug-ui:{}:{}", self.app, path));
        let bounds = [INDENT * depth as f64, self.y, WIDTH - INDENT * depth as f64, ROW_H];
        self.y += ROW_H;
        let sem = Sem { role, ak_role, name: name.clone(), value, numeric, toggled, states, actions };
        self.draws.push((
            slug_ref.clone(),
            vec![
                DrawCmd::Rect { x: bounds[0], y: bounds[1], w: bounds[2], h: bounds[3], role: "item" },
                DrawCmd::Text { x: bounds[0] + 4.0, y: bounds[1] + 4.0, text: name.unwrap_or_default() },
            ],
        ));
        self.emit(&slug_ref, wid.node_id(), &sem, bounds, &[], &[]);
        (slug_ref, wid.node_id())
    }

    /// Push the `BusNode` + AccessKit node for an already-lowered widget.
    fn emit(
        &mut self,
        slug_ref: &str,
        node_id: NodeId,
        sem: &Sem,
        bounds: [f64; 4],
        child_refs: &[String],
        child_ids: &[NodeId],
    ) {
        self.nodes.push(BusNode {
            slug_ref: slug_ref.to_string(),
            role: sem.role,
            name: sem.name.clone(),
            value: sem.value.clone(),
            states: sem.states.clone(),
            actions: sem.actions.clone(),
            bounds,
            children: child_refs.to_vec(),
        });

        let mut n = Node::new(sem.ak_role);
        if let Some(name) = &sem.name {
            n.set_label(name.clone());
        }
        if let Some(v) = &sem.value {
            n.set_value(v.clone());
        }
        if let Some((val, min, max)) = sem.numeric {
            n.set_numeric_value(val);
            n.set_min_numeric_value(min);
            n.set_max_numeric_value(max);
        }
        if let Some(t) = sem.toggled {
            n.set_toggled(if t { Toggled::True } else { Toggled::False });
        }
        for a in &sem.actions {
            if let Some(ak) = ak_action(a) {
                n.add_action(ak);
            }
        }
        n.set_bounds(Rect { x0: bounds[0], y0: bounds[1], x1: bounds[0] + bounds[2], y1: bounds[1] + bounds[3] });
        if !child_ids.is_empty() {
            n.set_children(child_ids.to_vec());
        }
        self.ak.push((node_id, n));
    }
}

/// The intrinsic semantics for a widget (no bounds; the walk fills those).
fn semantics_of<Msg: Clone>(w: &Widget<Msg>) -> Sem {
    let base = || vec![SlugState::Enabled, SlugState::Showing, SlugState::Visible];
    match w {
        Widget::Container { label, .. } => Sem {
            role: SlugRole::Group,
            ak_role: Role::GenericContainer,
            name: label.clone(),
            value: None,
            numeric: None,
            toggled: None,
            states: base(),
            actions: vec![],
        },
        Widget::Label { text, .. } => Sem {
            role: SlugRole::Label,
            ak_role: Role::Label,
            name: Some(text.clone()),
            value: None,
            numeric: None,
            toggled: None,
            states: base(),
            actions: vec![],
        },
        Widget::Button { text, .. } => Sem {
            role: SlugRole::Button,
            ak_role: Role::Button,
            name: Some(text.clone()),
            value: None,
            numeric: None,
            toggled: None,
            states: {
                let mut s = base();
                s.push(SlugState::Focusable);
                s
            },
            actions: vec!["click".into(), "focus".into()],
        },
        Widget::TextBox { label, value, multiline, .. } => Sem {
            role: if *multiline { SlugRole::EntryMultiline } else { SlugRole::Entry },
            ak_role: Role::TextInput,
            name: Some(label.clone()),
            value: Some(value.clone()),
            numeric: None,
            toggled: None,
            states: {
                let mut s = base();
                s.push(SlugState::Focusable);
                s.push(SlugState::Editable);
                if *multiline {
                    s.push(SlugState::MultiLine);
                }
                s
            },
            actions: vec!["set_text".into(), "focus".into()],
        },
        Widget::Checkbox { label, checked, .. } => Sem {
            role: SlugRole::Checkbox,
            ak_role: Role::CheckBox,
            name: Some(label.clone()),
            value: None,
            numeric: None,
            toggled: Some(*checked),
            states: {
                let mut s = base();
                s.push(SlugState::Focusable);
                if *checked {
                    s.push(SlugState::Checked);
                }
                s
            },
            actions: vec!["toggle".into(), "click".into()],
        },
        Widget::Slider { label, value, min, max, .. } => Sem {
            role: SlugRole::Slider,
            ak_role: Role::Slider,
            name: Some(label.clone()),
            value: Some(fmt_num(*value)),
            numeric: Some((*value, *min, *max)),
            toggled: None,
            states: {
                let mut s = base();
                s.push(SlugState::Focusable);
                s
            },
            actions: vec!["set_value".into(), "increment".into(), "decrement".into()],
        },
        Widget::List { label, .. } => Sem {
            role: SlugRole::List,
            ak_role: Role::List,
            name: Some(label.clone()),
            value: None,
            numeric: None,
            toggled: None,
            states: base(),
            actions: vec![],
        },
        Widget::Menu { label, .. } => Sem {
            role: SlugRole::Menu,
            ak_role: Role::Menu,
            name: Some(label.clone()),
            value: None,
            numeric: None,
            toggled: None,
            states: base(),
            actions: vec![],
        },
    }
}

/// The draw commands for a widget — always at least one (it paints something).
fn draw_of<Msg: Clone>(w: &Widget<Msg>, b: [f64; 4], sem: &Sem) -> Vec<DrawCmd> {
    let kind = match w {
        Widget::Container { .. } => "container",
        Widget::Label { .. } => "label",
        Widget::Button { .. } => "button",
        Widget::TextBox { .. } => "textbox",
        Widget::Checkbox { .. } => "checkbox",
        Widget::Slider { .. } => "slider",
        Widget::List { .. } => "list",
        Widget::Menu { .. } => "menu",
    };
    let mut cmds = vec![DrawCmd::Rect { x: b[0], y: b[1], w: b[2], h: b[3], role: kind }];
    let text = match (&sem.name, &sem.value) {
        (Some(n), Some(v)) => format!("{n}: {v}"),
        (Some(n), None) => n.clone(),
        (None, Some(v)) => v.clone(),
        (None, None) => String::new(),
    };
    if !text.is_empty() {
        cmds.push(DrawCmd::Text { x: b[0] + 4.0, y: b[1] + 4.0, text });
    }
    cmds
}

fn ak_action(a: &str) -> Option<Action> {
    Some(match a {
        "click" | "toggle" | "select" => Action::Click,
        "focus" => Action::Focus,
        "set_text" | "set_value" => Action::SetValue,
        "increment" => Action::Increment,
        "decrement" => Action::Decrement,
        _ => return None,
    })
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
