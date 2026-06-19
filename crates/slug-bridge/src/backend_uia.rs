//! Windows accessibility backend: UI Automation (`IUIAutomation`).
//!
//! Compiled only on Windows. Enumerates top-level windows as applications, walks
//! each with the Control view `IUIAutomationTreeWalker`, maps UIA `ControlType` →
//! [`SlugRole`] and UIA properties (`IsEnabled`, `IsOffscreen`, …) → [`SlugState`],
//! and performs actions through UIA control patterns. The node's native identity
//! (brief §4) is the stringified UIA `RuntimeId`.
//!
//! Live events (`AddAutomationEventHandler` / `AddFocusChangedEventHandler` / …)
//! require COM event-sink objects; that wiring is a documented follow-up
//! ([`UiaBackend::subscribe_events`]). Snapshot + invoke — the semantic-first core
//! — are fully implemented.

#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::sync::Mutex;

use slug_core::{derive_ref, Bounds, SlugNode, SlugRole, SlugState};
use tracing::{info, warn};
use windows::core::BSTR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
};
use windows::Win32::UI::Accessibility::*;

use crate::action::Action;
use crate::backend::{
    AccessibilityBackend, AppHandle, BackendNodeId, BoxFuture, EventSink, Subscription,
};
use crate::coverage::{self, Coverage};
use crate::error::{BridgeError, Result};

const MAX_DEPTH: usize = 60;
const MAX_NODES: usize = 20_000;

/// The UI Automation backend.
pub struct UiaBackend {
    automation: IUIAutomation,
    /// derived ref → UIA element, captured during each snapshot.
    handles: Mutex<HashMap<String, IUIAutomationElement>>,
    /// app backend_node_id → top-level window element.
    app_handles: Mutex<HashMap<String, IUIAutomationElement>>,
    /// app backend_node_id → coverage from the most recent snapshot.
    coverage: Mutex<HashMap<String, Coverage>>,
}

// `IUIAutomation` and `IUIAutomationElement` are `Send + Sync` in windows-rs.
unsafe impl Send for UiaBackend {}
unsafe impl Sync for UiaBackend {}

impl UiaBackend {
    /// Create the UI Automation client. No special permissions are required on
    /// Windows.
    pub fn new() -> Result<Self> {
        unsafe {
            // MTA so the client can be driven from worker threads. A prior STA
            // init in this thread is fine — ignore the "changed mode" result.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL)
                .map_err(|e| BridgeError::Connect(format!("CoCreateInstance(CUIAutomation): {e}")))?;
            info!("UI Automation client created");
            Ok(UiaBackend {
                automation,
                handles: Mutex::new(HashMap::new()),
                app_handles: Mutex::new(HashMap::new()),
                coverage: Mutex::new(HashMap::new()),
            })
        }
    }

    fn find_window(&self, backend_node_id: &str) -> Result<IUIAutomationElement> {
        if let Some(el) = self.app_handles.lock().expect("mutex").get(backend_node_id).cloned() {
            return Ok(el);
        }
        // Re-enumerate to locate the window by its stable runtime id.
        for app in self.enumerate_windows()? {
            if app.0 == backend_node_id {
                self.app_handles.lock().expect("mutex").insert(app.0.clone(), app.1.clone());
                return Ok(app.1);
            }
        }
        Err(BridgeError::UnknownRef(backend_node_id.to_string()))
    }

    /// Enumerate top-level windows as `(backend_node_id, element)`.
    fn enumerate_windows(&self) -> Result<Vec<(String, IUIAutomationElement)>> {
        unsafe {
            let root = self
                .automation
                .GetRootElement()
                .map_err(|e| BridgeError::Backend(format!("GetRootElement: {e}")))?;
            let walker = self
                .automation
                .ControlViewWalker()
                .map_err(|e| BridgeError::Backend(format!("ControlViewWalker: {e}")))?;
            let mut out = Vec::new();
            let mut child = walker.GetFirstChildElement(&root).ok();
            while let Some(win) = child {
                let id = native_id(&win);
                out.push((id, win.clone()));
                child = walker.GetNextSiblingElement(&win).ok();
            }
            Ok(out)
        }
    }
}

