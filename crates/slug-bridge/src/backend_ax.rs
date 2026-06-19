//! macOS accessibility backend: the Accessibility (AX) API (`AXUIElement`).
//!
//! Compiled only on macOS. Enumerates running GUI applications via `NSWorkspace`,
//! walks each app's `AXUIElement` tree through `kAXChildrenAttribute`, maps
//! `kAXRoleAttribute` → [`SlugRole`] and AX attributes (`kAXEnabledAttribute`,
//! `kAXFocusedAttribute`, …) → [`SlugState`], and performs actions via
//! `AXUIElementPerformAction` / `AXUIElementSetAttributeValue`. The node's native
//! identity (brief §4) is a hash of `{pid}:{ax_tree_path}`.
//!
//! ## TCC permission
//!
//! AX access requires the user to grant Accessibility permission. [`AxBackend::new`]
//! checks [`AXIsProcessTrusted`] and returns a typed
//! [`BridgeError::PermissionDenied`] (never a panic) with instructions if it is
//! not yet granted.
//!
//! Live events (`AXObserver` + `kAXFocusedUIElementChangedNotification` /
//! `kAXValueChangedNotification` / `kAXUIElementDestroyedNotification` /
//! `kAXCreatedNotification`, pumped on a dedicated `CFRunLoop`) are a documented
//! follow-up ([`AxBackend::subscribe_events`]); snapshot + invoke are fully
//! implemented.

#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::sync::Mutex;

use accessibility_sys::{
    kAXChildrenAttribute, kAXEnabledAttribute, kAXFocusedAttribute, kAXPositionAttribute,
    kAXPressAction, kAXRoleAttribute, kAXSizeAttribute, kAXTitleAttribute, kAXValueAttribute,
    kAXValueTypeCGPoint, kAXValueTypeCGSize, kAXButtonRole, kAXCheckBoxRole, kAXErrorSuccess,
    kAXMenuItemRole, kAXSliderRole, kAXTextFieldRole, kAXWindowRole, AXIsProcessTrusted,
    AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementPerformAction,
    AXUIElementRef, AXUIElementSetAttributeValue, AXValueGetValue, AXValueRef,
};
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use core_foundation_sys::base::{CFRelease, CFRetain, CFTypeRef};
use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
use slug_core::{derive_ref, Bounds, SlugNode, SlugRole, SlugState};
use tracing::{info, warn};

use crate::action::Action;
use crate::backend::{
    AccessibilityBackend, AppHandle, BackendNodeId, BoxFuture, EventSink, Subscription,
};
use crate::coverage::{self, Coverage};
use crate::error::{BridgeError, Result};

const MAX_DEPTH: usize = 60;
const MAX_NODES: usize = 20_000;

/// A reference-counted wrapper over an `AXUIElementRef` (CFType under the hood).
struct AxElem(AXUIElementRef);

impl AxElem {
    /// Wrap a +1 (create-rule) reference we already own.
    unsafe fn from_create(ptr: AXUIElementRef) -> Self {
        AxElem(ptr)
    }
    /// Wrap a borrowed (get-rule) reference, retaining it.
    unsafe fn retain(ptr: AXUIElementRef) -> Self {
        unsafe { CFRetain(ptr as CFTypeRef) };
        AxElem(ptr)
    }
    fn as_ref(&self) -> AXUIElementRef {
        self.0
    }
}

impl Clone for AxElem {
    fn clone(&self) -> Self {
        unsafe { CFRetain(self.0 as CFTypeRef) };
        AxElem(self.0)
    }
}

impl Drop for AxElem {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0 as CFTypeRef) };
        }
    }
}

// AX element refs are CoreFoundation objects; we serialize all access via Mutex.
unsafe impl Send for AxElem {}
unsafe impl Sync for AxElem {}

