//! Rendering primitives.
//!
//! The toolkit lowers each widget to draw commands *and* a semantic node in the
//! same pass (see [`crate::semantics`]). Real GPU backends (wgpu / Skia) consume
//! the same [`DrawCmd`] stream a widget produces; the default [`HeadlessRenderer`]
//! just records them, which is all the tests and the completeness guarantee need.

/// A single low-level draw primitive.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    /// A filled/stroked rectangle (background, border, track, thumb, …).
    Rect { x: f64, y: f64, w: f64, h: f64, role: &'static str },
    /// A run of text.
    Text { x: f64, y: f64, text: String },
}

/// Anything that can consume a widget's draw commands.
///
/// A wgpu/Skia backend is just another `Renderer`; the toolkit never calls a
/// renderer without also emitting the widget's semantic node.
pub trait Renderer {
    fn push(&mut self, cmd: DrawCmd);
}

/// The default renderer: records every command. Used by the demo and tests, and
/// proves the completeness guarantee without a GPU.
#[derive(Default)]
pub struct HeadlessRenderer {
    pub cmds: Vec<DrawCmd>,
}

impl Renderer for HeadlessRenderer {
    fn push(&mut self, cmd: DrawCmd) {
        self.cmds.push(cmd);
    }
}
