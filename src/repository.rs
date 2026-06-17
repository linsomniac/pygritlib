//! Python wrapper over `grit_lib::repo::Repository`.

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::error::map_err;

// AIDEV-NOTE: Accept str | bytes | os.PathLike[str] | os.PathLike[bytes] path inputs
// (design §5). On Unix, bytes map to an OsString 1:1 (exact non-UTF-8 path fidelity);
// str/os.PathLike[str] go through PyO3's PathBuf extractor (surrogateescape via fsdecode).
// We try the PathBuf extractor first so os.PathLike[str] (and str) take the standard path,
// then a raw `bytes` input, and FINALLY fall back to calling `os.fspath(obj)` ourselves to
// support an os.PathLike whose `__fspath__()` returns `bytes` (PyO3's PathBuf extractor
// calls os.fspath but then requires the result be str, so a bytes __fspath__ is rejected
// upstream). The os.fspath() result is then handled: str -> PathBuf, bytes -> OsString.
// This touches Python, so callers MUST run it BEFORE releasing the GIL with allow_threads.
pub(crate) fn extract_path(obj: &Bound<'_, PyAny>) -> PyResult<std::path::PathBuf> {
    if let Ok(p) = obj.extract::<std::path::PathBuf>() {
        return Ok(p);
    }
    if let Ok(p) = bytes_to_pathbuf(obj) {
        return Ok(p);
    }
    // Fall back to os.fspath() to handle os.PathLike whose __fspath__ returns bytes (or str).
    let py = obj.py();
    if let Ok(fspath) = py.import("os")?.call_method1("fspath", (obj,)) {
        if let Ok(p) = fspath.extract::<std::path::PathBuf>() {
            return Ok(p);
        }
        if let Ok(p) = bytes_to_pathbuf(&fspath) {
            return Ok(p);
        }
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "path must be str, bytes, or os.PathLike",
    ))
}

// AIDEV-NOTE: Map a Python `bytes` object to a PathBuf via OsString (Unix: bytes 1:1).
// Returns Err if `obj` is not extractable as Vec<u8> (so callers can chain fallbacks).
// On non-Unix there is no lossless bytes->path mapping, so this always errors there.
fn bytes_to_pathbuf(obj: &Bound<'_, PyAny>) -> PyResult<std::path::PathBuf> {
    let b = obj.extract::<Vec<u8>>()?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(std::path::PathBuf::from(std::ffi::OsString::from_vec(b)))
    }
    #[cfg(not(unix))]
    {
        let _ = b;
        Err(pyo3::exceptions::PyTypeError::new_err(
            "bytes paths are only supported on Unix",
        ))
    }
}

// AIDEV-NOTE: We hold an `Arc<grit_lib::repo::Repository>` so the `.odb` accessor can
// hand out an `Odb` that clones the Arc and outlives this Python `Repository` handle
// (design §6: a child Odb keeps the repo alive). grit-lib exposes git_dir/work_tree/odb
// as PUBLIC FIELDS (no getter methods); is_bare() is the only method here.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct Repository {
    pub(crate) inner: Arc<grit_lib::repo::Repository>,
}