/// The macOS AX backend.
pub struct AxBackend {
    /// derived ref → AX element, captured during each snapshot.
    handles: Mutex<HashMap<String, AxElem>>,
    /// app backend_node_id (`{pid}:`) → pid.
    app_pids: Mutex<HashMap<String, i32>>,
    /// app backend_node_id → coverage from the most recent snapshot.
    coverage: Mutex<HashMap<String, Coverage>>,
}

impl AxBackend {
    /// Create the backend, verifying the process is trusted for accessibility.
    pub fn new() -> Result<Self> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(BridgeError::PermissionDenied(
                "Slug needs Accessibility permission. Grant it in System Settings → \
                 Privacy & Security → Accessibility (toggle on the app/terminal running \
                 Slug), then restart. (AXIsProcessTrusted() returned false.)"
                    .to_string(),
            ));
        }
        info!("AX accessibility permission granted");
        Ok(AxBackend {
            handles: Mutex::new(HashMap::new()),
            app_pids: Mutex::new(HashMap::new()),
            coverage: Mutex::new(HashMap::new()),
        })
    }
}

impl AccessibilityBackend for AxBackend {
    fn label(&self) -> &'static str {
        "ax"
    }

    fn enumerate_apps(&self) -> BoxFuture<'_, Result<Vec<AppHandle>>> {
        Box::pin(async move {
            let workspace = NSWorkspace::sharedWorkspace();
            let running = workspace.runningApplications();
            let mut apps = Vec::new();
            let mut pids = self.app_pids.lock().expect("mutex");
            for app in running.iter() {
                // GUI apps only (have a Dock presence / menu bar).
                if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
                    continue;
                }
                let pid = app.processIdentifier();
                let name = app.localizedName().map(|s| s.to_string()).unwrap_or_default();
                let backend_node_id = format!("{pid}:");
                pids.insert(backend_node_id.clone(), pid);
                apps.push(AppHandle { app_id: name.clone(), title: name, backend_node_id });
            }
            Ok(apps)
        })
    }

    fn focused_app(&self) -> BoxFuture<'_, Result<Option<AppHandle>>> {
        Box::pin(async move {
            let workspace = NSWorkspace::sharedWorkspace();
            let Some(app) = workspace.frontmostApplication() else { return Ok(None) };
            let pid = app.processIdentifier();
            let name = app.localizedName().map(|s| s.to_string()).unwrap_or_default();
            let backend_node_id = format!("{pid}:");
            self.app_pids.lock().expect("mutex").insert(backend_node_id.clone(), pid);
            Ok(Some(AppHandle { app_id: name.clone(), title: name, backend_node_id }))
        })
    }

    fn snapshot_app<'a>(&'a self, app: &'a AppHandle) -> BoxFuture<'a, Result<Vec<SlugNode>>> {
        Box::pin(async move {
            let pid = self
                .app_pids
                .lock()
                .expect("mutex")
                .get(&app.backend_node_id)
                .copied()
                .or_else(|| app.backend_node_id.trim_end_matches(':').parse::<i32>().ok())
                .ok_or_else(|| BridgeError::UnknownRef(app.backend_node_id.clone()))?;

            let root = unsafe { AxElem::from_create(AXUIElementCreateApplication(pid)) };
            if root.as_ref().is_null() {
                return Err(BridgeError::Backend(format!("no AX element for pid {pid}")));
            }

            let mut nodes: Vec<SlugNode> = Vec::new();
            let mut handles = self.handles.lock().expect("mutex");
            let mut node_count = 0usize;
            let mut max_depth = 0usize;

            // DFS: (element, parent_ref, window_ref, depth, ax_path).
            let mut stack: Vec<(AxElem, Option<String>, String, usize, String)> =
                vec![(root, None, String::new(), 0, String::new())];

            while let Some((el, parent_ref, window_ref, depth, path)) = stack.pop() {
                if node_count >= MAX_NODES || depth > MAX_DEPTH {
                    warn!(app = %app.app_id, "AX harvest limit hit; truncating");
                    break;
                }
                let native = format!("{pid}:{path}");
                let slug_ref = derive_ref(&native);
                let role_str =
                    string_attr(el.as_ref(), kAXRoleAttribute).unwrap_or_else(|| "AXUnknown".into());
                let role = map_role(&role_str);

                let mut node = SlugNode::new(&slug_ref, role);
                node.parent_ref = parent_ref.clone();
                node.app_id = app.app_id.clone();
                let this_window =
                    if matches!(role, SlugRole::Window | SlugRole::Dialog) || window_ref.is_empty() {
                        slug_ref.clone()
                    } else {
                        window_ref.clone()
                    };
                node.window_id = this_window.clone();
                node.states = read_states(el.as_ref());
                if let Some(title) = string_attr(el.as_ref(), kAXTitleAttribute) {
                    if !title.is_empty() {
                        node.name = Some(title);
                    }
                }
                if let Some(v) = value_attr(el.as_ref()) {
                    node.value = Some(v);
                }
                node.bounds = read_bounds(el.as_ref());

                let children = ax_children(el.as_ref());
                node.child_refs = children
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let child_role =
                            string_attr(children[i].as_ref(), kAXRoleAttribute).unwrap_or_default();
                        derive_ref(&format!("{pid}:{path}/{child_role}[{i}]"))
                    })
                    .collect();

                handles.insert(slug_ref.clone(), el.clone());
                nodes.push(node);
                node_count += 1;
                max_depth = max_depth.max(depth);

                for (i, child) in children.into_iter().enumerate().rev() {
                    let child_role =
                        string_attr(child.as_ref(), kAXRoleAttribute).unwrap_or_default();
                    let child_path = format!("{path}/{child_role}[{i}]");
                    stack.push((
                        child,
                        Some(slug_ref.clone()),
                        this_window.clone(),
                        depth + 1,
                        child_path,
                    ));
                }
            }

            drop(handles);
            let app_ref = derive_ref(&app.backend_node_id);
            let cov = coverage::assess(&app.app_id, &app_ref, node_count, max_depth);
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
            perform(el.as_ref(), action)
        })
    }

    fn synth_input<'a>(&'a self, action: &'a Action) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { crate::synth_macos::perform_synth(action) })
    }

    fn subscribe_events(&self, _sink: EventSink) -> BoxFuture<'_, Result<Subscription>> {
        Box::pin(async move {
            // Live AX events require an AXObserver (AXObserverCreate +
            // AXObserverAddNotification for kAXFocusedUIElementChanged /
            // kAXValueChanged / kAXUIElementDestroyed / kAXCreated) whose
            // run-loop source is pumped on a dedicated CFRunLoop thread, with the
            // C callback translating to SlugEvent. Tracked as a follow-up;
            // snapshot + invoke are the supported core for M1.5.
            warn!("AX live events not yet wired; snapshot/invoke only");
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

/// Perform an action via AX.
fn perform(el: AXUIElementRef, action: &Action) -> Result<()> {
    match action {
        Action::Activate => press_or_click(el),
        Action::Named(name) => match name.as_str() {
            "toggle" | "check" | "uncheck" | "select" | "expand" | "collapse" => {
                press_or_click(el)
            }
            other => Err(BridgeError::ActionUnavailable {
                slug_ref: String::new(),
                action: other.to_string(),
            }),
        },
        Action::Focus => ax_set(el, kAXFocusedAttribute, CFBoolean::true_value().as_CFType()),
        Action::SetText(text) => {
            ax_set(el, kAXValueAttribute, CFString::new(text).as_CFType())
        }
        Action::SetValue(v) => ax_set(el, kAXValueAttribute, CFNumber::from(*v).as_CFType()),
        // Synthetic input doesn't target a node; it's routed via `synth_input`.
        Action::Key(_) | Action::TypeText(_) | Action::MouseClick { .. } => {
            Err(BridgeError::InvalidArgs {
                action: action.id(),
                detail: "synthetic input must go through synth_input, not a node ref".into(),
            })
        }
    }
}

/// Press the element via AX; if it exposes no press action, fall back to a
/// synthetic mouse click at the centre of its bounds — so "click" works even on
/// canvas/graphics nodes that have geometry but no accessibility action. No
/// pixels are captured; the coordinates come from the element's own bounds.
fn press_or_click(el: AXUIElementRef) -> Result<()> {
    match ax_action(el, kAXPressAction) {
        Ok(()) => Ok(()),
        Err(e) => match read_bounds(el) {
            Some(b) => crate::synth_macos::mouse_click(b.x + b.width / 2.0, b.y + b.height / 2.0),
            None => Err(e),
        },
    }
}

/// Read a node's screen bounds from `AXPosition` + `AXSize` (AXValue wrappers).
fn read_bounds(el: AXUIElementRef) -> Option<Bounds> {
    #[repr(C)]
    struct CgPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    struct CgSize {
        width: f64,
        height: f64,
    }
    let pos = copy_attr(el, kAXPositionAttribute)?;
    let size = copy_attr(el, kAXSizeAttribute)?;
    let mut p = CgPoint { x: 0.0, y: 0.0 };
    let mut s = CgSize { width: 0.0, height: 0.0 };
    let okp = unsafe {
        AXValueGetValue(
            pos.as_CFTypeRef() as AXValueRef,
            kAXValueTypeCGPoint,
            &mut p as *mut _ as *mut std::ffi::c_void,
        )
    };
    let oks = unsafe {
        AXValueGetValue(
            size.as_CFTypeRef() as AXValueRef,
            kAXValueTypeCGSize,
            &mut s as *mut _ as *mut std::ffi::c_void,
        )
    };
    if okp && oks && (s.width > 0.0 || s.height > 0.0) {
        Some(Bounds { x: p.x, y: p.y, width: s.width, height: s.height })
    } else {
        None
    }
}

fn ax_action(el: AXUIElementRef, action: &str) -> Result<()> {
    let cf = CFString::new(action);
    let err = unsafe { AXUIElementPerformAction(el, cf.as_concrete_TypeRef()) };
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        Err(BridgeError::Backend(format!("AXUIElementPerformAction({action}) -> {err}")))
    }
}

fn ax_set(el: AXUIElementRef, attr: &str, value: CFType) -> Result<()> {
    let cf = CFString::new(attr);
    let err =
        unsafe { AXUIElementSetAttributeValue(el, cf.as_concrete_TypeRef(), value.as_CFTypeRef()) };
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        Err(BridgeError::Backend(format!("AXUIElementSetAttributeValue({attr}) -> {err}")))
    }
}

