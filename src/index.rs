//! Python wrappers over grit-lib's index (`Index`, `IndexEntry`) write surface.

use std::sync::{Arc, Mutex};

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::error::map_err;
use crate::objects::ObjectId;

// AIDEV-NOTE: Wraps grit_lib::index::IndexEntry (a 15-field struct). The constructor exposes
// the settable stat/mode/oid/path/flags subset; flags_extended is always None and
// base_index_pos always 0 (split-index is not a Phase A concern). `flags` defaults to 0; the
// index serializer recomputes the low 12 bits (path length) on write, so 0 is safe for a
// normal stage-0 entry.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct IndexEntry {
    pub(crate) inner: grit_lib::index::IndexEntry,
}

#[pymethods]
impl IndexEntry {
    #[new]
    #[pyo3(signature = (path, oid, mode, *, ctime=(0, 0), mtime=(0, 0),
                        dev=0, ino=0, uid=0, gid=0, size=0, flags=0))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        path: Vec<u8>,
        oid: ObjectId,
        mode: u32,
        ctime: (u32, u32),
        mtime: (u32, u32),
        dev: u32,
        ino: u32,
        uid: u32,
        gid: u32,
        size: u32,
        flags: u16,
    ) -> Self {
        Self {
            inner: grit_lib::index::IndexEntry {
                ctime_sec: ctime.0,
                ctime_nsec: ctime.1,
                mtime_sec: mtime.0,
                mtime_nsec: mtime.1,
                dev,
                ino,
                mode,
                uid,
                gid,
                size,
                oid: oid.inner(),
                flags,
                flags_extended: None,
                path,
                base_index_pos: 0,
            },
        }
    }

    #[getter]
    fn path<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.path)
    }
    #[getter]
    fn oid(&self) -> ObjectId {
        ObjectId::from_inner(self.inner.oid)
    }
    #[getter]
    fn mode(&self) -> u32 {
        self.inner.mode
    }
    #[getter]
    fn ctime(&self) -> (u32, u32) {
        (self.inner.ctime_sec, self.inner.ctime_nsec)
    }
    #[getter]
    fn mtime(&self) -> (u32, u32) {
        (self.inner.mtime_sec, self.inner.mtime_nsec)
    }
    #[getter]
    fn dev(&self) -> u32 {
        self.inner.dev
    }
    #[getter]
    fn ino(&self) -> u32 {
        self.inner.ino
    }
    #[getter]
    fn uid(&self) -> u32 {
        self.inner.uid
    }
    #[getter]
    fn gid(&self) -> u32 {
        self.inner.gid
    }
    #[getter]
    fn size(&self) -> u32 {
        self.inner.size
    }
    #[getter]
    fn flags(&self) -> u16 {
        self.inner.flags
    }
}

// AIDEV-NOTE: `Index` owns a grit_lib::index::Index behind a Mutex (binding-owned mutable
// value; grit's Index mutators take &mut self) plus an Arc<Repository> so write_tree (Task 5) can
// reach the odb and write() can target the repo's default index path. Index methods run UNDER the
// GIL: a std MutexGuard is !Send and cannot be held across allow_threads, and Phase A index ops
// are fast enough that this is fine.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct Index {
    inner: Mutex<grit_lib::index::Index>,
    repo: Arc<grit_lib::repo::Repository>,
}

impl Index {
    pub fn new_loaded(inner: grit_lib::index::Index, repo: Arc<grit_lib::repo::Repository>) -> Self {
        Self {
            inner: Mutex::new(inner),
            repo,
        }
    }
}

#[pymethods]
impl Index {
    // AIDEV-NOTE: Add a synthetic entry (blob already in the odb). Stat fields are zeroed (the
    // commit_tree.rs pattern); `flags` carries the path length so the in-memory entry is
    // well-formed, though the writer recomputes it. add_or_replace upserts by (path, stage 0).
    fn add(&self, path: Vec<u8>, oid: ObjectId, mode: u32) {
        let entry = grit_lib::index::IndexEntry {
            ctime_sec: 0,
            ctime_nsec: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            size: 0,
            oid: oid.inner(),
            flags: (path.len().min(0xFFF)) as u16,
            flags_extended: None,
            path,
            base_index_pos: 0,
        };
        self.inner.lock().unwrap().add_or_replace(entry);
    }

    fn add_entry(&self, entry: PyRef<'_, IndexEntry>) {
        self.inner.lock().unwrap().add_or_replace(entry.inner.clone());
    }

    fn remove(&self, path: Vec<u8>) -> bool {
        self.inner.lock().unwrap().remove(&path)
    }

    // AIDEV-NOTE: Persist the index. `path=None` writes the repo's default index (via
    // Repository::write_index, which honors sparse-index collapsing); an explicit path uses
    // Index::write directly. Runs under the GIL — a std MutexGuard is !Send so it cannot be held
    // across allow_threads, and index serialization is fast enough that this is fine for Phase A.
    #[pyo3(signature = (path=None))]
    fn write(&self, path: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        match path {
            None => {
                let mut guard = self.inner.lock().unwrap();
                self.repo.write_index(&mut guard).map_err(map_err)
            }
            Some(p) => {
                let pathbuf = crate::repository::extract_path(p)?;
                let guard = self.inner.lock().unwrap();
                guard.write(&pathbuf).map_err(map_err)
            }
        }
    }
}