#[pymethods]
impl Repository {
    // AIDEV-NOTE: discover/open release the GIL via allow_threads. grit-lib's
    // Repository and Error are Send (Odb is Arc<Mutex<..>>/PathBuf; Error wraps
    // io::Error + String), so the closure's `Result<Repository, Error>` is Send and
    // this compiles. These are not hot paths, but releasing the GIL keeps other
    // Python threads live during the filesystem walk.
    #[staticmethod]
    fn discover(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Self> {
        // extract_path touches Python, so do it BEFORE releasing the GIL.
        let path = extract_path(path)?;
        let repo = py
            .allow_threads(|| grit_lib::repo::Repository::discover(Some(&path)))
            .map_err(map_err)?;
        Ok(Self {
            inner: Arc::new(repo),
        })
    }

    // AIDEV-NOTE: Initialize (or reinitialize) a repository like `git init`. Wraps
    // grit_lib::repo::init_repository with template_dir=None, ref_storage="files" (the default
    // loose-ref backend; reftable is out of scope for Phase B). `initial_branch` becomes the
    // symbolic HEAD target refs/heads/<branch>; we validate it as a ref so a bad name cannot
    // corrupt HEAD. initial_branch=None defaults to "main" (matches our own default branch).
    // initial_branch is bytes (not str) for consistency with the rest of pylibgrit's write
    // surface (ref/name fields are all bytes for byte-fidelity).
    #[staticmethod]
    #[pyo3(signature = (path, *, bare=false, initial_branch=None))]
    fn init(
        py: Python<'_>,
        path: &Bound<'_, PyAny>,
        bare: bool,
        initial_branch: Option<Vec<u8>>,
    ) -> PyResult<Self> {
        let path = extract_path(path)?;
        let branch = match initial_branch {
            Some(b) => utf8_field("initial_branch", b)?,
            None => "main".to_owned(),
        };
        // Validate the resulting branch ref name (refs/heads/<branch>) before init writes HEAD.
        let mut full = b"refs/heads/".to_vec();
        full.extend_from_slice(branch.as_bytes());
        validate_ref_name(&full)?;
        let repo = py
            .allow_threads(|| grit_lib::repo::init_repository(&path, bare, &branch, None, "files"))
            .map_err(map_err)?;
        Ok(Self {
            inner: Arc::new(repo),
        })
    }

    // AIDEV-NOTE: Clone `url` into `path` (== `git clone`, worktree only). Assembles init + origin
    // config + fetch + branch/HEAD + checkout (grit has no clone porcelain). Uses tags="all" (git
    // clone fetches all tags). bare/shallow deferred (spec §1). Returns the opened Repository.
    // Progress is unconditionally NoProgress (grit-lib 0.4.1 forces no-progress; progress= param
    // dropped to avoid a misleading dead knob).
    #[staticmethod]
    #[pyo3(signature = (url, path, *, branch=None, username=None, password=None,
                        use_credential_helpers=true))]
    #[allow(clippy::too_many_arguments)]
    fn clone(
        py: Python<'_>,
        url: String,
        path: &Bound<'_, PyAny>,
        branch: Option<String>,
        username: Option<String>,
        password: Option<String>,
        use_credential_helpers: bool,
    ) -> PyResult<Self> {
        let path = extract_path(path)?;
        crate::remote::clone_impl(
            py,
            url,
            path,
            branch,
            username,
            password,
            use_credential_helpers,
        )
    }

    #[staticmethod]
    #[pyo3(signature = (git_dir, work_tree=None))]
    fn open(
        py: Python<'_>,
        git_dir: &Bound<'_, PyAny>,
        work_tree: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        // extract_path touches Python, so resolve both paths BEFORE releasing the GIL.
        let git_dir = extract_path(git_dir)?;
        let work_tree = work_tree.map(extract_path).transpose()?;
        let repo = py
            .allow_threads(|| grit_lib::repo::Repository::open(&git_dir, work_tree.as_deref()))
            .map_err(map_err)?;
        Ok(Self {
            inner: Arc::new(repo),
        })
    }

    // AIDEV-NOTE: Paths are returned as `bytes` (not str) to preserve non-UTF-8
    // filesystem path fidelity (design §5 byte policy). `as_encoded_bytes()` is the
    // round-trippable OS-native byte form (compare with os.fsencode() on the Python
    // side).
    #[getter]
    fn git_dir<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.git_dir.as_os_str().as_encoded_bytes())
    }

    #[getter]
    fn work_tree<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.inner
            .work_tree
            .as_ref()
            .map(|p| PyBytes::new(py, p.as_os_str().as_encoded_bytes()))
    }

    #[getter]
    fn is_bare(&self) -> bool {
        self.inner.is_bare()
    }

    #[getter]
    fn odb(&self) -> crate::odb::Odb {
        crate::odb::Odb {
            repo: Arc::clone(&self.inner),
        }
    }

    // AIDEV-NOTE: loads the effective config (system+global+local, git-like) on EACH
    // `.config` access — it reads the cascade files; cache at the call site if accessed
    // repeatedly. `include_system=true` matches git's effective resolution; the host
    // `/etc/gitconfig` cannot make tests non-deterministic for repo-local-set keys because
    // Local scope WINS LAST over System/Global (see ConfigSet::get, last-wins). The load
    // releases the GIL (filesystem reads + parse). The returned `ConfigSet` OWNS its entries,
    // so it outlives this Repository handle.
    #[getter]
    fn config(&self, py: Python<'_>) -> PyResult<crate::config::ConfigSet> {
        let git_dir = self.inner.git_dir.clone();
        let cfg = py
            .allow_threads(|| grit_lib::config::ConfigSet::load(Some(&git_dir), true))
            .map_err(map_err)?;
        Ok(crate::config::ConfigSet::new(cfg))
    }

    // AIDEV-NOTE: Read any object, VERIFY its kind is Commit, then `parse_commit` over its
    // bytes. We check the ODB object's kind BEFORE parsing (mirroring blob()): a blob whose
    // content happens to be a valid commit payload must NOT be accepted as a commit (type
    // invariant). A kind mismatch → InvalidObjectError; a corrupt commit → parse-fail →
    // InvalidObjectError. The odb read releases the GIL; parse_commit runs under the GIL (it
    // touches Python only when building Signatures). `oid.inner()` is an owned Copy, so it
    // moves into the closure with no lifetime tie to `oid`.
    fn commit(
        &self,
        py: Python<'_>,
        oid: &crate::objects::ObjectId,
    ) -> PyResult<crate::objects::Commit> {
        let want = oid.inner();
        let grit_lib::objects::Object { kind, data } = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?;
        if kind != grit_lib::objects::ObjectKind::Commit {
            return Err(crate::error::InvalidObjectError::new_err(format!(
                "object {} is a {}, not a commit",
                want.to_hex(),
                kind
            )));
        }
        crate::objects::Commit::from_bytes(py, oid.clone(), &data)
    }

    // AIDEV-NOTE: Read any object, VERIFY its kind is Tree, then `parse_tree` over its bytes.
    // We check the ODB object's kind BEFORE parsing (mirroring blob()): a blob whose content
    // happens to be a valid tree payload must NOT be accepted as a tree. A kind mismatch →
    // InvalidObjectError. Same GIL/lifetime pattern as `commit`. The returned `Tree` OWNS its
    // entries (Arc), so it outlives this Repository handle.
    fn tree(
        &self,
        py: Python<'_>,
        oid: &crate::objects::ObjectId,
    ) -> PyResult<crate::objects::Tree> {
        let want = oid.inner();
        let grit_lib::objects::Object { kind, data } = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?;
        if kind != grit_lib::objects::ObjectKind::Tree {
            return Err(crate::error::InvalidObjectError::new_err(format!(
                "object {} is a {}, not a tree",
                want.to_hex(),
                kind
            )));
        }
        crate::objects::Tree::from_bytes(&data)
    }

    // AIDEV-NOTE: Unlike commit/tree, a blob has no parser — its payload IS the body. But
    // the caller asked specifically for a blob, so we VERIFY the read object's kind is Blob
    // and raise InvalidObjectError otherwise (rather than silently returning a tree/commit's
    // bytes). `into_boxed_slice()` moves the payload into the shared `Arc<[u8]>`.
    fn blob(
        &self,
        py: Python<'_>,
        oid: &crate::objects::ObjectId,
    ) -> PyResult<crate::objects::Blob> {
        let want = oid.inner();
        let obj = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?;
        let grit_lib::objects::Object { kind, data } = obj;
        if kind != grit_lib::objects::ObjectKind::Blob {
            return Err(crate::error::InvalidObjectError::new_err(format!(
                "object {} is a {}, not a blob",
                want.to_hex(),
                kind
            )));
        }
        Ok(crate::objects::Blob::new(Arc::from(
            data.into_boxed_slice(),
        )))
    }

    // AIDEV-NOTE: Read any object, VERIFY its kind is Tag, then `parse_tag` over its bytes.
    // We check the ODB object's kind BEFORE parsing (mirroring blob()): a blob whose content
    // happens to be a valid tag payload must NOT be accepted as a tag. A kind mismatch →
    // InvalidObjectError; a non-UTF-8/corrupt tag → parse-fail → InvalidObjectError. Same
    // GIL/lifetime pattern as `commit`.
    fn tag(&self, py: Python<'_>, oid: &crate::objects::ObjectId) -> PyResult<crate::objects::Tag> {
        let want = oid.inner();
        let grit_lib::objects::Object { kind, data } = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?;
        if kind != grit_lib::objects::ObjectKind::Tag {
            return Err(crate::error::InvalidObjectError::new_err(format!(
                "object {} is a {}, not a tag",
                want.to_hex(),
                kind
            )));
        }
        crate::objects::Tag::from_bytes(py, &data)
    }

    // AIDEV-NOTE: We pass prefix="refs/" (NOT ""): in grit-lib 0.4.1, `list_refs(git_dir, "")`
    // walks `git_dir` itself and so INCLUDES top-level pseudorefs like HEAD (the spec's claim
    // that "" excludes HEAD is wrong for this version — verified against the source:
    // normalize_list_refs_prefix("") -> "" -> base == git_dir). Using "refs/" restricts the
    // walk to `refs/`, excluding HEAD/ORIG_HEAD/etc. and matching `git for-each-ref` exactly.
    // Use `head()` for HEAD. The returned ReferenceIter OWNS its data (Arc<[ReferenceData]> +
    // Arc<Repository>), so it outlives this Repository handle.
    fn references(&self, py: Python<'_>) -> PyResult<crate::refs::ReferenceIter> {
        let git_dir = self.inner.git_dir.clone();
        let refs = py
            .allow_threads(|| grit_lib::refs::list_refs(&git_dir, "refs/"))
            .map_err(map_err)?;
        let entries: Vec<crate::refs::ReferenceData> = refs
            .into_iter()
            .map(|(name, oid)| crate::refs::ReferenceData::direct(name.into_bytes(), oid))
            .collect();
        Ok(crate::refs::ReferenceIter::new(
            Arc::clone(&self.inner),
            entries,
        ))
    }

    // AIDEV-NOTE: Always load via grit's load_index — it returns an empty index with the REPO's
    // hash algo when the file is absent (load_expand_sparse_optional handles NotFound), so this is
    // correct for fresh repos AND SHA-256 repos. (An earlier exists()-check + Index::new() fallback
    // defaulted to SHA-1 for a fresh SHA-256 repo.)
    fn index(&self, py: Python<'_>) -> PyResult<crate::index::Index> {
        let loaded = py
            .allow_threads(|| self.inner.load_index())
            .map_err(map_err)?;
        Ok(crate::index::Index::new_loaded(
            loaded,
            Arc::clone(&self.inner),
        ))
    }

    // AIDEV-NOTE: HEAD is excluded from `list_refs`, so it gets its own accessor.
    // `read_head` returns `Some(refname)` when HEAD is symbolic (the normal case) and `None`
    // when detached. For a detached HEAD we resolve it to a direct oid via `resolve_ref`. The
    // returned `Reference` carries the repo Arc so its `peel()` can follow a symbolic HEAD.
    fn head(&self, py: Python<'_>) -> PyResult<crate::refs::Reference> {
        let git_dir = self.inner.git_dir.clone();
        let sym = py
            .allow_threads(|| grit_lib::refs::read_head(&git_dir))
            .map_err(map_err)?;
        let data = match sym {
            Some(refname) => {
                crate::refs::ReferenceData::symbolic(b"HEAD".to_vec(), refname.into_bytes())
            }
            None => {
                // Detached HEAD: resolve to a direct oid.
                let oid = py
                    .allow_threads(|| grit_lib::refs::resolve_ref(&git_dir, "HEAD"))
                    .map_err(map_err)?;
                crate::refs::ReferenceData::direct(b"HEAD".to_vec(), oid)
            }
        };
        Ok(crate::refs::Reference::new(Arc::clone(&self.inner), data))
    }

    // AIDEV-NOTE: `resolve_revision` is grit-lib's full rev-parse resolver. `self.inner` is
    // `Arc<Repository>`, which derefs to `&Repository` for the `&Repository` argument. See
    // tests/test_resolve.py for which rev-spec forms are supported (and which are xfail'd):
    // grit-lib 0.4.1 supports "HEAD", full/abbrev hex, ref names + DWIM, `^{tree}`/`^{commit}`
    // peeling, and `treeish:path`. An unknown bare ref returns Error::Message ("fatal:
    // ambiguous argument ..."), which maps to the base GritError (see test_resolve_unknown_raises).
    fn resolve(&self, py: Python<'_>, spec: String) -> PyResult<crate::objects::ObjectId> {
        let oid = py
            .allow_threads(|| grit_lib::rev_parse::resolve_revision(&self.inner, &spec))
            .map_err(map_err)?;
        Ok(crate::objects::ObjectId::from_inner(oid))
    }

    // AIDEV-NOTE: revwalk PRECOMPUTES the ordered oid sequence via grit-lib's batch
    // `rev_list`, then hands it to a lazy `RevWalk` iterator that reads+parses each commit
    // on demand (see src/revwalk.rs). The walk holds its own `Arc<Repository>`, so it
    // outlives this handle (design §6). The start is passed as an `ObjectId` (the plan calls
    // `repo.resolve("HEAD")`); we convert it to a 40/64-char hex spec for rev_list, which
    // treats positive specs as commit tips and returns all reachable ancestors in order.
    //
    // AIDEV-NOTE: ORDERING. grit-lib's `RevListOptions` natively supports
    // `ordering: OrderingMode { Default, DateOrderWalk, AuthorDateWalk, Topo, AuthorDateTopo }`
    // and `reverse: bool` (confirmed against grit-lib-0.4.1/src/rev_list.rs), so EVERY order
    // we expose is backed by grit-lib — nothing is binding-faked or xfail'd. We map `order=`:
    //   - None / "date"  -> OrderingMode::Default       (== `git rev-list HEAD`, committer-date)
    //   - "date-order"   -> OrderingMode::DateOrderWalk  (== `git rev-list --date-order`)
    //   - "topo"         -> OrderingMode::Topo           (== `git rev-list --topo-order`)
    //   - "reverse"      -> Default order + reverse=true (== `git rev-list --reverse`)
    // and `first_parent=True` sets `RevListOptions::first_parent` (== `--first-parent`).
    // An unknown `order` value raises ValueError. `output_mode = OidOnly` because we only
    // need the oids — RevWalk reads+parses the commits itself.
    //
    // AIDEV-NOTE: We deliberately surface a SUBSET of grit-lib's ordering levers
    // (author-date variants `AuthorDateWalk`/`AuthorDateTopo` exist but are not exposed yet)
    // — these are the orderings that have a direct `git rev-list` flag oracle in our tests.
    #[pyo3(signature = (start, *, order=None, first_parent=false))]
    fn revwalk(
        &self,
        py: Python<'_>,
        start: &crate::objects::ObjectId,
        order: Option<&str>,
        first_parent: bool,
    ) -> PyResult<crate::revwalk::RevWalk> {
        use grit_lib::rev_list::{OrderingMode, OutputMode, RevListOptions};

        let mut options = RevListOptions {
            output_mode: OutputMode::OidOnly,
            first_parent,
            ..Default::default()
        };
        match order {
            None | Some("date") => options.ordering = OrderingMode::Default,
            Some("date-order") => options.ordering = OrderingMode::DateOrderWalk,
            Some("topo") => options.ordering = OrderingMode::Topo,
            Some("reverse") => {
                options.ordering = OrderingMode::Default;
                options.reverse = true;
            }
            Some(other) => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown order: {other:?} (expected one of: \
                     'date', 'date-order', 'topo', 'reverse')"
                )));
            }
        }

        let spec = start.inner().to_hex();
        let positive = vec![spec];
        let repo = Arc::clone(&self.inner);
        // `rev_list` takes `&Repository`; we deref the owned Arc clone inside the closure so
        // nothing borrows `self` across the allow_threads boundary.
        let result = py
            .allow_threads(|| grit_lib::rev_list::rev_list(&repo, &positive, &[], &options))
            .map_err(map_err)?;
        Ok(crate::revwalk::RevWalk::new(
            Arc::clone(&self.inner),
            result.commits,
        ))
    }

    // AIDEV-NOTE: `diff(a, b)` accepts COMMIT or TREE oids on either side: we peel each to a
    // tree oid (`tree_oid_of`) before calling grit's `diff_trees`. grit-lib's `diff_trees`
    // does NOT do rename detection by default — that lives in a SEPARATE function
    // (`grit_lib::diff::detect_renames`), so an unrelated delete+add stays as separate `D`/`A`
    // entries, matching `git diff --raw` WITHOUT `-M`. The read releases the GIL for the tree
    // walk + blob reads. The returned `Diff` OWNS its entries (Arc), so it outlives this handle.
    fn diff(
        &self,
        py: Python<'_>,
        a: &crate::objects::ObjectId,
        b: &crate::objects::ObjectId,
    ) -> PyResult<crate::diff::Diff> {
        let ta = self.tree_oid_of(py, a.inner())?;
        let tb = self.tree_oid_of(py, b.inner())?;
        let repo = Arc::clone(&self.inner);
        let entries = py
            .allow_threads(|| grit_lib::diff::diff_trees(&repo.odb, Some(&ta), Some(&tb), ""))
            .map_err(map_err)?;
        // AIDEV-NOTE: LAZINESS (FIX 5). We do NOT compute stats here — `Diff` carries the repo
        // Arc + entry oids and computes `DiffStats` on FIRST `.stats` access (and caches it).
        // So callers that only iterate statuses never pay for the blob reads.
        Ok(crate::diff::Diff::from_entries(
            Arc::clone(&self.inner),
            entries,
        ))
    }

    // AIDEV-NOTE: Create/move a ref. Three states (design §Ref safety): default overwrites;
    // create=True requires the ref be absent; expected_old=<oid> is compare-and-swap. create +
    // expected_old together is a usage error. The create-only and compare-and-swap paths are now
    // ATOMIC, going through a binding-held-lockfile protocol (crate::refs::atomic_cas_write); the
    // plain-overwrite path stays plumbing-faithful via grit's write_ref (which locks atomically).
    // The atomic path refuses reftable repos (loose-file CAS would be a silent lost update there).
    // Ref name must be UTF-8. When message= is given (with signer=), an old->new reflog entry is
    // appended after the write.
    #[pyo3(signature = (name, target, *, expected_old=None, create=false, message=None, signer=None))]
    #[allow(clippy::too_many_arguments)]
    fn update_ref(
        &self,
        py: Python<'_>,
        name: Vec<u8>,
        target: &crate::objects::ObjectId,
        expected_old: Option<crate::objects::ObjectId>,
        create: bool,
        message: Option<Vec<u8>>,
        signer: Option<PyRef<'_, crate::objects::Signature>>,
    ) -> PyResult<()> {
        if create && expected_old.is_some() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "pass create=True or expected_old=, not both",
            ));
        }
        let refname = validate_ref_name(&name)?;
        let reflog = reflog_args(message, signer.as_deref())?;
        let git_dir = self.inner.git_dir.clone();
        let new_oid = target.inner();

        let old_for_log = if create || expected_old.is_some() {
            // AIDEV-NOTE: Atomic path — create-only or compare-and-swap via a held lockfile.
            let exp = expected_old.as_ref().map(|o| o.inner());
            let prev = py
                .allow_threads(|| {
                    crate::refs::atomic_cas_write(
                        &git_dir,
                        &refname,
                        &new_oid,
                        exp.as_ref(),
                        create,
                    )
                })
                .map_err(crate::refs::cas_to_pyerr)?;
            prev.unwrap_or_else(|| crate::refs::zero_like(&new_oid))
        } else {
            // Plain overwrite (plumbing-faithful, unchanged): grit's write_ref locks atomically.
            let current = py.allow_threads(|| crate::refs::read_current_oid(&git_dir, &refname));
            py.allow_threads(|| grit_lib::refs::write_ref(&git_dir, &refname, &new_oid))
                .map_err(map_err)?;
            current.unwrap_or_else(|| crate::refs::zero_like(&new_oid))
        };
        if let Some((ident, msg)) = reflog {
            // force_create=true: message= is an explicit opt-in, so guarantee the entry rather
            // than depending on grit-lib's core.logAllRefUpdates auto-create default.
            py.allow_threads(|| {
                grit_lib::refs::append_reflog(
                    &git_dir,
                    &refname,
                    &old_for_log,
                    &new_oid,
                    &ident,
                    &msg,
                    true,
                )
            })
            .map_err(map_err)?;
        }
        Ok(())
    }

    // AIDEV-NOTE: Delete a ref. Default deletes unconditionally via grit's delete_ref
    // (plumbing-faithful). expected_old=<oid> is an ATOMIC compare-and-swap delete via the
    // binding-held-lockfile protocol (crate::refs::atomic_cas_delete), which refuses reftable
    // repos; its loose unlink is atomic, but a ref that ALSO has a packed-refs entry delegates the
    // packed removal to grit's delete_ref after the verify (a small documented residual window for
    // the packed case). When message=/signer= are given and the ref existed, an old->zero reflog
    // entry is appended before the delete.
    #[pyo3(signature = (name, *, expected_old=None, message=None, signer=None))]
    fn delete_ref(
        &self,
        py: Python<'_>,
        name: Vec<u8>,
        expected_old: Option<crate::objects::ObjectId>,
        message: Option<Vec<u8>>,
        signer: Option<PyRef<'_, crate::objects::Signature>>,
    ) -> PyResult<()> {
        let refname = validate_ref_name(&name)?;
        let git_dir = self.inner.git_dir.clone();
        let reflog = reflog_args(message, signer.as_deref())?;
        let current = py.allow_threads(|| crate::refs::read_current_oid(&git_dir, &refname));

        if let (Some((ident, msg)), Some(cur)) = (&reflog, &current) {
            let zero = crate::refs::zero_like(cur);
            // force_create=true: explicit opt-in via message= (see update_ref).
            py.allow_threads(|| {
                grit_lib::refs::append_reflog(&git_dir, &refname, cur, &zero, ident, msg, true)
            })
            .map_err(map_err)?;
        }

        // AIDEV-NOTE: CAS delete goes through the atomic held-lock path; an unconditional delete
        // uses grit's delete_ref directly. (The reflog-before-delete block above already ran when
        // requested.)
        match &expected_old {
            Some(exp) => {
                let exp_oid = exp.inner();
                py.allow_threads(|| crate::refs::atomic_cas_delete(&git_dir, &refname, &exp_oid))
                    .map_err(crate::refs::cas_to_pyerr)
            }
            None => py
                .allow_threads(|| grit_lib::refs::delete_ref(&git_dir, &refname))
                .map_err(map_err),
        }
    }

    // AIDEV-NOTE: Explicitly append a reflog entry: <old> <new> <identity>\t<message>. signer is
    // the full wire identity; message and identity must be UTF-8. force_create=True creates the
    // reflog file even if the repo would not auto-create it (e.g. for arbitrary refs).
    #[pyo3(signature = (name, old, new, *, signer, message, force_create=false))]
    #[allow(clippy::too_many_arguments)]
    fn append_reflog(
        &self,
        py: Python<'_>,
        name: Vec<u8>,
        old: &crate::objects::ObjectId,
        new: &crate::objects::ObjectId,
        signer: PyRef<'_, crate::objects::Signature>,
        message: Vec<u8>,
        force_create: bool,
    ) -> PyResult<()> {
        let refname = validate_ref_name(&name)?;
        let ident = utf8_field("signer", signer.wire_bytes())?;
        let msg = utf8_field("reflog message", message)?;
        reject_wire_control("reflog message", &msg)?;
        let git_dir = self.inner.git_dir.clone();
        let (old_oid, new_oid) = (old.inner(), new.inner());
        py.allow_threads(|| {
            grit_lib::refs::append_reflog(
                &git_dir,
                &refname,
                &old_oid,
                &new_oid,
                &ident,
                &msg,
                force_create,
            )
        })
        .map_err(map_err)
    }

    // AIDEV-NOTE: Point HEAD at a branch (symbolic ref). target is a ref name, e.g.
    // b"refs/heads/main". Must be a valid ref name. The hardcoded "HEAD" literal passed to
    // grit stays as-is — only the target (the ref name HEAD should point to) is validated.
    fn set_head(&self, py: Python<'_>, target: Vec<u8>) -> PyResult<()> {
        let target_str = validate_ref_name(&target)?;
        let git_dir = self.inner.git_dir.clone();
        py.allow_threads(|| grit_lib::refs::write_symbolic_ref(&git_dir, "HEAD", &target_str))
            .map_err(map_err)
    }

    // AIDEV-NOTE: Write an arbitrary symbolic ref (name -> target ref name). Both must be valid
    // ref names (validated via check_refname_format with allow_onelevel to permit HEAD-like names).
    fn set_symbolic_ref(&self, py: Python<'_>, name: Vec<u8>, target: Vec<u8>) -> PyResult<()> {
        let name_str = validate_ref_name(&name)?;
        let target_str = validate_ref_name(&target)?;
        let git_dir = self.inner.git_dir.clone();
        py.allow_threads(|| grit_lib::refs::write_symbolic_ref(&git_dir, &name_str, &target_str))
            .map_err(map_err)
    }

    // AIDEV-NOTE: Build an annotated-tag OBJECT and write it; returns its oid (== git mktag).
    // Pointing refs/tags/<name> at it is a separate update_ref. FIDELITY LIMITATION: grit-lib's
    // TagData stores tag/tagger/message as String only (no *_raw byte fields like CommitData),
    // so all three must be valid UTF-8 — non-UTF-8 raises ValueError (mirrors the read-side Tag
    // limitation). target_kind names the tagged object's type ("commit"/"tree"/"blob"/"tag").
    // tagger comes from a Signature or raw bytes, or is omitted (None) for a tagger-less tag.
    #[pyo3(signature = (target, target_kind, name, *, message, tagger=None, tagger_raw=None))]
    #[allow(clippy::too_many_arguments)]
    fn create_tag(
        &self,
        py: Python<'_>,
        target: &crate::objects::ObjectId,
        target_kind: &Bound<'_, PyAny>,
        name: Vec<u8>,
        message: Vec<u8>,
        tagger: Option<PyRef<'_, crate::objects::Signature>>,
        tagger_raw: Option<Vec<u8>>,
    ) -> PyResult<crate::objects::ObjectId> {
        let kind = crate::objects::py_to_kind(target_kind)?;
        let type_str = match kind {
            grit_lib::objects::ObjectKind::Commit => "commit",
            grit_lib::objects::ObjectKind::Tree => "tree",
            grit_lib::objects::ObjectKind::Blob => "blob",
            grit_lib::objects::ObjectKind::Tag => "tag",
        };
        // tagger is optional; when present it must resolve (Signature XOR raw) and be UTF-8.
        let tagger_str = match (tagger, tagger_raw) {
            (None, None) => None,
            (s, r) => {
                let sig_ref: Option<&crate::objects::Signature> = s.as_deref();
                let bytes = crate::objects::resolve_ident("tagger", sig_ref, r)?;
                Some(utf8_field("tagger", bytes)?)
            }
        };
        let tag_name_string = utf8_field("tag name", name)?;
        reject_wire_control("tag name", &tag_name_string)?;
        let tdata = grit_lib::objects::TagData {
            object: target.inner(),
            object_type: type_str.to_owned(),
            tag: tag_name_string,
            tagger: tagger_str,
            message: utf8_field("tag message", message)?,
        };
        let raw = grit_lib::objects::serialize_tag(&tdata);
        let oid = py
            .allow_threads(|| {
                self.inner
                    .odb
                    .write(grit_lib::objects::ObjectKind::Tag, &raw)
            })
            .map_err(map_err)?;
        Ok(crate::objects::ObjectId::from_inner(oid))
    }

    // AIDEV-NOTE: Build a commit object and write it (== git commit-tree). Pure: returns the new
    // oid and moves no ref. Identity comes from a Signature (formatted to wire bytes) or a raw
    // byte header (author_raw/committer_raw) — exactly one of each pair, enforced by
    // resolve_ident. We always populate CommitData.author_raw/committer_raw and raw_message so
    // serialize_commit emits our exact bytes (byte-identical OID to git). `encoding` (an ASCII
    // charset name) is optional. The serialize is cheap and runs under the GIL; the odb write
    // releases it.
    #[pyo3(signature = (tree, parents, *, message, author=None, committer=None,
                        author_raw=None, committer_raw=None, encoding=None))]
    #[allow(clippy::too_many_arguments)]
    fn create_commit(
        &self,
        py: Python<'_>,
        tree: &crate::objects::ObjectId,
        parents: Vec<crate::objects::ObjectId>,
        message: Vec<u8>,
        author: Option<PyRef<'_, crate::objects::Signature>>,
        committer: Option<PyRef<'_, crate::objects::Signature>>,
        author_raw: Option<Vec<u8>>,
        committer_raw: Option<Vec<u8>>,
        encoding: Option<String>,
    ) -> PyResult<crate::objects::ObjectId> {
        let author_bytes = crate::objects::resolve_ident("author", author.as_deref(), author_raw)?;
        let committer_bytes =
            crate::objects::resolve_ident("committer", committer.as_deref(), committer_raw)?;
        let parent_oids: Vec<grit_lib::objects::ObjectId> =
            parents.iter().map(|p| p.inner()).collect();

        let cdata = grit_lib::objects::CommitData {
            tree: tree.inner(),
            parents: parent_oids,
            author: String::new(),
            committer: String::new(),
            author_raw: author_bytes,
            committer_raw: committer_bytes,
            encoding,
            message: String::new(),
            raw_message: Some(message),
        };
        let raw = grit_lib::objects::serialize_commit(&cdata);
        let oid = py
            .allow_threads(|| {
                self.inner
                    .odb
                    .write(grit_lib::objects::ObjectKind::Commit, &raw)
            })
            .map_err(map_err)?;
        Ok(crate::objects::ObjectId::from_inner(oid))
    }

    // AIDEV-NOTE: Raw three-way tree merge (design §Merge). Produces a MergeResult (merged index +
    // conflict report); touches no ref and no work tree. favor: None|"ours"|"theirs"|"union".
    #[pyo3(signature = (base, ours, theirs, *, favor=None))]
    fn merge_trees(
        &self,
        py: Python<'_>,
        base: &crate::objects::ObjectId,
        ours: &crate::objects::ObjectId,
        theirs: &crate::objects::ObjectId,
        favor: Option<&str>,
    ) -> PyResult<crate::merge::MergeResult> {
        let fav = crate::merge::parse_favor(favor)?;
        let repo = Arc::clone(&self.inner);
        let (b, o, t) = (base.inner(), ours.inner(), theirs.inner());
        let output = py
            .allow_threads(|| {
                grit_lib::merge_trees::merge_trees_three_way(
                    &repo,
                    b,
                    o,
                    t,
                    fav,
                    grit_lib::merge_trees::WhitespaceMergeOptions::default(),
                    None,
                    grit_lib::merge_trees::TreeMergeConflictPresentation::default(),
                )
            })
            .map_err(map_err)?;
        crate::merge::MergeResult::from_output(py, Arc::clone(&self.inner), output)
    }

    // AIDEV-NOTE: Commit-level three-way merge. base = first merge_base(ours, theirs); when the two
    // are unrelated (no base), use the empty tree as base (additive merge, git's
    // --allow-unrelated-histories behaviour). Returns a MergeResult exactly like merge_trees.
    #[pyo3(signature = (ours, theirs, *, favor=None))]
    fn merge_commits(
        &self,
        py: Python<'_>,
        ours: &crate::objects::ObjectId,
        theirs: &crate::objects::ObjectId,
        favor: Option<&str>,
    ) -> PyResult<crate::merge::MergeResult> {
        let fav = crate::merge::parse_favor(favor)?;
        let repo = Arc::clone(&self.inner);
        let (o, t) = (ours.inner(), theirs.inner());
        let output = py
            .allow_threads(|| -> Result<_, grit_lib::error::Error> {
                let bases = grit_lib::merge_base::merge_bases_all(&repo, &[o, t])?;
                let base_tree = match bases.into_iter().next() {
                    Some(base_commit) => crate::merge::tree_of_commit(&repo, base_commit)?,
                    None => repo.odb.write(
                        grit_lib::objects::ObjectKind::Tree,
                        &grit_lib::objects::serialize_tree(&[]),
                    )?,
                };
                let ours_tree = crate::merge::tree_of_commit(&repo, o)?;
                let theirs_tree = crate::merge::tree_of_commit(&repo, t)?;
                grit_lib::merge_trees::merge_trees_three_way(
                    &repo,
                    base_tree,
                    ours_tree,
                    theirs_tree,
                    fav,
                    grit_lib::merge_trees::WhitespaceMergeOptions::default(),
                    None,
                    grit_lib::merge_trees::TreeMergeConflictPresentation::default(),
                )
            })
            .map_err(map_err)?;
        crate::merge::MergeResult::from_output(py, Arc::clone(&self.inner), output)
    }

    // AIDEV-NOTE: First merge base of two commits (== `git merge-base`), or None if unrelated.
    // Phase B uses the FIRST base only (no recursive/virtual base for criss-cross histories;
    // documented limitation). grit's merge_bases_all returns all bases; we take the first.
    fn merge_base(
        &self,
        py: Python<'_>,
        a: &crate::objects::ObjectId,
        b: &crate::objects::ObjectId,
    ) -> PyResult<Option<crate::objects::ObjectId>> {
        let repo = Arc::clone(&self.inner);
        let (ao, bo) = (a.inner(), b.inner());
        let bases = py
            .allow_threads(|| grit_lib::merge_base::merge_bases_all(&repo, &[ao, bo]))
            .map_err(map_err)?;
        Ok(bases
            .into_iter()
            .next()
            .map(crate::objects::ObjectId::from_inner))
    }

    // AIDEV-NOTE: Low-level single-file working-tree write (escape hatch under checkout_tree).
    // Wraps porcelain::checkout::write_to_worktree, which ALWAYS overwrites and natively handles
    // symlinks (mode 0o120000) and the exec bit (mode 0o100755). Requires a non-bare repo with a
    // work tree; rel_path must be a clean relative path (validate_index_path) and UTF-8 (grit's
    // primitive takes &str).
    fn write_to_worktree(
        &self,
        py: Python<'_>,
        rel_path: Vec<u8>,
        data: Vec<u8>,
        mode: u32,
    ) -> PyResult<()> {
        crate::index::validate_index_path(&rel_path)?;
        let rel = std::str::from_utf8(&rel_path)
            .map_err(|_| pyo3::exceptions::PyValueError::new_err("rel_path must be valid UTF-8"))?;
        let work_tree = self.inner.work_tree.clone().ok_or_else(|| {
            crate::error::invalid_ref("cannot write to a bare repository (no work tree)")
        })?;
        let rel_owned = rel.to_owned();
        py.allow_threads(|| {
            grit_lib::porcelain::checkout::write_to_worktree(&work_tree, &rel_owned, &data, mode)
        })
        .map_err(map_err)
    }

    // AIDEV-NOTE: Commit-and-advance-branch porcelain (== `git commit`). Resolves HEAD's branch
    // (must be symbolic — detached HEAD raises), writes a tree from the repo index, builds a commit
    // whose FIRST parent is the current branch tip (none if the branch is unborn) plus any `parents=`
    // extras (merge commits), writes it, atomically advances the branch (create-only if unborn, CAS
    // on the tip otherwise), and appends a `commit:`/`commit (initial):` reflog entry with the
    // committer identity. Identity rules match create_commit (Signature XOR *_raw).
    #[pyo3(signature = (*, message, parents=None, author=None, committer=None,
                        author_raw=None, committer_raw=None, encoding=None))]
    #[allow(clippy::too_many_arguments)]
    fn commit_index(
        &self,
        py: Python<'_>,
        message: Vec<u8>,
        parents: Option<Vec<crate::objects::ObjectId>>,
        author: Option<PyRef<'_, crate::objects::Signature>>,
        committer: Option<PyRef<'_, crate::objects::Signature>>,
        author_raw: Option<Vec<u8>>,
        committer_raw: Option<Vec<u8>>,
        encoding: Option<String>,
    ) -> PyResult<crate::objects::ObjectId> {
        let author_bytes = crate::objects::resolve_ident("author", author.as_deref(), author_raw)?;
        let committer_bytes =
            crate::objects::resolve_ident("committer", committer.as_deref(), committer_raw)?;
        let git_dir = self.inner.git_dir.clone();
        let repo = Arc::clone(&self.inner);

        // Resolve HEAD -> branch (symbolic required).
        let branch = py
            .allow_threads(|| grit_lib::refs::read_head(&git_dir))
            .map_err(map_err)?
            .ok_or_else(|| {
                crate::error::invalid_ref(
                    "HEAD is detached; commit_index requires HEAD on a branch",
                )
            })?;

        // Current branch tip (None if unborn).
        let tip = py
            .allow_threads(
                || -> Result<Option<grit_lib::objects::ObjectId>, grit_lib::error::Error> {
                    match grit_lib::refs::read_raw_ref(&git_dir, &branch)? {
                        grit_lib::refs::RawRefLookup::NotFound => Ok(None),
                        _ => grit_lib::refs::resolve_ref(&git_dir, &branch).map(Some),
                    }
                },
            )
            .map_err(map_err)?;

        // Tree from the repo index.
        let tree = py
            .allow_threads(
                || -> Result<grit_lib::objects::ObjectId, grit_lib::error::Error> {
                    let index = repo.load_index()?;
                    grit_lib::write_tree::write_tree_from_index(&repo.odb, &index, "")
                },
            )
            .map_err(map_err)?;

        // Parents: branch tip first (if any), then caller extras.
        let mut parent_oids: Vec<grit_lib::objects::ObjectId> = Vec::new();
        if let Some(t) = &tip {
            parent_oids.push(*t);
        }
        if let Some(extra) = &parents {
            parent_oids.extend(extra.iter().map(|p| p.inner()));
        }

        // Reflog subject is the first line of the message; compute it before `message` moves
        // into CommitData below.
        let subject = first_line(&message);

        let cdata = grit_lib::objects::CommitData {
            tree,
            parents: parent_oids,
            author: String::new(),
            committer: String::new(),
            author_raw: author_bytes,
            committer_raw: committer_bytes.clone(),
            encoding,
            message: String::new(),
            raw_message: Some(message),
        };
        let raw = grit_lib::objects::serialize_commit(&cdata);
        let new_oid = py
            .allow_threads(|| repo.odb.write(grit_lib::objects::ObjectKind::Commit, &raw))
            .map_err(map_err)?;

        // Atomically advance the branch: create-only if unborn, else CAS on the tip we read.
        let exp = tip;
        let create = exp.is_none();
        py.allow_threads(|| {
            crate::refs::atomic_cas_write(&git_dir, &branch, &new_oid, exp.as_ref(), create)
        })
        .map_err(crate::refs::cas_to_pyerr)?;

        // Reflog: porcelain always logs (force_create). "commit (initial): <subject>" when unborn.
        let prefix = if exp.is_none() {
            "commit (initial): "
        } else {
            "commit: "
        };
        let log_msg = format!("{prefix}{subject}");
        let ident = utf8_field("committer", committer_bytes)?;
        let old_for_log = exp.unwrap_or_else(|| crate::refs::zero_like(&new_oid));
        py.allow_threads(|| {
            grit_lib::refs::append_reflog(
                &git_dir,
                &branch,
                &old_for_log,
                &new_oid,
                &ident,
                &log_msg,
                true,
            )
        })
        .map_err(map_err)?;

        Ok(crate::objects::ObjectId::from_inner(new_oid))
    }

    // AIDEV-NOTE: Non-destructive overlay checkout (design §Checkout safety). Writes the tree's
    // blobs into the work tree, never deletes files absent from the tree, and refuses to overwrite
    // an existing path unless force=True. update_index=True rebuilds matching index entries.
    #[pyo3(signature = (tree, *, force=false, update_index=true))]
    fn checkout_tree(
        &self,
        py: Python<'_>,
        tree: &crate::objects::ObjectId,
        force: bool,
        update_index: bool,
    ) -> PyResult<()> {
        crate::checkout::checkout_tree_method(&self.inner, py, tree, force, update_index)
    }

    // AIDEV-NOTE: Fetch from `url` into this repo (== `git fetch`). Default refspec fetches all
    // heads into refs/remotes/origin/*. git:// applies the returned ref updates here; https
    // (http_fetch) self-applies. Progress is unconditionally NoProgress (grit-lib 0.4.1 forces
    // no-progress; progress= param dropped to avoid a misleading dead knob).
    #[pyo3(signature = (url, refspecs=None, *, tags="following", prune=false,
                        username=None, password=None, use_credential_helpers=true))]
    #[allow(clippy::too_many_arguments)]
    fn fetch(
        &self,
        py: Python<'_>,
        url: String,
        refspecs: Option<Vec<String>>,
        tags: &str,
        prune: bool,
        username: Option<String>,
        password: Option<String>,
        use_credential_helpers: bool,
    ) -> PyResult<crate::remote::FetchReport> {
        crate::remote::fetch_method(
            py,
            &self.inner,
            url,
            refspecs,
            tags,
            prune,
            username,
            password,
            use_credential_helpers,
        )
    }

    // AIDEV-NOTE: Lightweight tag = a plain ref refs/tags/<name> -> target oid. Atomic create-only by
    // default (force=False); force=True overwrites (moves the tag). No tag object is created. Uses the
    // held-lock atomic_cas_write: create_only=!force gives create-only when force is false, and an
    // unconditional atomic overwrite when force is true.
    #[pyo3(signature = (name, target, *, force=false))]
    fn create_lightweight_tag(
        &self,
        py: Python<'_>,
        name: Vec<u8>,
        target: &crate::objects::ObjectId,
        force: bool,
    ) -> PyResult<()> {
        let refname = tag_refname(&name)?;
        let git_dir = self.inner.git_dir.clone();
        let new_oid = target.inner();
        py.allow_threads(|| {
            crate::refs::atomic_cas_write(&git_dir, &refname, &new_oid, None, !force)
        })
        .map_err(crate::refs::cas_to_pyerr)?;
        Ok(())
    }

    // AIDEV-NOTE: Annotated tag = create the tag OBJECT (via create_tag) then point refs/tags/<name>
    // at it. Atomic create-only unless force=True. Returns the tag-object oid. We validate the ref
    // name up front so a bad tag name fails before the object is written.
    #[pyo3(signature = (name, target, target_kind, *, message, tagger=None, tagger_raw=None, force=false))]
    #[allow(clippy::too_many_arguments)]
    fn create_annotated_tag(
        &self,
        py: Python<'_>,
        name: Vec<u8>,
        target: &crate::objects::ObjectId,
        target_kind: &Bound<'_, PyAny>,
        message: Vec<u8>,
        tagger: Option<PyRef<'_, crate::objects::Signature>>,
        tagger_raw: Option<Vec<u8>>,
        force: bool,
    ) -> PyResult<crate::objects::ObjectId> {
        let refname = tag_refname(&name)?;
        let tag_oid =
            self.create_tag(py, target, target_kind, name, message, tagger, tagger_raw)?;
        let git_dir = self.inner.git_dir.clone();
        let oid = tag_oid.inner();
        py.allow_threads(|| crate::refs::atomic_cas_write(&git_dir, &refname, &oid, None, !force))
            .map_err(crate::refs::cas_to_pyerr)?;
        Ok(tag_oid)
    }
}

