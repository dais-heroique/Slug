//! Platform-neutral error type for the bridge.
//!
//! Backend-specific source errors (AT-SPI/zbus on Linux) are `#[from]`-converted
//! behind `cfg`; Windows/macOS backends surface their `HRESULT`/`AXError` through
//! the neutral [`BridgeError::Backend`] / [`BridgeError::PermissionDenied`]
//! variants.

use thiserror::Error;

/// Errors surfaced by `slug-bridge`.
#[derive(Debug, Error)]
pub enum BridgeError {
    /// Could not connect to the platform accessibility service.
    #[error("could not connect to the accessibility backend: {0}")]
    Connect(String),

    /// A backend (UIA/AX/AT-SPI) operation failed.
    #[error("accessibility backend error: {0}")]
    Backend(String),

    /// Accessibility permission has not been granted (notably macOS TCC).
    #[error("accessibility permission denied: {0}")]
    PermissionDenied(String),

    /// This platform has no accessibility backend compiled in.
    #[error("no accessibility backend available on this platform: {0}")]
    Unsupported(String),

    /// The agent referenced a ref that is not in the current index.
    #[error("unknown ref: {0}")]
    UnknownRef(String),

    /// The requested action is not available on the target node.
    #[error("action '{action}' not available on ref {slug_ref}")]
    ActionUnavailable { slug_ref: String, action: String },

    /// The action's arguments were invalid (e.g. set_value with no number).
    #[error("invalid arguments for action '{action}': {detail}")]
    InvalidArgs { action: String, detail: String },

    /// The target interface (Action/Value/EditableText/Component, or a UIA
    /// pattern / AX attribute) is missing on the node.
    #[error("interface '{0}' not available on target")]
    InterfaceMissing(&'static str),

    /// AT-SPI2 connection / protocol error (Linux only).
    #[cfg(target_os = "linux")]
    #[error("AT-SPI connection error: {0}")]
    Atspi(#[from] atspi::AtspiError),

    /// Raw zbus/D-Bus error (Linux only).
    #[cfg(target_os = "linux")]
    #[error("D-Bus error: {0}")]
    Zbus(#[from] zbus::Error),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, BridgeError>;
