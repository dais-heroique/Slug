//! `slug-notes` — a notes editor built with `slug-ui`.
//!
//! The UI is pure data (`view`) over a small model, with an `update` reducer and
//! a couple of high-level Slug tools (`create_note`, `search_notes`,
//! `delete_note`). Because it is built on `slug-ui`, every rendered widget has a
//! semantic node automatically — an agent can drive it through the bus with zero
//! vision.

use serde_json::{json, Value};
use slug_ui::{App, ListItem, MenuItem, SlugTool, Widget};

/// A single note.
#[derive(Clone, Debug, Default)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub pinned: bool,
}

/// The application model.
#[derive(Default)]
pub struct Notes {
    pub notes: Vec<Note>,
    pub selected: usize,
}

impl Notes {
    /// A small seeded model for demos/tests.
    pub fn seeded() -> Notes {
        Notes {
            notes: vec![Note {
                title: "Welcome".into(),
                body: "This note is driven entirely through the semantic bus.".into(),
                pinned: true,
            }],
            selected: 0,
        }
    }

    fn current(&self) -> Option<&Note> {
        self.notes.get(self.selected)
    }
}

/// Messages produced by widgets and applied by [`update`].
#[derive(Clone, Debug)]
pub enum Msg {
    Select(usize),
    NewNote,
    DeleteSelected,
    SetTitle(String),
    SetBody(String),
    TogglePinned,
}

/// The reducer: apply a message to the model.
pub fn update(state: &mut Notes, msg: Msg) {
    match msg {
        Msg::Select(i) => {
            if i < state.notes.len() {
                state.selected = i;
            }
        }
        Msg::NewNote => {
            state.notes.push(Note { title: "Untitled".into(), ..Default::default() });
            state.selected = state.notes.len() - 1;
        }
        Msg::DeleteSelected => {
            if state.selected < state.notes.len() {
                state.notes.remove(state.selected);
                state.selected = state.selected.saturating_sub(1);
            }
        }
        Msg::SetTitle(t) => {
            if let Some(n) = state.notes.get_mut(state.selected) {
                n.title = t;
            }
        }
        Msg::SetBody(b) => {
            if let Some(n) = state.notes.get_mut(state.selected) {
                n.body = b;
            }
        }
        Msg::TogglePinned => {
            if let Some(n) = state.notes.get_mut(state.selected) {
                n.pinned = !n.pinned;
            }
        }
    }
}

/// The view: derive the widget tree from the model.
pub fn view(state: &Notes) -> Widget<Msg> {
    let (title, body, pinned) = state
        .current()
        .map(|n| (n.title.clone(), n.body.clone(), n.pinned))
        .unwrap_or_default();

    let items: Vec<ListItem<Msg>> = state
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let label = if n.title.is_empty() { "(untitled)".to_string() } else { n.title.clone() };
            let label = if n.pinned { format!("📌 {label}") } else { label };
            ListItem::new(label).selected(i == state.selected).on_select(Msg::Select(i))
        })
        .collect();

    let editor = Widget::container(vec![
        Widget::textbox("Title", title).id("title").on_input(Msg::SetTitle),
        Widget::textbox("Body", body).id("body").multiline(true).on_input(Msg::SetBody),
        Widget::checkbox("Pinned", pinned).id("pinned").on_toggle(Msg::TogglePinned),
    ])
    .id("editor");

    Widget::container(vec![
        Widget::label("Slug Notes").id("header"),
        Widget::menu(
            "File",
            vec![
                MenuItem::new("New").on_select(Msg::NewNote),
                MenuItem::new("Delete").on_select(Msg::DeleteSelected),
            ],
        )
        .id("menu"),
        Widget::list("Notes", items).id("notes"),
        editor,
        Widget::container(vec![
            Widget::button("New").id("new").on_press(Msg::NewNote),
            Widget::button("Delete").id("delete").on_press(Msg::DeleteSelected),
        ])
        .id("actions"),
    ])
    .id("root")
}

/// Build the app with its widgets *and* high-level tools.
pub fn build_app() -> App<Notes, Msg> {
    build_app_with(Notes::seeded())
}

/// Build the app over a given model (used by tests).
pub fn build_app_with(state: Notes) -> App<Notes, Msg> {
    App::new("slug-notes", state, view, update)
        .with_tool(SlugTool::new(
            "create_note",
            "Create a new note and select it. Returns its index.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body": { "type": "string" }
                },
                "required": ["title"]
            }),
            |state: &mut Notes, args: Value| {
                let title = args.get("title").and_then(Value::as_str).unwrap_or("").to_string();
                if title.is_empty() {
                    return Err("title is required".into());
                }
                let body = args.get("body").and_then(Value::as_str).unwrap_or("").to_string();
                state.notes.push(Note { title, body, pinned: false });
                state.selected = state.notes.len() - 1;
                Ok(json!({ "index": state.selected }))
            },
        ))
        .with_tool(SlugTool::new(
            "search_notes",
            "Search note titles/bodies for a query. Returns matching indices.",
            json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
            |state: &mut Notes, args: Value| {
                let q = args.get("query").and_then(Value::as_str).unwrap_or("").to_lowercase();
                let hits: Vec<Value> = state
                    .notes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        n.title.to_lowercase().contains(&q) || n.body.to_lowercase().contains(&q)
                    })
                    .map(|(i, n)| json!({ "index": i, "title": n.title }))
                    .collect();
                Ok(json!({ "matches": hits }))
            },
        ))
        .with_tool(SlugTool::new(
            "delete_note",
            "Delete the note at the given index.",
            json!({
                "type": "object",
                "properties": { "index": { "type": "integer" } },
                "required": ["index"]
            }),
            |state: &mut Notes, args: Value| {
                let i = args.get("index").and_then(Value::as_u64).ok_or("index is required")? as usize;
                if i >= state.notes.len() {
                    return Err(format!("no note at index {i}"));
                }
                state.notes.remove(i);
                state.selected = state.selected.min(state.notes.len().saturating_sub(1));
                Ok(json!({ "ok": true, "remaining": state.notes.len() }))
            },
        ))
}
