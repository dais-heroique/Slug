//! Action execution against AT-SPI2 objects.
//!
//! Maps Slug action verbs onto AT-SPI interface calls:
//!
//! | Slug action          | AT-SPI call                                 |
//! |----------------------|---------------------------------------------|
//! | `activate`/`click`/`press` | `Action.DoAction(0)`                  |
//! | `toggle`/named action | `Action.DoAction(<matching index>)`        |
//! | `set_text`           | `EditableText.SetTextContents`              |
//! | `set_value`          | `Value.SetCurrentValue`                      |
//! | `focus`              | `Component.GrabFocus`                         |
//!
//! Every invocation is logged with a `reasoning` slot (the agent's stated reason
//! for the action) per the structured-logging requirement.

use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::ObjectRefOwned;
use tracing::{info, instrument};

use crate::error::{BridgeError, Result};

/// A parsed action request.
#[derive(Clone, Debug)]
pub enum Action {
    /// Click/press/activate via `Action.DoAction(0)`.
    Activate,
    /// Invoke a named AT-SPI action (matched case-insensitively).
    Named(String),
    /// `EditableText.SetTextContents`.
    SetText(String),
    /// `Value.SetCurrentValue`.
    SetValue(f64),
    /// `Component.GrabFocus`.
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

/// Execute an action on an AT-SPI object. `reasoning` is the agent-supplied
/// rationale; it is logged but does not affect behaviour.
#[instrument(skip(conn, objref), fields(action = %action.id(), reasoning = reasoning.unwrap_or("")))]
pub async fn invoke(
    conn: &zbus::Connection,
    objref: &ObjectRefOwned,
    action: &Action,
    reasoning: Option<&str>,
) -> Result<bool> {
    let proxy = objref.as_accessible_proxy(conn).await?;
    let proxies = proxy.proxies().await?;

    let outcome = match action {
        Action::Activate => {
            let action_iface =
                proxies.action().await.map_err(|_| BridgeError::InterfaceMissing("Action"))?;
            action_iface.do_action(0).await?
        }
        Action::Named(name) => {
            let action_iface =
                proxies.action().await.map_err(|_| BridgeError::InterfaceMissing("Action"))?;
            let actions = action_iface.get_actions().await?;
            let idx = actions
                .iter()
                .position(|a| a.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| BridgeError::ActionUnavailable {
                    slug_ref: crate::harvest::obj_ref(objref),
                    action: name.clone(),
                })?;
            action_iface.do_action(idx as i32).await?
        }
        Action::SetText(text) => {
            let editable = proxies
                .editable_text()
                .await
                .map_err(|_| BridgeError::InterfaceMissing("EditableText"))?;
            editable.set_text_contents(text).await?
        }
        Action::SetValue(v) => {
            let value =
                proxies.value().await.map_err(|_| BridgeError::InterfaceMissing("Value"))?;
            value.set_current_value(*v).await?;
            true
        }
        Action::Focus => {
            let component = proxies
                .component()
                .await
                .map_err(|_| BridgeError::InterfaceMissing("Component"))?;
            component.grab_focus().await?
        }
    };

    info!(success = outcome, "action executed");
    Ok(outcome)
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
