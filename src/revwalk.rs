//! Lazy revision-walk iterator over a precomputed commit order.

use std::sync::Arc;

use pyo3::prelude::*;

use crate::error::map_err;
use crate::objects::{Commit, ObjectId};

// AIDEV-NOTE: grit-lib's `rev_list` is BATCH — it returns the full ordered `Vec<ObjectId>`
// of the ancestor walk up front (it is NOT a lazy iterator; see api-matrix Revwalk section
// and rev_list.rs). So `RevWalk` precomputes the oid sequence once (in
// `Repository::revwalk`) and parses each commit LAZILY here in `__next__`, yielding a
// `Commit`. The ordering is decided entirely by `rev_list`/`RevListOptions` at build time;
// this iterator only walks a fixed oid slice and does one odb read + parse per step.
//
// AIDEV-NOTE: Owning-iterator design (design §6), mirroring objects::Tree/TreeIter and
// refs::ReferenceIter. `RevWalk` holds an `Arc<Repository>` (for the per-step odb read) and
// an `Arc<[ObjectId]>` (the oid order). Both are owned Arcs, so the walk outlives the parent
// Python `Repository` handle — `del repo; gc.collect()` mid-iteration must not crash (see
// tests/test_ffi_lifetime.py).
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct RevWalk {
    repo: Arc<grit_lib::repo::Repository>,
    oids: Arc<[grit_lib::objects::ObjectId]>,
    idx: usize,
}

#[pymethods]
impl RevWalk {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Commit>> {
        let Some(oid) = self.oids.get(self.idx).copied() else {
            return Ok(None);
        };
        self.idx += 1;
        // The odb read releases the GIL; `oid` is an owned Copy so it moves into the closure
        // with no lifetime tie back into `self`. Commit::from_bytes runs under the GIL.
        let data = py
            .allow_threads(|| self.repo.odb.read(&oid))
            .map_err(map_err)?
            .data;
        let commit = Commit::from_bytes(py, ObjectId::from_inner(oid), &data)?;
        Ok(Some(commit))
    }
}

impl RevWalk {
    pub fn new(
        repo: Arc<grit_lib::repo::Repository>,
        oids: Vec<grit_lib::objects::ObjectId>,
    ) -> Self {
        Self {
            repo,
            oids: Arc::from(oids),
            idx: 0,
        }
    }
}
