//! # slug-brain
//!
//! A hybrid agentic loop that drives the Slug MCP tools, switching between a local
//! Ollama model and the Anthropic Claude API based on detected hardware.
//!
//! - [`hardware`]: probe VRAM/RAM/CPU → a [`hardware::CapabilityTier`] and a
//!   "Can I run it?" report.
//! - [`backend`]: the [`backend::LlmBackend`] trait with `ClaudeBackend` and
//!   `OllamaBackend` impls, driven by identical tool schemas.
//! - [`brain`]: the observe→reason→act→verify loop.
//! - [`safety`]: per-session token/cost caps, destructive-action confirmation,
//!   and an action log with undo.
//! - [`config`]: `slug.toml` parsing.
//! - [`tools`]: bridges the loop to the `slug-mcp` tools.

pub mod backend;
pub mod brain;
pub mod config;
pub mod hardware;
pub mod safety;
pub mod tools;

pub use backend::LlmBackend;
pub use brain::{Brain, Outcome};
pub use config::Config;
pub use hardware::{assess, CapabilityTier, Report, SystemProbe};
