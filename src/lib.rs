// AIDEV-NOTE: pyo3 0.23's #[pyfunction]/#[pymethods] expansion inserts a no-op
// PyErr -> PyErr `.into()` (from the trailing `?` on a PyResult). clippy flags this
// as useless_conversion at the macro site, and a function-level #[allow] does not
// reach it, so the allow must be crate-level. This is required by the read API's
// fallible methods (e.g. Repository::resolve/revwalk/diff in src/repository.rs).
// Revisit once a typed-error layer returns a domain error type instead of PyResult.
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;

mod config;
mod diff;
mod error;
mod objects;
mod odb;
mod refs;
mod repository;
mod revwalk;

#[pymodule]
fn _pylibgrit(m: &Bound<'_, PyModule>) -> PyResult<()> {
    error::register(m)?;
    m.add_class::<objects::ObjectId>()?;
    m.add_class::<objects::Object>()?;
    m.add_class::<objects::Commit>()?;
    m.add_class::<objects::Signature>()?;
    m.add_class::<objects::Tree>()?;
    m.add_class::<objects::TreeEntry>()?;
    m.add_class::<objects::TreeIter>()?;
    m.add_class::<objects::Blob>()?;
    m.add_class::<objects::Tag>()?;
    m.add_class::<odb::Odb>()?;
    m.add_class::<refs::Reference>()?;
    m.add_class::<refs::ReferenceIter>()?;
    m.add_class::<repository::Repository>()?;
    m.add_class::<config::ConfigSet>()?;
    m.add_class::<diff::Diff>()?;
    m.add_class::<diff::DiffEntry>()?;
    m.add_class::<diff::DiffStats>()?;
    // AIDEV-NOTE: DiffIter is an internal iterator (like TreeIter/ReferenceIter): registered
    // on the native module but NOT exported in python/pylibgrit/__init__.py's __all__. Users get
    // one via `iter(diff)`, never by constructing it directly.
    m.add_class::<diff::DiffIter>()?;
    // AIDEV-NOTE: RevWalk is an internal iterator (like TreeIter/ReferenceIter): registered
    // on the native module but NOT exported in python/pylibgrit/__init__.py's __all__. Users get
    // one via `repo.revwalk(start)`, never by constructing it directly.
    m.add_class::<revwalk::RevWalk>()?;
    Ok(())
}
