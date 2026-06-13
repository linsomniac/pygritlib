// AIDEV-NOTE: pyo3 0.23's #[pyfunction] expansion inserts a no-op PyErr -> PyErr
// `.into()` (from the trailing `?` on a PyResult). clippy flags this as
// useless_conversion at the macro site, and a function-level #[allow] does not
// reach it, so the allow must be crate-level. Revisit once the typed-error layer
// (Phase 8.x) returns a domain error type instead of PyResult directly.
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;

mod error;
mod objects;

/// Returns the pygrit version string. Smoke-test entry point for the spike.
#[pyfunction]
fn _hello() -> &'static str {
    "pygrit"
}

#[pymodule]
fn _pygrit(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(_hello, m)?)?;
    error::register(m)?;
    m.add_class::<objects::ObjectId>()?;
    Ok(())
}
