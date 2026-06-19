//! High-level imperative "tools" (WebMCP `navigator.modelContext`-style).
//!
//! Beyond raw widgets, a window can register semantic *tools*: named, described,
//! JSON-schema-typed imperative actions an agent can call directly (e.g.
//! `create_note`, `search_notes`). They are exported alongside the widget tree so
//! the agent can choose between low-level widget invocation and high-level intent.

use serde_json::Value;

use crate::protocol::ToolSpec;

/// A tool handler: mutates app state from JSON args, returns a JSON result.
pub type ToolHandler<S> = Box<dyn FnMut(&mut S, Value) -> Result<Value, String> + Send>;

/// A registered tool: its public [`ToolSpec`] plus its handler.
pub struct SlugTool<S> {
    pub spec: ToolSpec,
    pub handler: ToolHandler<S>,
}

impl<S> SlugTool<S> {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        params_schema: Value,
        handler: impl FnMut(&mut S, Value) -> Result<Value, String> + Send + 'static,
    ) -> Self {
        SlugTool {
            spec: ToolSpec {
                name: name.into(),
                description: description.into(),
                params_schema,
            },
            handler: Box::new(handler),
        }
    }
}

/// A window's tool registry.
#[derive(Default)]
pub struct ToolRegistry<S> {
    tools: Vec<SlugTool<S>>,
}

impl<S> ToolRegistry<S> {
    pub fn new() -> Self {
        ToolRegistry { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: SlugTool<S>) {
        self.tools.push(tool);
    }

    /// The public specs (for export on the bus).
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.iter().map(|t| t.spec.clone()).collect()
    }

    /// Invoke a tool by name.
    pub fn call(&mut self, state: &mut S, name: &str, args: Value) -> Result<Value, String> {
        let tool = self
            .tools
            .iter_mut()
            .find(|t| t.spec.name == name)
            .ok_or_else(|| format!("unknown tool: {name}"))?;
        (tool.handler)(state, args)
    }
}
