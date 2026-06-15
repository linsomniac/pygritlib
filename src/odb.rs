//! Python wrapper over `grit_lib::repo::Repository`'s object database (`Odb`).

use std::sync::Arc;

use pyo3::prelude::*;

use crate::error::map_err;
use crate::objects::{kind_to_py, Object, ObjectId};

// AIDEV-NOTE: Odb holds an `Arc<grit_lib::repo::Repository>` (a clone of the parent
// Repository's Arc) rather than a borrow, so a Python `repo.odb` handle keeps the repo
// alive independently of the Python `Repository` (design §6). grit-lib's Odb is reached
// via the public `repo.odb` field.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct Odb {
    pub(crate) repo: Arc<grit_lib::repo::Repository>,
}

#[pymethods]
impl Odb {
    // AIDEV-NOTE: `oid.inner()` returns an OWNED grit_lib ObjectId (Copy), so `want`
    // can move into the allow_threads closure with no lifetime tie to `oid`. The read
    // releases the GIL (decompress + hash verify can be heavy). `&self.repo.odb` is
    // Sync, so the closure is Send. On success we build the IntEnum kind member and
    // move the payload into an Arc<[u8]> for the frozen Object.
    fn read(&self, py: Python<'_>, oid: &ObjectId) -> PyResult<Object> {
        let want = oid.inner();
        let obj = py
            .allow_threads(|| self.repo.odb.read(&want))
            .map_err(map_err)?;
        let grit_lib::objects::Object { kind, data } = obj;
        let kind_py = kind_to_py(py, kind)?;
        Ok(Object::new(
            oid.clone(),
            kind_py,
            Arc::from(data.into_boxed_slice()),
        ))
    }

    fn exists(&self, py: Python<'_>, oid: &ObjectId) -> bool {
        let want = oid.inner();
        py.allow_threads(|| self.repo.odb.exists(&want))
    }
}
