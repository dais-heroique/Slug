//! # slug-ui
//!
//! A retained-mode, declarative **"UI-as-data"** GUI toolkit whose semantic
//! accessibility tree is derived **automatically and completely** from the same
//! widget tree that renders pixels — making *opaque widgets impossible*.
//!
//! ## The completeness guarantee
//!
//! [`semantics::build_frame`] walks the widget tree once and lowers **every**
//! widget to a draw-command list *and* a semantic node in the same pass, both
//! computed from one intrinsic description. There is no API that paints without
//! emitting a node, so for every rendered widget there is exactly one semantic
//! node. [`verify_completeness`] checks this for any [`Frame`] and is asserted in
//! the tests.
//!
//! ## Pieces
//!
//! - [`widget`]: the retained widget data model (Button, Label, TextBox, Checkbox,
//!   Slider, List, Menu, Container) + a builder API.
//! - [`semantics`]: per-frame derivation to a full [`accesskit`] tree + the Slug
//!   wire tree ([`protocol::BusNode`]).
//! - [`tools`]: high-level imperative tools (WebMCP-style) registered per window.
//! - [`app`]: the `state → view → update` runtime + the [`app::UiRuntime`] the bus
//!   drives.
//! - [`bus`]: export over a local Unix socket (`invoke` / `call_tool`), the native
//!   Slug path.
//! - [`draw`]: rendering primitives + a default headless renderer (a wgpu/Skia
//!   backend is the same [`draw::Renderer`] trait).

pub mod app;
pub mod bus;
pub mod declarative;
pub mod draw;
pub mod id;
pub mod protocol;
pub mod semantics;
pub mod tools;
pub mod widget;

pub use app::{App, UiRuntime};
pub use bus::{serve, shared, BusClient, SharedRuntime};
pub use declarative::{DeclMsg, DeclarativeApp};
pub use draw::{DrawCmd, HeadlessRenderer, Renderer};
pub use id::WidgetId;
pub use protocol::{BusNode, BusSnapshot, ToolSpec};
pub use semantics::{build_frame, verify_completeness, Frame};
pub use tools::{SlugTool, ToolRegistry};
pub use widget::{ListItem, MenuItem, Widget};

#[cfg(test)]
mod tests {
    use super::*;
    use slug_core::SlugRole;

    #[derive(Clone)]
    enum Msg {
        Noop,
    }

    fn sample() -> Widget<Msg> {
        Widget::container(vec![
            Widget::label("Title").id("title"),
            Widget::button("Save").id("save").on_press(Msg::Noop),
            Widget::textbox("Body", "hello").id("body").multiline(true),
            Widget::checkbox("Pinned", true).id("pin").on_toggle(Msg::Noop),
            Widget::slider("Zoom", 3.0, 0.0, 10.0).id("zoom"),
            Widget::list(
                "Notes",
                vec![ListItem::new("First").selected(true), ListItem::new("Second")],
            )
            .id("notes"),
        ])
        .id("root")
    }

    #[test]
    fn every_widget_has_a_node() {
        let frame = build_frame("test", &sample());
        // The guarantee holds.
        verify_completeness(&frame).expect("completeness");
        // Container + label + button + textbox + checkbox + slider + list
        // + 2 list items = 9 nodes, 9 draw groups.
        assert_eq!(frame.nodes.len(), 9);
        assert_eq!(frame.draws.len(), frame.nodes.len());
        // The AccessKit tree mirrors the wire tree node-for-node.
        assert_eq!(frame.ak.nodes.len(), frame.nodes.len());
    }

    #[test]
    fn roles_and_states_are_correct() {
        let frame = build_frame("test", &sample());
        let by_role = |r: SlugRole| frame.nodes.iter().filter(|n| n.role == r).count();
        assert_eq!(by_role(SlugRole::Button), 1);
        assert_eq!(by_role(SlugRole::Checkbox), 1);
        assert_eq!(by_role(SlugRole::Slider), 1);
        assert_eq!(by_role(SlugRole::EntryMultiline), 1);
        assert_eq!(by_role(SlugRole::ListItem), 2);

        let checkbox = frame.nodes.iter().find(|n| n.role == SlugRole::Checkbox).unwrap();
        assert!(checkbox.states.contains(&slug_core::SlugState::Checked));
        assert!(checkbox.actions.contains(&"toggle".to_string()));

        let selected = frame
            .nodes
            .iter()
            .find(|n| n.role == SlugRole::ListItem && n.states.contains(&slug_core::SlugState::Selected));
        assert!(selected.is_some(), "first list item should be selected");
    }
}