impl Repository {
    // AIDEV-NOTE: Peel an object id to its TREE oid so `diff` works for both commit and tree
    // inputs. Read the object; if it is a Commit, parse it and take its `.tree`; if it is
    // already a Tree, use the oid as-is; anything else (blob/tag) is an InvalidObjectError.
    // The odb read releases the GIL; parse_commit runs under the GIL (it touches Python only
    // when building Signatures, which we don't here — we just read CommitData.tree).
    fn tree_oid_of(
        &self,
        py: Python<'_>,
        oid: grit_lib::objects::ObjectId,
    ) -> PyResult<grit_lib::objects::ObjectId> {
        let obj = py
            .allow_threads(|| self.inner.odb.read(&oid))
            .map_err(map_err)?;
        match obj.kind {
            grit_lib::objects::ObjectKind::Tree => Ok(oid),
            grit_lib::objects::ObjectKind::Commit => {
                let c = grit_lib::objects::parse_commit(&obj.data).map_err(map_err)?;
                Ok(c.tree)
            }
            other => Err(crate::error::InvalidObjectError::new_err(format!(
                "object {} is a {}, cannot diff (expected a commit or tree)",
                oid.to_hex(),
                other
            ))),
        }
    }

    // AIDEV-NOTE: DIFFSTAT COMPUTATION. Compute a `git --numstat`-style summary from a tree
    // diff. grit-lib's `diffstat` module only LAYS OUT a stat block from pre-computed per-file
    // insertion/deletion counts — it does NOT derive them from a tree diff. So we re-read each
    // changed entry's old/new blobs here and count line changes the way Git's `--numstat` does:
    //   - files_changed = number of diff entries (matching git's per-file row count).
    //   - For each entry, read the old blob (empty if the old oid is zero/absent → Added) and
    //     the new blob (empty if absent → Deleted). If EITHER side is binary (contains a NUL,
    //     per grit_lib::merge_file::is_binary, == Git's heuristic for `--numstat`'s `-`), the
    //     file contributes 0 insertions/0 deletions (git prints `-`/`-`, not counted).
    //   - Otherwise count via grit_lib::diff::count_changes (similar's Myers), decoding bytes
    //     losslessly (latin-1-style 1:1) so the line counts are unaffected by encoding.
    //
    // AIDEV-NOTE: ERROR & GITLINK HANDLING (FIX 4). A real ODB read FAILURE now PROPAGATES as
    // a PyErr (the getter raises) instead of being swallowed as empty content, so a corrupt or
    // missing object can no longer yield silently-wrong stats. The zero (null) oid still maps to
    // empty content (the absent side of an Add/Delete — that is correct, not an error).
    //
    // GITLINK (submodule, mode 160000) handling: a gitlink entry references a COMMIT object, not
    // a blob. We must NOT read that commit and line-count its raw object bytes (tree/author/
    // committer headers) as if it were file content — that was the bug. EMPIRICAL NOTE: the task
    // brief asserted `git --numstat` does NOT count submodule pointer changes and said to treat
    // gitlinks as 0/0. That is NOT what git actually does (verified against git 2.53.0): git
    // renders a gitlink side as the single text line `Subproject commit <oid>`, so `--numstat`
    // reports add=1/0, modify=1/1, delete=0/1. To stay faithful to the `git --numstat` oracle
    // the whole suite uses, we synthesize exactly that line for a non-blob side (instead of
    // line-counting the commit object's bytes) — matching git for add/modify/delete gitlinks.
    // read_blob_bytes returns this synthesized text (NOT the raw commit bytes) for a non-blob.
    //
    // AIDEV-NOTE: --numstat PARITY LIMITATION (bare CR). For normal `\n`-terminated text the
    // counts match `git --numstat` exactly (verified in tests/test_diff.py). They can DIVERGE,
    // however, for files that contain a bare `\r` (CR not part of a CRLF) as CONTENT: grit's
    // `count_changes` delegates to `similar`'s line tokenizer, which treats `\r`, `\r\n`, AND
    // `\n` as line breaks, whereas `git --numstat` splits on `\n` ONLY (a bare `\r` is line
    // content there, not a terminator). So e.g. `a\rb\n` -> `a\rb\rc\rd\n` counts as ins=3/del=1
    // here but ins=1/del=1 in git. We accept this (the `count_changes` path is otherwise
    // verified-correct); exact parity would require splitting on `\n` only and diffing those
    // segments ourselves. See the xfail in tests/test_diff.py that encodes the divergence.
    // AIDEV-NOTE: LAZINESS (FIX 5). This is a `pub(crate)` FREE function (not a method) taking
    // the raw old/new oid pairs + the file count, so `Diff::stats` (src/diff.rs) can call it
    // on FIRST `.stats` access — `diff()` no longer computes stats eagerly. The caller owns the
    // GIL-release (allow_threads) around the call. `files_changed` is passed in (the number of
    // diff entries) since this function no longer sees the entries themselves.
    pub(crate) fn compute_diff_stats(
        odb: &grit_lib::odb::Odb,
        oid_pairs: &[(grit_lib::objects::ObjectId, grit_lib::objects::ObjectId)],
        files_changed: usize,
    ) -> PyResult<crate::diff::DiffStats> {
        let mut insertions = 0usize;
        let mut deletions = 0usize;

        for (old_oid, new_oid) in oid_pairs {
            // Read each side's content. A read error propagates (raises) instead of lying about
            // counts; a non-blob (gitlink) side yields the synthesized `Subproject commit <oid>`
            // line so it counts like `git --numstat` (see read_blob_bytes).
            let old_bytes = read_blob_bytes(odb, old_oid)?;
            let new_bytes = read_blob_bytes(odb, new_oid)?;

            if grit_lib::merge_file::is_binary(&old_bytes)
                || grit_lib::merge_file::is_binary(&new_bytes)
            {
                // Binary file: git --numstat prints `-`/`-`; not counted as line changes.
                continue;
            }

            let old_text = bytes_to_lossy_string(&old_bytes);
            let new_text = bytes_to_lossy_string(&new_bytes);
            let (ins, del) = grit_lib::diff::count_changes(&old_text, &new_text);
            insertions += ins;
            deletions += del;
        }

        Ok(crate::diff::DiffStats::new(
            files_changed,
            insertions,
            deletions,
        ))
    }
}