/// Copy an attribute as a generic CFType (create-rule owned).
fn copy_attr(el: AXUIElementRef, attr: &str) -> Option<CFType> {
    let cf_attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err =
        unsafe { AXUIElementCopyAttributeValue(el, cf_attr.as_concrete_TypeRef(), &mut value) };
    if err == kAXErrorSuccess && !value.is_null() {
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    } else {
        None
    }
}

fn string_attr(el: AXUIElementRef, attr: &str) -> Option<String> {
    copy_attr(el, attr)?.downcast::<CFString>().map(|s| s.to_string())
}

/// Read the node's value attribute as a string (text or number).
fn value_attr(el: AXUIElementRef) -> Option<String> {
    let cf = copy_attr(el, kAXValueAttribute)?;
    if let Some(s) = cf.downcast::<CFString>() {
        let s = s.to_string();
        return (!s.is_empty()).then_some(s);
    }
    if let Some(n) = cf.downcast::<CFNumber>() {
        if let Some(f) = n.to_f64() {
            return Some(format_number(f));
        }
    }
    None
}

fn read_states(el: AXUIElementRef) -> Vec<SlugState> {
    let mut states = Vec::new();
    if attr_bool(el, kAXEnabledAttribute) {
        states.push(SlugState::Enabled);
    }
    if attr_bool(el, kAXFocusedAttribute) {
        states.push(SlugState::Focused);
    }
    // AX exposes visible elements only, so showing/visible are implied.
    states.push(SlugState::Showing);
    states.push(SlugState::Visible);
    states
}

