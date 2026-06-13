// AIDEV-NOTE: pyo3 0.23's #[pyfunction] expansion inserts a no-op PyErr -> PyErr
// `.into()` (from the trailing `?` on a PyResult). clippy flags this as
// useless_conversion at the macro site, and a function-level #[allow] does not
// reach it, so the allow must be crate-level. Revisit once the typed-error layer
// (Phase 8.x) returns a domain error type instead of PyResult directly.
#![allow(clippy::useless_conversion)]

use std::path::Path;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use grit_lib::repo::Repository;
use grit_lib::rev_parse::resolve_revision;

/// Returns the pygrit version string. Smoke-test entry point for the spike.
#[pyfunction]
fn _hello() -> &'static str {
    "pygrit"
}

// AIDEV-NOTE: Spike-only binding. grit-lib's real read-core API is a free-function
// style: `Repository::discover(Some(&Path))` then `rev_parse::resolve_revision(&repo,
// "HEAD") -> ObjectId`, hex via `ObjectId::to_hex()`. See docs/superpowers/api-matrix.md.
// Error handling here is a placeholder (every grit_lib::error::Error -> RuntimeError);
// Phase 8.x replaces this with the table-driven GritError hierarchy from the matrix.
// NOTE: Repository::discover also consults GIT_DIR/GIT_WORK_TREE/cwd env even when a
// start path is given; explicit-open semantics come later.
#[pyfunction]
fn _discover_head_hex(path: &str) -> PyResult<String> {
    let repo = Repository::discover(Some(Path::new(path)))
        .map_err(|e| PyRuntimeError::new_err(format!("discover failed: {e}")))?;
    let oid = resolve_revision(&repo, "HEAD")
        .map_err(|e| PyRuntimeError::new_err(format!("resolve HEAD failed: {e}")))?;
    Ok(oid.to_hex())
}

#[pymodule]
fn _pygrit(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(_hello, m)?)?;
    m.add_function(wrap_pyfunction!(_discover_head_hex, m)?)?;
    Ok(())
}
