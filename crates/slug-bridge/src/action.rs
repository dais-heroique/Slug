//! Platform-neutral action commands.
//!
//! [`Action`] is the verb the MCP/CLI layer asks the bridge to perform on a node.
//! It is parsed once here and then executed by whichever platform backend is
//! active (AT-SPI `DoAction`/`SetTextContents`, UIA `InvokePattern`/`ValuePattern`,
//! or AX `AXUIElementPerformAction`/`AXUIElementSetAttributeValue`).

use crate::error::{BridgeError, Result};

/// A parsed action request, independent of any accessibility backend.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Click / press / activate the node (UIA `Invoke`, AT-SPI `DoAction(0)`,
    /// AX `AXPress`).
    Activate,
    /// Invoke a backend-named action (matched case-insensitively): `toggle`,
    /// `expand`, `collapse`, `select`, …
    Named(String),
    /// Set the node's text value.
    SetText(String),
    /// Set the node's numeric value.
    SetValue(f64),
    /// Move focus to the node.
    Focus,
}

impl Action {
    /// Parse an action verb + optional argument string from the MCP/CLI layer.
    pub fn parse(verb: &str, arg: Option<&str>) -> Result<Action> {
        let v = verb.trim().to_ascii_lowercase();
        match v.as_str() {
            "activate" | "click" | "press" | "invoke" | "do_action" => Ok(Action::Activate),
            "focus" | "grab_focus" => Ok(Action::Focus),
            "set_text" | "type" | "fill" => Ok(Action::SetText(arg.unwrap_or("").to_string())),
            "set_value" => {
                let n = arg
                    .and_then(|a| a.trim().parse::<f64>().ok())
                    .ok_or_else(|| BridgeError::InvalidArgs {
                        action: "set_value".into(),
                        detail: "expected a numeric argument".into(),
                    })?;
                Ok(Action::SetValue(n))
            }
            "toggle" | "expand" | "collapse" | "check" | "uncheck" | "select" => {
                Ok(Action::Named(v))
            }
            other => Ok(Action::Named(other.to_string())),
        }
    }

    /// Stable id used in logs and the `ActionCompleted` event.
    pub fn id(&self) -> String {
        match self {
            Action::Activate => "activate".into(),
            Action::Named(n) => n.clone(),
            Action::SetText(_) => "set_text".into(),
            Action::SetValue(_) => "set_value".into(),
            Action::Focus => "focus".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verbs() {
        assert!(matches!(Action::parse("click", None).unwrap(), Action::Activate));
        assert!(matches!(Action::parse("focus", None).unwrap(), Action::Focus));
        assert!(matches!(Action::parse("set_text", Some("hi")).unwrap(), Action::SetText(_)));
        assert!(matches!(Action::parse("set_value", Some("0.5")).unwrap(), Action::SetValue(_)));
        assert!(Action::parse("set_value", Some("xyz")).is_err());
        assert!(matches!(Action::parse("toggle", None).unwrap(), Action::Named(_)));
    }
}