// AIDEV-NOTE: Read one diff side's content bytes for stat counting (FIX 4). Returns:
//   - Ok(empty)       for a ZERO (null) oid — the absent side of an Add/Delete (correct, not an
//     error): the other side's content drives the count.
//   - Ok(blob bytes)  for a real BLOB — its content, to be line-counted.
//   - Ok(gitlink text) for a NON-BLOB (a submodule GITLINK references a COMMIT object): we do
//     NOT line-count the commit object's raw bytes (that was the bug). Instead we synthesize the
//     single line `Subproject commit <hex>\n` that git renders for a submodule side, so the
//     diffstat matches `git --numstat` (gitlink add=1/0, modify=1/1, delete=0/1).
//   - Err(..)         if the ODB read FAILS — propagated so `.stats` RAISES rather than returning
//     silently-wrong counts (a corrupt/missing object must not be swallowed as empty).
pub(crate) fn read_blob_bytes(
    odb: &grit_lib::odb::Odb,
    oid: &grit_lib::objects::ObjectId,
) -> PyResult<Vec<u8>> {
    if oid.is_zero() {
        return Ok(Vec::new());
    }
    let obj = odb.read(oid).map_err(map_err)?;
    if obj.kind != grit_lib::objects::ObjectKind::Blob {
        // Gitlink (commit) or any other non-blob: render git's one-line submodule text.
        return Ok(format!("Subproject commit {}\n", oid.to_hex()).into_bytes());
    }
    Ok(obj.data)
}

