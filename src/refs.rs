//! Reference views and an owning iterator over a repository's refs.

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::error::map_err;
use crate::objects::ObjectId;

// AIDEV-NOTE: Owning-iterator design (design §6), mirroring objects::Tree/TreeIter. grit's
// `list_refs` returns OWNED `(String, ObjectId)` pairs, which we copy into
// `Arc<[ReferenceData]>`. A `ReferenceIter` holds that Arc plus an `Arc` of the repo; each
// yielded `Reference` clones one `ReferenceData` and an `Arc<Repository>`. So a `Reference`
// (and the iterator) own ALL their data and stay valid after the parent `Repository` is
// dropped. The repo Arc is kept so `peel()` can resolve symbolic refs against `git_dir`.
//
// AIDEV-NOTE: HEAD/symbolic handling: a ref is EITHER direct (`target` is Some, holding the
// resolved oid) OR symbolic (`symbolic_target` is Some, holding e.g. b"refs/heads/main");
// exactly one is set. `list_refs` only yields direct refs (it resolves and excludes HEAD).
// Symbolic `Reference`s are produced only by `Repository::head()` (see repository.rs), which
// reads HEAD via `read_head`. `peel()` follows a symbolic ref to its final oid.
#[derive(Clone)]
pub struct ReferenceData {
    name: Vec<u8>,
    target: Option<grit_lib::objects::ObjectId>, // direct oid; None for symbolic
    symbolic_target: Option<Vec<u8>>,            // e.g. b"refs/heads/main"; None for direct
}

impl ReferenceData {
    /// A direct reference: `name` resolved to `oid`.
    pub fn direct(name: Vec<u8>, oid: grit_lib::objects::ObjectId) -> Self {
        Self {
            name,
            target: Some(oid),
            symbolic_target: None,
        }
    }

    /// A symbolic reference: `name` points at another ref `symbolic_target`.
    pub fn symbolic(name: Vec<u8>, symbolic_target: Vec<u8>) -> Self {
        Self {
            name,
            target: None,
            symbolic_target: Some(symbolic_target),
        }
    }
}

/// A single Git reference: a name plus either a direct oid or a symbolic target.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct Reference {
    repo: Arc<grit_lib::repo::Repository>, // so peel() can resolve symbolic refs
    data: ReferenceData,
}

impl Reference {
    pub fn new(repo: Arc<grit_lib::repo::Repository>, data: ReferenceData) -> Self {
        Self { repo, data }
    }
}

#[pymethods]
impl Reference {
    /// The full ref name as raw bytes (e.g. `b"refs/heads/main"`, `b"HEAD"`; design §5).
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data.name)
    }

    /// The direct target oid, or `None` for a symbolic ref.
    #[getter]
    fn target(&self) -> Option<ObjectId> {
        self.data.target.map(ObjectId::from_inner)
    }

    /// The symbolic target ref name as raw bytes, or `None` for a direct ref.
    #[getter]
    fn symbolic_target<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.data
            .symbolic_target
            .as_ref()
            .map(|s| PyBytes::new(py, s))
    }

    /// Whether this reference is symbolic (points at another ref).
    #[getter]
    fn is_symbolic(&self) -> bool {
        self.data.symbolic_target.is_some()
    }

    /// Resolve to a final object id (follows symbolic refs).
    fn peel(&self, py: Python<'_>) -> PyResult<ObjectId> {
        if let Some(oid) = self.data.target {
            return Ok(ObjectId::from_inner(oid));
        }
        // Symbolic: resolve via the ref name. Ref names are UTF-8 in practice.
        let name = std::str::from_utf8(&self.data.name)
            .map_err(|_| crate::error::invalid_ref("non-UTF-8 ref name"))?
            .to_owned();
        let git_dir = self.repo.git_dir.clone();
        let oid = py
            .allow_threads(|| grit_lib::refs::resolve_ref(&git_dir, &name))
            .map_err(map_err)?;
        Ok(ObjectId::from_inner(oid))
    }
}

/// Iterator over a repository's references; owns its own `Arc`s so it outlives the parent.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct ReferenceIter {
    repo: Arc<grit_lib::repo::Repository>,
    entries: Arc<[ReferenceData]>,
    idx: usize,
}

impl ReferenceIter {
    pub fn new(repo: Arc<grit_lib::repo::Repository>, entries: Vec<ReferenceData>) -> Self {
        Self {
            repo,
            entries: Arc::from(entries),
            idx: 0,
        }
    }
}

#[pymethods]
impl ReferenceIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<Reference> {
        let d = self.entries.get(self.idx)?.clone();
        self.idx += 1;
        Some(Reference {
            repo: Arc::clone(&self.repo),
            data: d,
        })
    }
}
