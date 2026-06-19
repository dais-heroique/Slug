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
    /// Synthetic key chord sent to the OS-focused app (e.g. `cmd+s`, `return`,
    /// `tab`). Works on **any** app — including opaque ones with no accessibility
    /// tree — because it injects an OS input event, not a node action. No pixels,
    /// no screenshot.
    Key(String),
    /// Synthetic literal text typed into the OS-focused app (unicode). Same
    /// "works on any app" property as [`Action::Key`].
    TypeText(String),
    /// Synthetic left mouse click at absolute screen coordinates. Lets the agent
    /// click anywhere — including inside opaque apps — when it has a position
    /// (e.g. a node's `bounds`). No pixels are captured.
    MouseClick { x: f64, y: f64 },
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
            "key" | "hotkey" | "keystroke" | "press_key" => {
                Ok(Action::Key(arg.unwrap_or("").to_string()))
            }
            "type_text" | "synth_type" => Ok(Action::TypeText(arg.unwrap_or("").to_string())),
            "click_at" | "mouse_click" => {
                let (x, y) = parse_xy(arg).ok_or_else(|| BridgeError::InvalidArgs {
                    action: "click_at".into(),
                    detail: "expected coordinates as 'x,y'".into(),
                })?;
                Ok(Action::MouseClick { x, y })
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
            Action::Key(_) => "key".into(),
            Action::TypeText(_) => "type_text".into(),
            Action::MouseClick { .. } => "click_at".into(),
        }
    }

    /// Whether this action is synthetic OS input (targets the focused app/screen,
    /// not a specific node) — routed to [`crate::AccessibilityBackend::synth_input`].
    pub fn is_synthetic(&self) -> bool {
        matches!(self, Action::Key(_) | Action::TypeText(_) | Action::MouseClick { .. })
    }
}

/// Parse an `"x,y"` (or `"x y"`) coordinate string.
fn parse_xy(arg: Option<&str>) -> Option<(f64, f64)> {
    let s = arg?.trim();
    let (a, b) = s.split_once(',').or_else(|| s.split_once(char::is_whitespace))?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
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

    #[test]
    fn parses_synthetic_input() {
        let k = Action::parse("key", Some("cmd+s")).unwrap();
        assert!(matches!(&k, Action::Key(s) if s == "cmd+s"));
        assert!(k.is_synthetic());
        let t = Action::parse("type_text", Some("hello")).unwrap();
        assert!(matches!(&t, Action::TypeText(s) if s == "hello"));
        assert!(t.is_synthetic());
        // Regular actions are not synthetic.
        assert!(!Action::parse("click", None).unwrap().is_synthetic());
    }

    #[test]
    fn parses_mouse_click_coordinates() {
        let m = Action::parse("click_at", Some("100,250")).unwrap();
        assert!(matches!(m, Action::MouseClick { x, y } if x == 100.0 && y == 250.0));
        assert!(m.is_synthetic());
        // Space-separated also works.
        assert!(matches!(Action::parse("click_at", Some("12 34")).unwrap(), Action::MouseClick { .. }));
        // Bad coords error.
        assert!(Action::parse("click_at", Some("nope")).is_err());
    }
}