// AIDEV-NOTE: Build and validate refs/tags/<name>. Reuses validate_ref_name (UTF-8 + git ref
// format), so e.g. a name with spaces or ".." is rejected before any write.
fn tag_refname(name: &[u8]) -> PyResult<String> {
    let mut full = b"refs/tags/".to_vec();
    full.extend_from_slice(name);
    validate_ref_name(&full)
}

// AIDEV-NOTE: Validate a ref name before handing it to grit-lib, which joins it to the git dir
// unchecked (an absolute or '..' name would escape). allow_onelevel=true so one-level pseudorefs
// like HEAD are accepted. Returns the validated UTF-8 name; RepositoryError on a malformed name.
fn validate_ref_name(name: &[u8]) -> PyResult<String> {
    let s =
        std::str::from_utf8(name).map_err(|_| crate::error::invalid_ref("non-UTF-8 ref name"))?;
    let opts = grit_lib::check_ref_format::RefNameOptions {
        allow_onelevel: true,
        refspec_pattern: false,
        normalize: false,
    };
    grit_lib::check_ref_format::check_refname_format(s, &opts)
        .map_err(|e| crate::error::invalid_ref(&format!("invalid ref name: {s:?}: {e}")))
}

// AIDEV-NOTE: Reject bytes that would corrupt a single-line wire record (reflog entry) or an
// object header line (tag name). NUL/LF/CR are the structural delimiters.
fn reject_wire_control(what: &str, s: &str) -> PyResult<()> {
    if s.bytes().any(|b| matches!(b, 0 | b'\n' | b'\r')) {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "{what} must not contain NUL, newline, or carriage return"
        )));
    }
    Ok(())
}

