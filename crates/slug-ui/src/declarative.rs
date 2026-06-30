//! A declarative, language-agnostic runtime: build a `slug-ui` app from a JSON
//! spec, with no Rust generics or callbacks. This is what the C and Python SDKs
//! wrap so non-Rust apps get the same completeness guarantee.
//!
//! Interactivity is data: interactive widgets bind to named state fields
//! (`field`) that auto-update on action, and buttons/menu items `emit` named
//! events the host can drain. State is a JSON object.
//!
//! ```json
//! {
//!   "app": "demo",
//!   "root": {
//!     "type": "container", "id": "root", "children": [
//!       { "type": "label", "id": "h", "text": "Hello" },
//!       { "type": "textbox", "id": "name", "label": "Name", "field": "name" },
//!       { "type": "checkbox", "id": "ok", "label": "Agree", "field": "agree" },
//!       { "type": "button", "id": "go", "text": "Submit", "emit": "submit" }
//!     ]
//!   }
//! }
//! ```

use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::app::{App, UiRuntime};
use crate::protocol::BusSnapshot;
use crate::widget::{ListItem, MenuItem, Widget};

/// A message in the declarative runtime.
#[derive(Clone)]
pub enum DeclMsg {
    /// Set `field` to a value.
    SetText(String, String),
    SetBool(String, bool),
    SetNum(String, f64),
    /// Emit a named event for the host to observe.
    Emit(String),
}

/// An app built from a JSON spec.
pub struct DeclarativeApp {
    app: App<Value, DeclMsg>,
    events: Arc<Mutex<Vec<String>>>,
}

impl DeclarativeApp {
    /// Build from a JSON spec. Initial state may seed field values.
    pub fn from_spec(spec: Value, initial_state: Value) -> Result<DeclarativeApp, String> {
        let name = spec.get("app").and_then(Value::as_str).unwrap_or("slug-ui").to_string();
        let root = spec.get("root").cloned().ok_or("spec is missing `root`")?;
        let spec_root = Arc::new(root);
        let events = Arc::new(Mutex::new(Vec::new()));

        let state = match initial_state {
            Value::Object(_) => initial_state,
            Value::Null => Value::Object(Map::new()),
            _ => return Err("initial_state must be a JSON object".into()),
        };

        let view_root = spec_root.clone();
        let view = move |state: &Value| build_node(&view_root, state);

        let ev = events.clone();
        let update = move |state: &mut Value, msg: DeclMsg| {
            let obj = match state {
                Value::Object(m) => m,
                _ => return,
            };
            match msg {
                DeclMsg::SetText(f, v) => {
                    obj.insert(f, Value::String(v));
                }
                DeclMsg::SetBool(f, v) => {
                    obj.insert(f, Value::Bool(v));
                }
                DeclMsg::SetNum(f, v) => {
                    obj.insert(f, serde_json::json!(v));
                }
                DeclMsg::Emit(name) => ev.lock().expect("events mutex").push(name),
            }
        };

        Ok(DeclarativeApp {
            app: App::new(name, state, view, update),
            events,
        })
    }

    /// A complete semantic snapshot.
    pub fn snapshot(&self) -> BusSnapshot {
        self.app.snapshot()
    }

    /// Perform an action on a node by ref.
    pub fn invoke(&mut self, slug_ref: &str, action: &str, args: Option<&str>) -> Result<(), String> {
        self.app.invoke(slug_ref, action, args)
    }

    /// Drain the queued emitted events (button presses, menu selections).
    pub fn drain_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events.lock().expect("events mutex"))
    }

    /// The current state as JSON.
    pub fn state(&self) -> &Value {
        self.app.state()
    }

    /// The current frame (draws + AccessKit tree + wire nodes) — useful to assert
    /// the completeness guarantee on a declarative app.
    pub fn frame(&self) -> crate::Frame {
        self.app.frame()
    }

    /// Consume into the bus runtime so it can be served on a socket.
    pub fn into_runtime(self) -> App<Value, DeclMsg> {
        self.app
    }
}

fn field_str(state: &Value, field: &str) -> String {
    state.get(field).and_then(Value::as_str).unwrap_or("").to_string()
}
fn field_bool(state: &Value, field: &str) -> bool {
    state.get(field).and_then(Value::as_bool).unwrap_or(false)
}
fn field_num(state: &Value, field: &str) -> f64 {
    state.get(field).and_then(Value::as_f64).unwrap_or(0.0)
}