impl AccessibilityBackend for UiaBackend {
    fn label(&self) -> &'static str {
        "uia"
    }

    fn enumerate_apps(&self) -> BoxFuture<'_, Result<Vec<AppHandle>>> {
        Box::pin(async move {
            let windows = self.enumerate_windows()?;
            let mut apps = Vec::new();
            {
                let mut cache = self.app_handles.lock().expect("mutex");
                for (id, el) in &windows {
                    cache.insert(id.clone(), el.clone());
                }
            }
            for (id, el) in windows {
                let name = unsafe { el.CurrentName().map(|b| b.to_string()).unwrap_or_default() };
                apps.push(AppHandle { app_id: name.clone(), title: name, backend_node_id: id });
            }
            Ok(apps)
        })
    }

    fn snapshot_app<'a>(&'a self, app: &'a AppHandle) -> BoxFuture<'a, Result<Vec<SlugNode>>> {
        Box::pin(async move {
            let window = self.find_window(&app.backend_node_id)?;
            let walker = unsafe {
                self.automation
                    .ControlViewWalker()
                    .map_err(|e| BridgeError::Backend(format!("ControlViewWalker: {e}")))?
            };

            let mut nodes: Vec<SlugNode> = Vec::new();
            let mut handles = self.handles.lock().expect("mutex");
            let mut node_count = 0usize;
            let mut max_depth = 0usize;

            // Iterative DFS: (element, parent_ref, window_ref, depth).
            let win_ref = derive_ref(&app.backend_node_id);
            let mut stack: Vec<(IUIAutomationElement, Option<String>, String, usize)> =
                vec![(window, None, win_ref.clone(), 0)];

            while let Some((el, parent_ref, window_ref, depth)) = stack.pop() {
                if node_count >= MAX_NODES || depth > MAX_DEPTH {
                    warn!(app = %app.app_id, "UIA harvest limit hit; truncating");
                    break;
                }
                let nid = native_id(&el);
                let slug_ref = derive_ref(&nid);

                let ct = unsafe { el.CurrentControlType() }.unwrap_or(UIA_CONTROLTYPE_ID(0));
                let role = map_control_type(ct);
                let mut node = SlugNode::new(&slug_ref, role);
                node.parent_ref = parent_ref.clone();
                node.app_id = app.app_id.clone();
                node.window_id = window_ref.clone();
                node.states = unsafe { read_states(&el) };
                if let Ok(name) = unsafe { el.CurrentName() } {
                    let s = name.to_string();
                    if !s.is_empty() {
                        node.name = Some(s);
                    }
                }
                if let Ok(r) = unsafe { el.CurrentBoundingRectangle() } {
                    node.bounds = Some(Bounds {
                        x: r.left as f64,
                        y: r.top as f64,
                        width: (r.right - r.left) as f64,
                        height: (r.bottom - r.top) as f64,
                    });
                }

                // Children (collect first so we can record child_refs).
                let mut children: Vec<IUIAutomationElement> = Vec::new();
                let mut c = unsafe { walker.GetFirstChildElement(&el).ok() };
                while let Some(ch) = c {
                    children.push(ch.clone());
                    c = unsafe { walker.GetNextSiblingElement(&ch).ok() };
                }
                node.child_refs =
                    children.iter().map(|ch| derive_ref(&native_id(ch))).collect();

                handles.insert(slug_ref.clone(), el.clone());
                nodes.push(node);
                node_count += 1;
                max_depth = max_depth.max(depth);

                let next_window = if matches!(role, SlugRole::Window | SlugRole::Dialog) {
                    slug_ref.clone()
                } else {
                    window_ref.clone()
                };
                for ch in children.into_iter().rev() {
                    stack.push((ch, Some(slug_ref.clone()), next_window.clone(), depth + 1));
                }
            }

            drop(handles);
            let cov = coverage::assess(&app.app_id, &win_ref, node_count, max_depth);
            self.coverage.lock().expect("mutex").insert(app.backend_node_id.clone(), cov);
            Ok(nodes)
        })
    }

    fn invoke<'a>(
        &'a self,
        node_id: &'a BackendNodeId,
        action: &'a Action,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let el = self
                .handles
                .lock()
                .expect("mutex")
                .get(node_id.as_str())
                .cloned()
                .ok_or_else(|| BridgeError::UnknownRef(node_id.0.clone()))?;
            unsafe { perform(&el, action) }
        })
    }

    fn subscribe_events(&self, _sink: EventSink) -> BoxFuture<'_, Result<Subscription>> {
        Box::pin(async move {
            // Live UIA events need COM event-sink objects implementing
            // IUIAutomationEventHandler / IUIAutomationFocusChangedEventHandler /
            // IUIAutomationPropertyChangedEventHandler / IUIAutomationStructureChangedEventHandler
            // registered via Add*EventHandler, marshalled back to async. Tracked as
            // a follow-up; snapshot + invoke are the supported core for M1.5.
            warn!("UIA live events not yet wired; snapshot/invoke only");
            Ok(Subscription::detached())
        })
    }

    fn coverage(&self, app: &AppHandle) -> Coverage {
        self.coverage
            .lock()
            .expect("mutex")
            .get(&app.backend_node_id)
            .cloned()
            .unwrap_or_else(|| coverage::assess(&app.app_id, &derive_ref(&app.backend_node_id), 0, 0))
    }
}

