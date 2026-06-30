//! Python binding for the `slug-ui` SDK (PyO3), mirroring AccessKit's binding
//! strategy. Non-Rust apps describe their UI as a JSON spec and get the same
//! completeness guarantee — every widget is semantic, drivable by an agent.
//!
//! ```python
//! import slug_ui, json
//! app = slug_ui.SlugUiApp(json.dumps(spec))
//! tree = json.loads(app.snapshot())          # semantic tree
//! app.invoke(ref, "set_text", "hello")
//! events = json.loads(app.drain_events())    # button/menu emissions
//! ```

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use slugui::DeclarativeApp;

/// A declarative `slug-ui` app, driven from Python via JSON.
///
/// `unsendable`: the app's view/update closures are `Send` but not `Sync`, and a
/// UI is single-threaded anyway — PyO3 enforces it's used from its owning thread.
#[pyclass(unsendable)]
struct SlugUiApp {
    inner: DeclarativeApp,
}

#[pymethods]
impl SlugUiApp {
    /// Build from a JSON `spec` and optional JSON `state` object string.
    #[new]
    #[pyo3(signature = (spec, state=None))]
    fn new(spec: &str, state: Option<&str>) -> PyResult<Self> {
        let spec_val: serde_json::Value =
            serde_json::from_str(spec).map_err(|e| PyValueError::new_err(format!("bad spec: {e}")))?;
        let state_val: serde_json::Value = match state {
            Some(s) if !s.is_empty() => serde_json::from_str(s)
                .map_err(|e| PyValueError::new_err(format!("bad state: {e}")))?,
            _ => serde_json::Value::Null,
        };
        let inner = DeclarativeApp::from_spec(spec_val, state_val).map_err(PyValueError::new_err)?;
        Ok(SlugUiApp { inner })
    }

    /// The complete semantic snapshot (nodes + tools) as a JSON string.
    fn snapshot(&self) -> PyResult<String> {
        serde_json::to_string(&self.inner.snapshot())
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Perform an action on a node by ref. `args` is optional.
    #[pyo3(signature = (node_ref, action, args=None))]
    fn invoke(&mut self, node_ref: &str, action: &str, args: Option<&str>) -> PyResult<()> {
        self.inner.invoke(node_ref, action, args).map_err(PyValueError::new_err)
    }

    /// Drain queued events (button presses, menu selections) as a JSON array.
    fn drain_events(&mut self) -> PyResult<String> {
        serde_json::to_string(&self.inner.drain_events())
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// The current app state as a JSON object string.
    fn state(&self) -> PyResult<String> {
        serde_json::to_string(self.inner.state()).map_err(|e| PyValueError::new_err(e.to_string()))
    }
}

/// The `slug_ui` Python module.
#[pymodule]
fn slug_ui(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SlugUiApp>()?;
    m.add("__doc__", "Python SDK for the slug-ui semantic GUI toolkit.")?;
    Ok(())
}