fn build_node(node: &Value, state: &Value) -> Widget<DeclMsg> {
    let id = node.get("id").and_then(Value::as_str);
    let ty = node.get("type").and_then(Value::as_str).unwrap_or("label");
    let text = || node.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let label = || node.get("label").and_then(Value::as_str).unwrap_or("").to_string();
    let field = || node.get("field").and_then(Value::as_str).unwrap_or("").to_string();

    let mut w = match ty {
        "container" => {
            let children = node
                .get("children")
                .and_then(Value::as_array)
                .map(|cs| cs.iter().map(|c| build_node(c, state)).collect())
                .unwrap_or_default();
            Widget::container(children)
        }
        "label" => Widget::label(text()),
        "button" => {
            let emit = node.get("emit").and_then(Value::as_str).unwrap_or("press").to_string();
            Widget::button(text()).on_press(DeclMsg::Emit(emit))
        }
        "textbox" => {
            let f = field();
            let multiline = node.get("multiline").and_then(Value::as_bool).unwrap_or(false);
            let fc = f.clone();
            Widget::textbox(label(), field_str(state, &f))
                .multiline(multiline)
                .on_input(move |v| DeclMsg::SetText(fc.clone(), v))
        }
        "checkbox" => {
            let f = field();
            let cur = field_bool(state, &f);
            let fc = f.clone();
            Widget::checkbox(label(), cur).on_toggle(DeclMsg::SetBool(fc, !cur))
        }
        "slider" => {
            let f = field();
            let min = node.get("min").and_then(Value::as_f64).unwrap_or(0.0);
            let max = node.get("max").and_then(Value::as_f64).unwrap_or(100.0);
            let fc = f.clone();
            Widget::slider(label(), field_num(state, &f), min, max)
                .on_change(move |v| DeclMsg::SetNum(fc.clone(), v))
        }
        "list" => {
            let items = node
                .get("items")
                .and_then(Value::as_array)
                .map(|its| {
                    its.iter()
                        .map(|it| {
                            let t = it.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                            let mut li = ListItem::new(t);
                            if let Some(e) = it.get("emit").and_then(Value::as_str) {
                                li = li.on_select(DeclMsg::Emit(e.to_string()));
                            }
                            li
                        })
                        .collect()
                })
                .unwrap_or_default();
            Widget::list(label(), items)
        }
        "menu" => {
            let items = node
                .get("items")
                .and_then(Value::as_array)
                .map(|its| {
                    its.iter()
                        .map(|it| {
                            let t = it.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                            let mut mi = MenuItem::new(t);
                            if let Some(e) = it.get("emit").and_then(Value::as_str) {
                                mi = mi.on_select(DeclMsg::Emit(e.to_string()));
                            }
                            mi
                        })
                        .collect()
                })
                .unwrap_or_default();
            Widget::menu(label(), items)
        }
        _ => Widget::label(text()),
    };
    if let Some(id) = id {
        w = w.id(id);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify_completeness;

    #[test]
    fn builds_and_drives_from_json() {
        let spec = serde_json::json!({
            "app": "decl-demo",
            "root": { "type": "container", "id": "root", "children": [
                { "type": "textbox", "id": "name", "label": "Name", "field": "name" },
                { "type": "checkbox", "id": "agree", "label": "Agree", "field": "agree" },
                { "type": "button", "id": "go", "text": "Submit", "emit": "submit" }
            ]}
        });
        let mut app = DeclarativeApp::from_spec(spec, Value::Null).unwrap();
        let snap = app.snapshot();
        verify_completeness(&app.frame()).unwrap();

        let name_ref = snap
            .nodes
            .iter()
            .find(|n| n.role == slug_core::SlugRole::Entry)
            .unwrap()
            .slug_ref
            .clone();
        app.invoke(&name_ref, "set_text", Some("Ada")).unwrap();
        assert_eq!(app.state().get("name").unwrap(), "Ada");

        let go_ref = snap
            .nodes
            .iter()
            .find(|n| n.role == slug_core::SlugRole::Button)
            .unwrap()
            .slug_ref
            .clone();
        app.invoke(&go_ref, "click", None).unwrap();
        assert_eq!(app.drain_events(), vec!["submit".to_string()]);
    }
}