/// Perform an action via the appropriate UIA control pattern.
unsafe fn perform(el: &IUIAutomationElement, action: &Action) -> Result<()> {
    unsafe {
        match action {
            Action::Activate => {
                let p: IUIAutomationInvokePattern = el
                    .GetCurrentPatternAs(UIA_InvokePatternId)
                    .map_err(|_| BridgeError::InterfaceMissing("InvokePattern"))?;
                p.Invoke().map_err(|e| BridgeError::Backend(e.to_string()))?;
            }
            Action::Focus => {
                el.SetFocus().map_err(|e| BridgeError::Backend(e.to_string()))?;
            }
            Action::SetText(text) => {
                let p: IUIAutomationValuePattern = el
                    .GetCurrentPatternAs(UIA_ValuePatternId)
                    .map_err(|_| BridgeError::InterfaceMissing("ValuePattern"))?;
                let h = BSTR::from(text.as_str());
                p.SetValue(&h).map_err(|e| BridgeError::Backend(e.to_string()))?;
            }
            Action::SetValue(v) => {
                // RangeValuePattern would be ideal; fall back to ValuePattern text.
                let p: IUIAutomationValuePattern = el
                    .GetCurrentPatternAs(UIA_ValuePatternId)
                    .map_err(|_| BridgeError::InterfaceMissing("ValuePattern"))?;
                let h = BSTR::from(format_number(*v).as_str());
                p.SetValue(&h).map_err(|e| BridgeError::Backend(e.to_string()))?;
            }
            Action::Named(name) => match name.as_str() {
                "toggle" | "check" | "uncheck" => {
                    let p: IUIAutomationTogglePattern = el
                        .GetCurrentPatternAs(UIA_TogglePatternId)
                        .map_err(|_| BridgeError::InterfaceMissing("TogglePattern"))?;
                    p.Toggle().map_err(|e| BridgeError::Backend(e.to_string()))?;
                }
                "select" => {
                    let p: IUIAutomationSelectionItemPattern = el
                        .GetCurrentPatternAs(UIA_SelectionItemPatternId)
                        .map_err(|_| BridgeError::InterfaceMissing("SelectionItemPattern"))?;
                    p.Select().map_err(|e| BridgeError::Backend(e.to_string()))?;
                }
                "expand" => {
                    let p: IUIAutomationExpandCollapsePattern = el
                        .GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
                        .map_err(|_| BridgeError::InterfaceMissing("ExpandCollapsePattern"))?;
                    p.Expand().map_err(|e| BridgeError::Backend(e.to_string()))?;
                }
                "collapse" => {
                    let p: IUIAutomationExpandCollapsePattern = el
                        .GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
                        .map_err(|_| BridgeError::InterfaceMissing("ExpandCollapsePattern"))?;
                    p.Collapse().map_err(|e| BridgeError::Backend(e.to_string()))?;
                }
                "scroll_into_view" => {
                    let p: IUIAutomationScrollItemPattern = el
                        .GetCurrentPatternAs(UIA_ScrollItemPatternId)
                        .map_err(|_| BridgeError::InterfaceMissing("ScrollItemPattern"))?;
                    p.ScrollIntoView().map_err(|e| BridgeError::Backend(e.to_string()))?;
                }
                other => {
                    return Err(BridgeError::ActionUnavailable {
                        slug_ref: native_id(el),
                        action: other.to_string(),
                    })
                }
            },
            // Synthetic OS input is routed through `synth_input`, not node actions.
            Action::Key(_) | Action::TypeText(_) => {
                return Err(BridgeError::Unsupported(
                    "synthetic input is not yet implemented on Windows".into(),
                ))
            }
        }
    }
    Ok(())
}

/// Read the node's salient states from UIA properties.
unsafe fn read_states(el: &IUIAutomationElement) -> Vec<SlugState> {
    unsafe {
        let mut states = Vec::new();
        if el.CurrentIsEnabled().map(|b| b.as_bool()).unwrap_or(false) {
            states.push(SlugState::Enabled);
        }
        // On-screen → Showing/Visible; offscreen → omit (the YAML renderer treats
        // absence accordingly).
        if !el.CurrentIsOffscreen().map(|b| b.as_bool()).unwrap_or(false) {
            states.push(SlugState::Showing);
            states.push(SlugState::Visible);
        }
        states
    }
}

