//! # slug-mcp
//!
//! The Slug MCP server: exposes the AT-SPI2 semantic bus (via `slug-bridge`) as
//! Model Context Protocol tools over stdio and streamable HTTP.
//!
//! Tools: `slug_snapshot`, `slug_invoke`, `slug_wait_for`, `slug_list_apps`.
//!
//! This is the session-daemon layer where step-1 rule #1 holds: the agent only
//! ever sees short ref aliases (`b1`, `e5`) — ULIDs never cross this boundary.

pub mod agent;
pub mod approval;
pub mod mcp;
pub mod server;
pub mod session;

pub use agent::AgentController;
pub use session::{Scope, Session, SessionError, SnapshotFilter};