// AIDEV-NOTE: Resolve the optional reflog request for a ref op. Returns Some((identity, message))
// only when a message is given; a message without a signer is a usage error. append_reflog wants
// the full wire identity ("Name <email> <unix> <+HHMM>") and a UTF-8 message. signer.wire_bytes()
// is UTF-8 for normal identities; non-UTF-8 signer/message raise ValueError.
fn reflog_args(
    message: Option<Vec<u8>>,
    signer: Option<&crate::objects::Signature>,
) -> PyResult<Option<(String, String)>> {
    match message {
        None => Ok(None),
        Some(msg) => {
            let signer = signer.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err("message= requires signer=")
            })?;
            let ident = utf8_field("signer", signer.wire_bytes())?;
            let msg = utf8_field("reflog message", msg)?;
            reject_wire_control("reflog message", &msg)?;
            Ok(Some((ident, msg)))
        }
    }
}

// AIDEV-NOTE: First line of a commit message (the reflog subject). UTF-8-lossy and single-line by
// construction (we cut at the first '\n'), so it is safe as a one-line reflog record.
fn first_line(msg: &[u8]) -> String {
    let end = msg.iter().position(|&b| b == b'\n').unwrap_or(msg.len());
    String::from_utf8_lossy(&msg[..end]).into_owned()
}

