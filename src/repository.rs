//! Python wrapper over `grit_lib::repo::Repository`.

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::error::map_err;

// AIDEV-NOTE: Accept str | bytes | os.PathLike path inputs (design §5). On Unix,
// bytes map to an OsString 1:1 (exact non-UTF-8 path fidelity); str/os.PathLike go
// through PyO3's PathBuf extractor (surrogateescape via fsdecode). We try the
// PathBuf extractor first so os.PathLike (and str) take the standard path, and fall
// back to raw bytes only for `bytes` inputs. This touches Python, so callers MUST
// run it BEFORE releasing the GIL with allow_threads.
fn extract_path(obj: &Bound<'_, PyAny>) -> PyResult<std::path::PathBuf> {
    if let Ok(p) = obj.extract::<std::path::PathBuf>() {
        return Ok(p);
    }
    if let Ok(b) = obj.extract::<Vec<u8>>() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            return Ok(std::path::PathBuf::from(std::ffi::OsString::from_vec(b)));
        }
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "path must be str, bytes, or os.PathLike",
    ))
}

// AIDEV-NOTE: We hold an `Arc<grit_lib::repo::Repository>` so the `.odb` accessor can
// hand out an `Odb` that clones the Arc and outlives this Python `Repository` handle
// (design §6: a child Odb keeps the repo alive). grit-lib exposes git_dir/work_tree/odb
// as PUBLIC FIELDS (no getter methods); is_bare() is the only method here.
#[pyclass(module = "pygrit._pygrit")]
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

    // AIDEV-NOTE: Read any object then `parse_commit` over its bytes. A non-commit oid
    // parses-fail → InvalidObjectError (acceptable: the caller asked for a commit). The
    // odb read releases the GIL; parse_commit runs under the GIL (it touches Python only
    // when building Signatures). `oid.inner()` is an owned Copy, so it moves into the
    // closure with no lifetime tie to `oid`.
    fn commit(
        &self,
        py: Python<'_>,
        oid: &crate::objects::ObjectId,
    ) -> PyResult<crate::objects::Commit> {
        let want = oid.inner();
        let data = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?
            .data;
        crate::objects::Commit::from_bytes(py, oid.clone(), &data)
    }

    // AIDEV-NOTE: Read any object then `parse_tree` over its bytes. A non-tree oid
    // parses-fail → InvalidObjectError (acceptable: the caller asked for a tree). Same
    // GIL/lifetime pattern as `commit`. The returned `Tree` OWNS its entries (Arc), so it
    // outlives this Repository handle.
    fn tree(
        &self,
        py: Python<'_>,
        oid: &crate::objects::ObjectId,
    ) -> PyResult<crate::objects::Tree> {
        let want = oid.inner();
        let data = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?
            .data;
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

    // AIDEV-NOTE: Read any object then `parse_tag` over its bytes. A non-tag (or non-UTF-8)
    // object parses-fail → InvalidObjectError. Same GIL/lifetime pattern as `commit`.
    fn tag(&self, py: Python<'_>, oid: &crate::objects::ObjectId) -> PyResult<crate::objects::Tag> {
        let want = oid.inner();
        let data = py
            .allow_threads(|| self.inner.odb.read(&want))
            .map_err(map_err)?
            .data;
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
        let stats = py.allow_threads(|| Self::compute_diff_stats(&repo.odb, &entries));
        Ok(crate::diff::Diff::from_entries(entries, stats))
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
    // A blob read failure is treated as an empty side (best-effort; a missing/absent oid).
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
    fn compute_diff_stats(
        odb: &grit_lib::odb::Odb,
        entries: &[grit_lib::diff::DiffEntry],
    ) -> crate::diff::DiffStats {
        let mut insertions = 0usize;
        let mut deletions = 0usize;

        for e in entries {
            let old_bytes = read_blob_bytes(odb, &e.old_oid);
            let new_bytes = read_blob_bytes(odb, &e.new_oid);

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

        crate::diff::DiffStats::new(entries.len(), insertions, deletions)
    }
}

// AIDEV-NOTE: Read a blob's bytes for stat counting. A zero (null) oid means the side is
// absent (Added has zero old_oid, Deleted has zero new_oid) → empty content. A read failure
// is also treated as empty (best-effort numstat). We do NOT verify kind == Blob: diff entries
// only reference blobs on the file sides, and treating a non-blob as its raw bytes would still
// be harmless for line counting (it never happens for tree-to-tree file diffs).
fn read_blob_bytes(odb: &grit_lib::odb::Odb, oid: &grit_lib::objects::ObjectId) -> Vec<u8> {
    if oid.is_zero() {
        return Vec::new();
    }
    match odb.read(oid) {
        Ok(obj) => obj.data,
        Err(_) => Vec::new(),
    }
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
