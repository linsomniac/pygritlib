//! Python wrappers over grit-lib object-model primitives (`ObjectId`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyBytes;

use crate::error::map_err;

// AIDEV-NOTE: We wrap grit-lib's own `ObjectId` (which derives
// Clone/Copy/Eq/Ord/Hash and provides to_hex/as_bytes/from_hex/from_bytes/algo)
// rather than reimplementing hex parsing — grit-lib owns the canonical SHA-1/256
// width logic. `frozen` makes the Python object immutable, matching the Copy oid.
#[pyclass(frozen, module = "pygrit._pygrit")]
#[derive(Clone)]
pub struct ObjectId {
    pub(crate) inner: grit_lib::objects::ObjectId,
}

#[pymethods]
impl ObjectId {
    /// Parses an `ObjectId` from a 40- (SHA-1) or 64-char (SHA-256) hex string.
    #[staticmethod]
    fn from_hex(hex: &str) -> PyResult<Self> {
        grit_lib::objects::ObjectId::from_hex(hex)
            .map(|inner| Self { inner })
            .map_err(map_err)
    }

    /// The lowercase hex digest (40 chars for SHA-1, 64 for SHA-256).
    #[getter]
    fn hex(&self) -> String {
        self.inner.to_hex()
    }

    /// The raw digest bytes (20 for SHA-1, 32 for SHA-256).
    #[getter]
    fn raw<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.as_bytes())
    }

    /// The hash algorithm name (`"sha1"` or `"sha256"`), inferred from length.
    #[getter]
    fn hash_algorithm(&self) -> &'static str {
        self.inner.algo().name()
    }

    fn __richcmp__(&self, other: &ObjectId, op: CompareOp) -> bool {
        match op {
            CompareOp::Eq => self.inner == other.inner,
            CompareOp::Ne => self.inner != other.inner,
            _ => false,
        }
    }

    fn __hash__(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.inner.hash(&mut h);
        h.finish()
    }

    fn __repr__(&self) -> String {
        format!("ObjectId('{}')", self.inner.to_hex())
    }
}

// AIDEV-NOTE: `#[allow(dead_code)]` is intentional — these conversion helpers are
// consumed by later tasks (2.6/2.7: odb read, parsed object views, refs) that
// produce ObjectId from a grit-lib oid and pass it back to grit-lib. Remove the
// allow once those bindings land and call from_inner/inner.
#[allow(dead_code)]
impl ObjectId {
    pub fn from_inner(inner: grit_lib::objects::ObjectId) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> grit_lib::objects::ObjectId {
        self.inner
    }
}

// AIDEV-NOTE: ObjectKind is a Python enum.IntEnum defined in python/pygrit/__init__.py.
// Native PyO3 enums lack .name and type-iteration, so kind getters return the IntEnum
// member instead. We cache the class once and construct members by integer value.
// The discriminants here MUST match the IntEnum values in __init__.py (asserted by a test).
static OBJECT_KIND_CLS: GILOnceCell<Py<PyAny>> = GILOnceCell::new();

fn object_kind_discriminant(k: grit_lib::objects::ObjectKind) -> i32 {
    match k {
        grit_lib::objects::ObjectKind::Commit => 0,
        grit_lib::objects::ObjectKind::Tree => 1,
        grit_lib::objects::ObjectKind::Blob => 2,
        grit_lib::objects::ObjectKind::Tag => 3,
    }
}

// AIDEV-NOTE: `#[allow(dead_code)]` is intentional — `kind_to_py` is consumed by the
// odb read binding (task 2.6) which surfaces a grit-lib ObjectKind to Python. Remove
// the allow once that caller lands.
/// Convert a grit-lib object kind into the public `pygrit.ObjectKind` IntEnum member.
#[allow(dead_code)]
pub fn kind_to_py(py: Python<'_>, k: grit_lib::objects::ObjectKind) -> PyResult<Py<PyAny>> {
    let cls = OBJECT_KIND_CLS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        Ok(py.import("pygrit")?.getattr("ObjectKind")?.unbind())
    })?;
    let member = cls.bind(py).call1((object_kind_discriminant(k),))?;
    Ok(member.unbind())
}
