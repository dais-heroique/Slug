//! Capability token — SEMANTIC-SCHEMA §5.4 (STUBBED for milestone 1).
//!
//! Per §5.4 the initial snapshot is gated behind a session-daemon capability
//! check: the agent must present a valid capability token. Security is milestone
//! 5 (see RISK-REGISTER.md), so at milestone 1 this is a deliberate stub:
//! [`CapabilityToken::check`] always succeeds. The type exists so call sites are
//! already wired through the gate and only the validation body changes in M5.

use std::fmt;

use serde::{Deserialize, Serialize};

/// An opaque capability token presented by an agent to receive a snapshot/deltas.
///
/// STUB: at milestone 1 any token (including the empty/anonymous one) is accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityToken(pub String);

impl CapabilityToken {
    /// The anonymous token used while the security layer is unimplemented.
    pub fn anonymous() -> Self {
        CapabilityToken(String::new())
    }

    /// Validate the token for snapshot/delta access (§5.4).
    ///
    /// STUB (M1): always `Ok(())`. M5 will verify a signed, scoped token here.
    pub fn check(&self) -> Result<(), CapabilityError> {
        // TODO(M5): real verification — signature, scope, expiry, session binding.
        Ok(())
    }
}

/// Error returned by a failed capability check. Unreachable while stubbed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CapabilityError {
    Rejected(String),
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityError::Rejected(why) => write!(f, "capability token rejected: {why}"),
        }
    }
}

impl std::error::Error for CapabilityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_token_is_accepted_at_m1() {
        assert!(CapabilityToken::anonymous().check().is_ok());
    }
}
