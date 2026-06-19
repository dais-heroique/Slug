//! C ABI for the `slug-ui` SDK.
//!
//! Mirrors AccessKit's binding strategy: a thin, opaque-handle C layer over the
//! Rust toolkit so non-Rust apps get the same completeness guarantee. The app is
//! described by a JSON spec (see [`slug_ui::declarative`]); all data crosses the
//! boundary as JSON / C strings.
//!
//! Memory rule: every `char*` returned by this library must be freed with
//! [`slug_ui_string_free`]; every app handle with [`slug_ui_app_free`].
//!
//! ```c
//! SlugUiApp* app = slug_ui_app_new(spec_json, "{}");
//! char* tree = slug_ui_snapshot_json(app);   // semantic tree as JSON
//! slug_ui_invoke(app, ref, "set_text", "hi");
//! char* events = slug_ui_drain_events_json(app);
//! slug_ui_string_free(tree); slug_ui_string_free(events);
//! slug_ui_app_free(app);
//! ```

use std::ffi::{c_char, c_int, CStr, CString};

use slug_ui::DeclarativeApp;

/// Opaque handle to a running declarative app.
pub struct SlugUiApp {
    inner: DeclarativeApp,
}

unsafe fn cstr<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

fn into_cstring(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

/// Create an app from a JSON `spec` and a JSON `initial_state` object (may be
/// `"{}"` or NULL). Returns NULL on error.
///
/// # Safety
/// `spec_json` must be a valid NUL-terminated UTF-8 string; `state_json` may be
/// NULL.
#[no_mangle]
pub unsafe extern "C" fn slug_ui_app_new(
    spec_json: *const c_char,
    state_json: *const c_char,
) -> *mut SlugUiApp {
    let Some(spec) = (unsafe { cstr(spec_json) }) else {
        return std::ptr::null_mut();
    };
    let spec_val: serde_json::Value = match serde_json::from_str(spec) {
        Ok(v) => v,
        Err(_) => return std::ptr::null_mut(),
    };
    let state_val: serde_json::Value = match unsafe { cstr(state_json) } {
        Some(s) if !s.is_empty() => serde_json::from_str(s).unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    };
    match DeclarativeApp::from_spec(spec_val, state_val) {
        Ok(inner) => Box::into_raw(Box::new(SlugUiApp { inner })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Return the current semantic snapshot (nodes + tools) as a JSON string.
///
/// # Safety
/// `app` must be a handle from [`slug_ui_app_new`].
#[no_mangle]
pub unsafe extern "C" fn slug_ui_snapshot_json(app: *mut SlugUiApp) -> *mut c_char {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let json = serde_json::to_string(&app.inner.snapshot()).unwrap_or_default();
    into_cstring(json)
}

/// Perform an action on a node by ref. `args` may be NULL. Returns 0 on success,
/// -1 on error.
///
/// # Safety
/// `app` must be a valid handle; `node_ref`/`action` valid UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn slug_ui_invoke(
    app: *mut SlugUiApp,
    node_ref: *const c_char,
    action: *const c_char,
    args: *const c_char,
) -> c_int {
    let (Some(app), Some(node_ref), Some(action)) =
        (unsafe { app.as_mut() }, unsafe { cstr(node_ref) }, unsafe { cstr(action) })
    else {
        return -1;
    };
    match app.inner.invoke(node_ref, action, unsafe { cstr(args) }) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Drain queued events (button presses, menu selections) as a JSON array.
///
/// # Safety
/// `app` must be a valid handle.
#[no_mangle]
pub unsafe extern "C" fn slug_ui_drain_events_json(app: *mut SlugUiApp) -> *mut c_char {
    let Some(app) = (unsafe { app.as_mut() }) else {
        return std::ptr::null_mut();
    };
    into_cstring(serde_json::to_string(&app.inner.drain_events()).unwrap_or_default())
}

/// Return the current app state as a JSON object string.
///
/// # Safety
/// `app` must be a valid handle.
#[no_mangle]
pub unsafe extern "C" fn slug_ui_state_json(app: *mut SlugUiApp) -> *mut c_char {
    let Some(app) = (unsafe { app.as_ref() }) else {
        return std::ptr::null_mut();
    };
    into_cstring(serde_json::to_string(app.inner.state()).unwrap_or_default())
}

/// Free a string returned by this library.
///
/// # Safety
/// `s` must have been returned by one of this library's functions, and freed once.
#[no_mangle]
pub unsafe extern "C" fn slug_ui_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Free an app handle.
///
/// # Safety
/// `app` must have been returned by [`slug_ui_app_new`], and freed once.
#[no_mangle]
pub unsafe extern "C" fn slug_ui_app_free(app: *mut SlugUiApp) {
    if !app.is_null() {
        drop(unsafe { Box::from_raw(app) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_c_api() {
        let spec = CString::new(
            r#"{"app":"t","root":{"type":"container","id":"root","children":[
                {"type":"textbox","id":"name","label":"Name","field":"name"}]}}"#,
        )
        .unwrap();
        let app = unsafe { slug_ui_app_new(spec.as_ptr(), std::ptr::null()) };
        assert!(!app.is_null());

        let snap = unsafe { slug_ui_snapshot_json(app) };
        let snap_str = unsafe { CStr::from_ptr(snap) }.to_str().unwrap().to_string();
        assert!(snap_str.contains("\"ref\""));
        unsafe { slug_ui_string_free(snap) };

        // Find the entry ref and set its text via the C API.
        let v: serde_json::Value = serde_json::from_str(&snap_str).unwrap();
        let entry = v["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["role"] == "ENTRY")
            .unwrap()["ref"]
            .as_str()
            .unwrap()
            .to_string();
        let r = CString::new(entry).unwrap();
        let action = CString::new("set_text").unwrap();
        let args = CString::new("Ada").unwrap();
        assert_eq!(unsafe { slug_ui_invoke(app, r.as_ptr(), action.as_ptr(), args.as_ptr()) }, 0);

        let state = unsafe { slug_ui_state_json(app) };
        let state_str = unsafe { CStr::from_ptr(state) }.to_str().unwrap();
        assert!(state_str.contains("Ada"));
        unsafe { slug_ui_string_free(state) };

        unsafe { slug_ui_app_free(app) };
    }
}
