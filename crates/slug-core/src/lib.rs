//! # slug-core
//!
//! The unified semantic document model for Slug. This crate is a faithful Rust
//! mirror of **Doc 1 — SEMANTIC-SCHEMA** (`docs/SEMANTIC-SCHEMA.md`): the
//! `SlugNode` schema (§2), role/state enumerations (§3), the stable-ref scheme
//! (§4), and the delta/event model (§5).
//!
//! It depends only on `serde` (+ `serde_json` for tests) so it can be embedded
//! anywhere — bridge, MCP server, CLI, or a future Wayland compositor.
//!
//! ## Milestone-1 adaptations
//!
//! Two step-1 deviations from the canonical (compositor-era) spec are documented
//! at their definitions:
//!
//! 1. **Refs** ([`refs`]): the ULID is *derived* from the AT-SPI identity
//!    `{unique_bus_name}:{accessible_path}` rather than minted by a compositor.
//!    Internally everything uses the ULID; agents only ever see short session
//!    [`AliasTable`] aliases (`b1`, `e5`).
//! 2. **Deltas** ([`delta`]): [`SlugDelta`] frames are produced from AT-SPI2
//!    signals, not Wayland frame commits. The wire format is unchanged.
//!
//! The §5.4 capability token is **stubbed** ([`capability`]) — security is M5.

pub mod alias;
pub mod capability;
pub mod delta;
pub mod document;
pub mod node;
pub mod refs;
pub mod role;
pub mod state;
pub mod yaml;

pub use alias::AliasTable;
pub use capability::{CapabilityError, CapabilityToken};
pub use delta::{SlugDelta, SlugEvent, SlugNodePatch, SlugReorder};
pub use document::{Snapshot, SlugDocument};
pub use node::{Bounds, SlugAction, SlugNode, SlugOption, Validation, ValidationState};
pub use refs::{derive_ref, derive_ref_from_atspi};
pub use role::SlugRole;
pub use state::SlugState;