/// Map a UIA control type id to a [`SlugRole`].
///
/// `UIA_CONTROLTYPE_ID` is a windows-rs newtype that can't be used in `match`
/// patterns, so we compare by equality.
fn map_control_type(ct: UIA_CONTROLTYPE_ID) -> SlugRole {
    let table = [
        (UIA_ButtonControlTypeId, SlugRole::Button),
        (UIA_CheckBoxControlTypeId, SlugRole::Checkbox),
        (UIA_RadioButtonControlTypeId, SlugRole::RadioButton),
        (UIA_ComboBoxControlTypeId, SlugRole::ComboBox),
        (UIA_EditControlTypeId, SlugRole::Entry),
        (UIA_HyperlinkControlTypeId, SlugRole::Link),
        (UIA_ImageControlTypeId, SlugRole::Image),
        (UIA_ListItemControlTypeId, SlugRole::ListItem),
        (UIA_ListControlTypeId, SlugRole::List),
        (UIA_MenuControlTypeId, SlugRole::Menu),
        (UIA_MenuBarControlTypeId, SlugRole::MenuBar),
        (UIA_MenuItemControlTypeId, SlugRole::MenuItem),
        (UIA_ProgressBarControlTypeId, SlugRole::ProgressBar),
        (UIA_ScrollBarControlTypeId, SlugRole::ScrollBar),
        (UIA_SliderControlTypeId, SlugRole::Slider),
        (UIA_SpinnerControlTypeId, SlugRole::SpinButton),
        (UIA_StatusBarControlTypeId, SlugRole::StatusBar),
        (UIA_TabControlTypeId, SlugRole::PageTabList),
        (UIA_TabItemControlTypeId, SlugRole::PageTab),
        (UIA_TextControlTypeId, SlugRole::StaticText),
        (UIA_ToolBarControlTypeId, SlugRole::ToolBar),
        (UIA_ToolTipControlTypeId, SlugRole::ToolTip),
        (UIA_TreeControlTypeId, SlugRole::Tree),
        (UIA_TreeItemControlTypeId, SlugRole::TreeItem),
        (UIA_GroupControlTypeId, SlugRole::Group),
        (UIA_DocumentControlTypeId, SlugRole::Document),
        (UIA_WindowControlTypeId, SlugRole::Window),
        (UIA_PaneControlTypeId, SlugRole::Panel),
        (UIA_TableControlTypeId, SlugRole::Table),
        (UIA_HeaderControlTypeId, SlugRole::Header),
        (UIA_SeparatorControlTypeId, SlugRole::Separator),
        (UIA_TitleBarControlTypeId, SlugRole::TitleBar),
        (UIA_CalendarControlTypeId, SlugRole::DateEditor),
        (UIA_DataGridControlTypeId, SlugRole::Grid),
        (UIA_DataItemControlTypeId, SlugRole::Row),
    ];
    table
        .into_iter()
        .find(|(id, _)| *id == ct)
        .map(|(_, role)| role)
        .unwrap_or(SlugRole::Generic)
}

/// The platform-native stable identity (brief §4): the stringified UIA RuntimeId.
fn native_id(el: &IUIAutomationElement) -> String {
    match unsafe { el.GetRuntimeId() } {
        Ok(psa) if !psa.is_null() => {
            let s = unsafe { runtime_id_to_string(psa) };
            unsafe { let _ = SafeArrayDestroy(psa); }
            format!("uia:{s}")
        }
        _ => {
            // Fallback: compose from name + control type if RuntimeId is absent.
            let name = unsafe { el.CurrentName().map(|b| b.to_string()).unwrap_or_default() };
            let ct = unsafe { el.CurrentControlType() }.unwrap_or(UIA_CONTROLTYPE_ID(0));
            format!("uia:fallback:{}:{}", ct.0, name)
        }
    }
}

/// Read a 1-D SAFEARRAY of i4 (the RuntimeId) into a dotted string.
unsafe fn runtime_id_to_string(psa: *mut SAFEARRAY) -> String {
    unsafe {
        let lb = SafeArrayGetLBound(psa, 1).unwrap_or(0);
        let ub = SafeArrayGetUBound(psa, 1).unwrap_or(-1);
        let mut parts = Vec::new();
        for i in lb..=ub {
            let mut val: i32 = 0;
            if SafeArrayGetElement(psa, &i, &mut val as *mut i32 as *mut core::ffi::c_void).is_ok() {
                parts.push(val.to_string());
            }
        }
        parts.join(".")
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