// AIDEV-NOTE: grit-lib's TagData fields are `String`, so tag name/tagger/message must be UTF-8.
// Convert here and raise ValueError (not a silent lossy decode) on non-UTF-8 input.
fn utf8_field(what: &str, bytes: Vec<u8>) -> PyResult<String> {
    String::from_utf8(bytes)
        .map_err(|_| pyo3::exceptions::PyValueError::new_err(format!("{what} must be valid UTF-8")))
}

// AIDEV-NOTE: Map raw bytes to a String 1:1 (each byte -> its U+00xx code point, latin-1
// style). This is lossless and order-preserving, so `count_changes` over the resulting `&str`
// yields the same insertion/deletion counts as operating on the raw bytes. We do this only
// AFTER the binary check, so text files are well-behaved. NOTE on line breaks: `count_changes`
// (via `similar`) tokenizes lines on `\r`, `\r\n`, AND `\n`, NOT on `\n` only like `git
// --numstat`. The 1:1 byte mapping preserves every byte (including any `\r`), so it does not
// itself cause divergence — but the differing line-break set means bare-`\r`-as-content files
// can still count differently from git (see the parity-limitation note on compute_diff_stats).
fn bytes_to_lossy_string(data: &[u8]) -> String {
    data.iter().map(|&b| b as char).collect()
}
