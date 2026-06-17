//! Error type for the bridge.

use thiserror::Error;

/// Errors surfaced by `slug-bridge`.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Could not connect to / talk to the AT-SPI2 accessibility bus.
    #[error("AT-SPI connection error: {0}")]
    Atspi(#[from] atspi::AtspiError),

    /// A raw zbus/D-Bus error.
    #[error("D-Bus error: {0}")]
    Zbus(#[from] zbus::Error),

    /// The agent referenced a ref that is not in the current index.
    #[error("unknown ref: {0}")]
    UnknownRef(String),

    /// The requested action is not available on the target node.
    #[error("action '{action}' not available on ref {slug_ref}")]
    ActionUnavailable { slug_ref: String, action: String },

    /// The action's arguments were invalid (e.g. set_value with no number).
    #[error("invalid arguments for action '{action}': {detail}")]
    InvalidArgs { action: String, detail: String },

    /// The target interface (Action/Value/EditableText/Component) is missing.
    #[error("interface '{0}' not available on target")]
    InterfaceMissing(&'static str),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, BridgeError>;
