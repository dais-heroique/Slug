//! The application runtime: state + view + update (Elm-style), tools, and the
//! type-erased [`UiRuntime`] the bus drives.
//!
//! The retained widget tree is a pure function of state (`view`); an incoming
//! `invoke(ref, action)` is resolved to a typed `Msg`, fed to `update`, and the
//! tree is rebuilt. The agent never needs pixels: it reads the exported semantic
//! tree and acts on refs.

use serde_json::Value;
use slug_core::derive_ref;

use crate::protocol::BusSnapshot;
use crate::semantics::{build_frame, Frame};
use crate::tools::{SlugTool, ToolRegistry};
use crate::widget::Widget;

/// Derives the widget tree from application state.
type ViewFn<S, Msg> = Box<dyn Fn(&S) -> Widget<Msg> + Send>;
/// Applies a message to application state.
type UpdateFn<S, Msg> = Box<dyn FnMut(&mut S, Msg) + Send>;

/// The object-safe interface the bus server speaks to (monomorphic over the
/// app's `State`/`Msg`).
pub trait UiRuntime: Send {
    fn app_name(&self) -> &str;
    /// A complete semantic snapshot (nodes + tools).
    fn snapshot(&self) -> BusSnapshot;
    /// Perform a widget action addressed by its ref.
    fn invoke(&mut self, slug_ref: &str, action: &str, args: Option<&str>) -> Result<(), String>;
    /// Call a registered high-level tool.
    fn call_tool(&mut self, name: &str, args: Value) -> Result<Value, String>;
}

/// An application built from `state`, a `view`, and an `update` reducer.
pub struct App<S, Msg> {
    name: String,
    state: S,
    view: ViewFn<S, Msg>,
    update: UpdateFn<S, Msg>,
    tools: ToolRegistry<S>,
    tree: Widget<Msg>,
}

impl<S, Msg: Clone> App<S, Msg> {
    /// Create an app. `view` derives the widget tree from state; `update` applies
    /// a message to state.
    pub fn new(
        name: impl Into<String>,
        state: S,
        view: impl Fn(&S) -> Widget<Msg> + Send + 'static,
        update: impl FnMut(&mut S, Msg) + Send + 'static,
    ) -> Self {
        let name = name.into();
        let tree = view(&state);
        App { name, state, view: Box::new(view), update: Box::new(update), tools: ToolRegistry::new(), tree }
    }

    /// Register a high-level tool (builder style).
    pub fn with_tool(mut self, tool: SlugTool<S>) -> Self {
        self.tools.register(tool);
        self
    }

    /// Borrow the application state (for tests/inspection).
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Build the current frame (draws + AccessKit tree + wire nodes).
    pub fn frame(&self) -> Frame {
        build_frame(&self.name, &self.tree)
    }

    fn rebuild(&mut self) {
        self.tree = (self.view)(&self.state);
    }

    fn apply(&mut self, msg: Msg) {
        (self.update)(&mut self.state, msg);
        self.rebuild();
    }

    /// Resolve a ref + action to a message against the current tree.
    fn resolve(&self, target: &str, action: &str, args: Option<&str>) -> ResolveOutcome<Msg> {
        resolve_widget(&self.name, &self.tree, "root", target, action, args)
    }
}

enum ResolveOutcome<Msg> {
    /// Found the node and produced a message to apply.
    Msg(Msg),
    /// Found the node but the action is a no-op (no handler bound).
    NoOp,
    /// No node matched the ref.
    NotFound,
}

impl<S: Send, Msg: Clone + Send> UiRuntime for App<S, Msg> {
    fn app_name(&self) -> &str {
        &self.name
    }

    fn snapshot(&self) -> BusSnapshot {
        let frame = self.frame();
        BusSnapshot {
            app: self.name.clone(),
            root: frame.root_ref,
            nodes: frame.nodes,
            tools: self.tools.specs(),
        }
    }

    fn invoke(&mut self, slug_ref: &str, action: &str, args: Option<&str>) -> Result<(), String> {
        match self.resolve(slug_ref, action, args) {
            ResolveOutcome::Msg(msg) => {
                self.apply(msg);
                Ok(())
            }
            ResolveOutcome::NoOp => Ok(()),
            ResolveOutcome::NotFound => Err(format!("unknown ref: {slug_ref}")),
        }
    }

    fn call_tool(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let out = self.tools.call(&mut self.state, name, args)?;
        self.rebuild();
        Ok(out)
    }
}

/// Recursive resolver mirroring [`build_frame`]'s key/ref derivation.
fn resolve_widget<Msg: Clone>(
    app: &str,
    w: &Widget<Msg>,
    path: &str,
    target: &str,
    action: &str,
    args: Option<&str>,
) -> ResolveOutcome<Msg> {
    let key = w.key().map(str::to_string).unwrap_or_else(|| path.to_string());
    let this_ref = derive_ref(&format!("slug-ui:{app}:{key}"));

    if this_ref == target {
        return widget_msg(w, action, args);
    }

    match w {
        Widget::Container { children, .. } => {
            for (i, c) in children.iter().enumerate() {
                match resolve_widget(app, c, &format!("{key}/{i}"), target, action, args) {
                    ResolveOutcome::NotFound => continue,
                    found => return found,
                }
            }
            ResolveOutcome::NotFound
        }
        Widget::List { items, .. } => {
            for (i, item) in items.iter().enumerate() {
                let item_ref = derive_ref(&format!("slug-ui:{app}:{key}/item/{i}"));
                if item_ref == target {
                    return match &item.on_select {
                        Some(m) => ResolveOutcome::Msg(m.clone()),
                        None => ResolveOutcome::NoOp,
                    };
                }
            }
            ResolveOutcome::NotFound
        }
        Widget::Menu { items, .. } => {
            for (i, item) in items.iter().enumerate() {
                let item_ref = derive_ref(&format!("slug-ui:{app}:{key}/item/{i}"));
                if item_ref == target {
                    return match &item.on_select {
                        Some(m) => ResolveOutcome::Msg(m.clone()),
                        None => ResolveOutcome::NoOp,
                    };
                }
            }
            ResolveOutcome::NotFound
        }
        _ => ResolveOutcome::NotFound,
    }
}

/// Translate an (action, args) pair on a specific widget to its message.
fn widget_msg<Msg: Clone>(w: &Widget<Msg>, action: &str, args: Option<&str>) -> ResolveOutcome<Msg> {
    let opt = match w {
        Widget::Button { on_press, .. } if matches!(action, "click" | "activate" | "press") => {
            on_press.clone()
        }
        Widget::Checkbox { on_toggle, .. } if matches!(action, "toggle" | "click") => {
            on_toggle.clone()
        }
        Widget::TextBox { on_input, .. } if matches!(action, "set_text" | "fill" | "type") => {
            on_input.as_ref().map(|f| f(args.unwrap_or("").to_string()))
        }
        Widget::Slider { on_change, value, min, max, .. } => match action {
            "set_value" => args
                .and_then(|a| a.trim().parse::<f64>().ok())
                .and_then(|v| on_change.as_ref().map(|f| f(v.clamp(*min, *max)))),
            "increment" => on_change.as_ref().map(|f| f((value + 1.0).min(*max))),
            "decrement" => on_change.as_ref().map(|f| f((value - 1.0).max(*min))),
            _ => None,
        },
        _ => None,
    };
    match opt {
        Some(msg) => ResolveOutcome::Msg(msg),
        None => ResolveOutcome::NoOp,
    }
}
