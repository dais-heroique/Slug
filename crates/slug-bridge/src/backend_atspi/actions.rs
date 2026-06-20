//! Action execution against AT-SPI2 objects.
//!
//! Maps the neutral [`crate::action::Action`] onto AT-SPI interface calls:
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

use crate::action::Action;
use crate::error::{BridgeError, Result};

/// Execute an action on an AT-SPI object. `reasoning` is the agent-supplied
/// rationale; it is logged but does not affect behaviour.
#[instrument(skip(conn, objref), fields(action = %action.id(), reasoning = reasoning.unwrap_or("")))]
pub async fn perform(
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
                    slug_ref: super::harvest::obj_ref(objref),
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
        // Synthetic OS input is routed through `synth_input`, not node actions.
        Action::Key(_) | Action::TypeText(_) | Action::MouseClick { .. } | Action::Scroll { .. } => {
            return Err(BridgeError::Unsupported(
                "synthetic input is not yet implemented on Linux".into(),
            ))
        }
    };

    info!(success = outcome, "action executed");
    Ok(outcome)
}
