//! The retained widget tree — UI described as data.
//!
//! Widgets are a plain data enum (no hidden rendering): an interactive widget
//! carries the [`Msg`](crate::app::App) it emits when acted on (Elm-style), so
//! the whole UI is inspectable, diffable data. A widget *cannot* exist without a
//! role the semantics layer can derive — there is no "custom paint" escape hatch.

use std::sync::Arc;

/// Maps a typed text/number input to an application message.
pub type InputFn<Msg> = Arc<dyn Fn(String) -> Msg + Send + Sync>;
/// Maps a slider value to an application message.
pub type ValueFn<Msg> = Arc<dyn Fn(f64) -> Msg + Send + Sync>;

/// An entry in a [`Widget::List`].
#[derive(Clone)]
pub struct ListItem<Msg> {
    pub text: String,
    pub selected: bool,
    pub on_select: Option<Msg>,
}

impl<Msg> ListItem<Msg> {
    pub fn new(text: impl Into<String>) -> Self {
        ListItem { text: text.into(), selected: false, on_select: None }
    }
    pub fn selected(mut self, sel: bool) -> Self {
        self.selected = sel;
        self
    }
    pub fn on_select(mut self, msg: Msg) -> Self {
        self.on_select = Some(msg);
        self
    }
}

/// An entry in a [`Widget::Menu`].
#[derive(Clone)]
pub struct MenuItem<Msg> {
    pub text: String,
    pub on_select: Option<Msg>,
}

impl<Msg> MenuItem<Msg> {
    pub fn new(text: impl Into<String>) -> Self {
        MenuItem { text: text.into(), on_select: None }
    }
    pub fn on_select(mut self, msg: Msg) -> Self {
        self.on_select = Some(msg);
        self
    }
}

/// A retained-mode widget.
#[derive(Clone)]
pub enum Widget<Msg> {
    Container { key: Option<String>, label: Option<String>, children: Vec<Widget<Msg>> },
    Label { key: Option<String>, text: String },
    Button { key: Option<String>, text: String, on_press: Option<Msg> },
    TextBox { key: Option<String>, label: String, value: String, multiline: bool, on_input: Option<InputFn<Msg>> },
    Checkbox { key: Option<String>, label: String, checked: bool, on_toggle: Option<Msg> },
    Slider { key: Option<String>, label: String, value: f64, min: f64, max: f64, on_change: Option<ValueFn<Msg>> },
    List { key: Option<String>, label: String, items: Vec<ListItem<Msg>> },
    Menu { key: Option<String>, label: String, items: Vec<MenuItem<Msg>> },
}

impl<Msg: Clone> Widget<Msg> {
    pub fn container(children: Vec<Widget<Msg>>) -> Self {
        Widget::Container { key: None, label: None, children }
    }
    pub fn label(text: impl Into<String>) -> Self {
        Widget::Label { key: None, text: text.into() }
    }
    pub fn button(text: impl Into<String>) -> Self {
        Widget::Button { key: None, text: text.into(), on_press: None }
    }
    pub fn textbox(label: impl Into<String>, value: impl Into<String>) -> Self {
        Widget::TextBox {
            key: None,
            label: label.into(),
            value: value.into(),
            multiline: false,
            on_input: None,
        }
    }
    pub fn checkbox(label: impl Into<String>, checked: bool) -> Self {
        Widget::Checkbox { key: None, label: label.into(), checked, on_toggle: None }
    }
    pub fn slider(label: impl Into<String>, value: f64, min: f64, max: f64) -> Self {
        Widget::Slider { key: None, label: label.into(), value, min, max, on_change: None }
    }
    pub fn list(label: impl Into<String>, items: Vec<ListItem<Msg>>) -> Self {
        Widget::List { key: None, label: label.into(), items }
    }
    pub fn menu(label: impl Into<String>, items: Vec<MenuItem<Msg>>) -> Self {
        Widget::Menu { key: None, label: label.into(), items }
    }

    /// Set the stable key (the agent-facing ref derives from it).
    pub fn id(mut self, key: impl Into<String>) -> Self {
        let k = Some(key.into());
        match &mut self {
            Widget::Container { key, .. }
            | Widget::Label { key, .. }
            | Widget::Button { key, .. }
            | Widget::TextBox { key, .. }
            | Widget::Checkbox { key, .. }
            | Widget::Slider { key, .. }
            | Widget::List { key, .. }
            | Widget::Menu { key, .. } => *key = k,
        }
        self
    }

    /// Make a [`Widget::TextBox`] multi-line.
    pub fn multiline(mut self, on: bool) -> Self {
        if let Widget::TextBox { multiline, .. } = &mut self {
            *multiline = on;
        }
        self
    }

    /// Button press message.
    pub fn on_press(mut self, msg: Msg) -> Self {
        if let Widget::Button { on_press, .. } = &mut self {
            *on_press = Some(msg);
        }
        self
    }

    /// Text input mapper.
    pub fn on_input(mut self, f: impl Fn(String) -> Msg + Send + Sync + 'static) -> Self {
        if let Widget::TextBox { on_input, .. } = &mut self {
            *on_input = Some(Arc::new(f));
        }
        self
    }

    /// Checkbox toggle message.
    pub fn on_toggle(mut self, msg: Msg) -> Self {
        if let Widget::Checkbox { on_toggle, .. } = &mut self {
            *on_toggle = Some(msg);
        }
        self
    }

    /// Slider change mapper.
    pub fn on_change(mut self, f: impl Fn(f64) -> Msg + Send + Sync + 'static) -> Self {
        if let Widget::Slider { on_change, .. } = &mut self {
            *on_change = Some(Arc::new(f));
        }
        self
    }

    /// The explicit key, if any.
    pub fn key(&self) -> Option<&str> {
        match self {
            Widget::Container { key, .. }
            | Widget::Label { key, .. }
            | Widget::Button { key, .. }
            | Widget::TextBox { key, .. }
            | Widget::Checkbox { key, .. }
            | Widget::Slider { key, .. }
            | Widget::List { key, .. }
            | Widget::Menu { key, .. } => key.as_deref(),
        }
    }
}