fn attr_bool(el: AXUIElementRef, attr: &str) -> bool {
    copy_attr(el, attr)
        .and_then(|cf| cf.downcast::<CFBoolean>())
        .map(|b| b.into())
        .unwrap_or(false)
}

/// Read `kAXChildrenAttribute` into a vector of retained elements.
fn ax_children(el: AXUIElementRef) -> Vec<AxElem> {
    let Some(cf) = copy_attr(el, kAXChildrenAttribute) else {
        return Vec::new();
    };
    let arr = cf.as_CFTypeRef() as CFArrayRef;
    let mut out = Vec::new();
    unsafe {
        let count = CFArrayGetCount(arr);
        for i in 0..count {
            let item = CFArrayGetValueAtIndex(arr, i) as AXUIElementRef;
            if !item.is_null() {
                out.push(AxElem::retain(item));
            }
        }
    }
    out
}

/// Map an AX role string to a [`SlugRole`].
fn map_role(role: &str) -> SlugRole {
    use accessibility_sys::{
        kAXCellRole, kAXComboBoxRole, kAXGroupRole, kAXImageRole, kAXListRole, kAXMenuBarItemRole,
        kAXMenuBarRole, kAXMenuButtonRole, kAXMenuRole, kAXOutlineRole, kAXPopUpButtonRole,
        kAXProgressIndicatorRole, kAXRadioButtonRole, kAXRowRole, kAXScrollBarRole, kAXSheetRole,
        kAXStaticTextRole, kAXTabGroupRole, kAXTableRole, kAXTextAreaRole, kAXToolbarRole,
    };
    match role {
        r if r == kAXButtonRole => SlugRole::Button,
        r if r == kAXMenuButtonRole || r == kAXPopUpButtonRole => SlugRole::PopupButton,
        r if r == kAXCheckBoxRole => SlugRole::Checkbox,
        r if r == kAXRadioButtonRole => SlugRole::RadioButton,
        r if r == kAXComboBoxRole => SlugRole::ComboBox,
        r if r == kAXTextFieldRole => SlugRole::Entry,
        r if r == kAXTextAreaRole => SlugRole::EntryMultiline,
        r if r == kAXImageRole => SlugRole::Image,
        r if r == kAXListRole => SlugRole::List,
        r if r == kAXOutlineRole => SlugRole::Tree,
        r if r == kAXRowRole => SlugRole::Row,
        r if r == kAXCellRole => SlugRole::Cell,
        r if r == kAXMenuRole => SlugRole::Menu,
        r if r == kAXMenuBarRole => SlugRole::MenuBar,
        r if r == kAXMenuItemRole || r == kAXMenuBarItemRole => SlugRole::MenuItem,
        r if r == kAXScrollBarRole => SlugRole::ScrollBar,
        r if r == kAXSliderRole => SlugRole::Slider,
        r if r == kAXStaticTextRole => SlugRole::StaticText,
        r if r == kAXTabGroupRole => SlugRole::PageTabList,
        r if r == kAXTableRole => SlugRole::Table,
        r if r == kAXToolbarRole => SlugRole::ToolBar,
        r if r == kAXProgressIndicatorRole => SlugRole::ProgressBar,
        r if r == kAXGroupRole => SlugRole::Group,
        r if r == kAXSheetRole => SlugRole::Dialog,
        r if r == kAXWindowRole => SlugRole::Window,
        _ => SlugRole::Generic,
    }
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
