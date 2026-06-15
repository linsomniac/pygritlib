//! Python wrappers over grit-lib object-model primitives (`ObjectId`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use pyo3::basic::CompareOp;
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::PyBytes;

use crate::error::map_err;

// AIDEV-NOTE: We wrap grit-lib's own `ObjectId` (which derives
// Clone/Copy/Eq/Ord/Hash and provides to_hex/as_bytes/from_hex/from_bytes/algo)
// rather than reimplementing hex parsing — grit-lib owns the canonical SHA-1/256
// width logic. `frozen` makes the Python object immutable, matching the Copy oid.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
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

// AIDEV-NOTE: `inner()` is used by the odb read/exists bindings (task 2.6); `from_inner`
// is now consumed by `Commit` (tree/parents) in task 2.7. Both have callers, so no
// dead-code allow is needed.
impl ObjectId {
    pub fn from_inner(inner: grit_lib::objects::ObjectId) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> grit_lib::objects::ObjectId {
        self.inner
    }
}

// AIDEV-NOTE: Decode bytes using Python's own codec machinery (full encoding + errors
// support: utf-8/latin-1/.../strict/replace/surrogateescape) rather than reimplementing
// codecs in Rust. Shared by Signature.name_str/email_str and Commit.message().
//
// We RETURN THE PYTHON str OBJECT directly (a `Bound<PyAny>`) rather than round-tripping
// through a Rust `String`. This is essential for non-strict error handlers: with
// errors="surrogateescape" (or "replace" on data yielding lone surrogates), Python returns
// a str containing UNPAIRED SURROGATES, which a Rust `String` CANNOT hold — so an
// `.extract::<String>()` here would raise, defeating the `errors=` parameter. By handing the
// Python str straight back, surrogate-escaped/replacement strings flow through intact.
fn decode_bytes<'py>(
    py: Python<'py>,
    data: &[u8],
    encoding: &str,
    errors: &str,
) -> PyResult<Bound<'py, PyAny>> {
    PyBytes::new(py, data).call_method1("decode", (encoding, errors))
}

// AIDEV-NOTE: grit-lib has NO Signature struct — author/committer are raw Git-wire idents
// (`Name <email> <unix-seconds> <+HHMM>`). This binding-layer type splits name/email from
// the RAW header bytes (preserving non-UTF-8 fidelity, design §5) and derives the time via
// grit_lib::ident::parse_signature_times on the decoded String form.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct Signature {
    name: Vec<u8>,
    email: Vec<u8>,
    when_secs: i64,
    when_offset_secs: i32,
}

#[pymethods]
impl Signature {
    /// The identity name as raw bytes (non-UTF-8 fidelity; design §5).
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }

    /// The identity email as raw bytes.
    #[getter]
    fn email<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.email)
    }

    /// `(unix_seconds, utc_offset_seconds)`. Offset is signed (e.g. `+0530` → `19800`).
    #[getter]
    fn when(&self) -> (i64, i32) {
        (self.when_secs, self.when_offset_secs)
    }

    /// The name decoded as UTF-8 (strict). Raises `UnicodeDecodeError` on non-UTF-8.
    // AIDEV-NOTE: `py` is PyO3-injected (NOT part of the Python-visible signature), so the
    // stub stays `-> str`. We return the decoded Python str object (see decode_bytes).
    #[getter]
    fn name_str<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        decode_bytes(py, &self.name, "utf-8", "strict")
    }

    /// The email decoded as UTF-8 (strict). Raises `UnicodeDecodeError` on non-UTF-8.
    #[getter]
    fn email_str<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        decode_bytes(py, &self.email, "utf-8", "strict")
    }
}

impl Signature {
    // AIDEV-NOTE: Git ident wire format is `Name <email> <unix-seconds> <+HHMM>`. We split
    // name/email from the RAW bytes for non-UTF-8 fidelity; the time comes from
    // grit_lib::ident::parse_signature_times on the decoded String form (it parses the
    // trailing `<unix> <+HHMM>`, returning tz_offset_secs ALREADY in seconds). We use the
    // LAST `<`/`>` pair so a literal `<` inside a name does not fool the split. If the time
    // parse returns None (corrupt/missing/overflow date), we fall back to (0, 0) — a
    // non-fatal read of a malformed signature, matching Git's sentinel handling.
    pub fn parse(raw: &[u8], ident_str: &str) -> Self {
        let (name, email) = split_name_email(raw);
        let (when_secs, when_offset_secs) = match grit_lib::ident::parse_signature_times(ident_str)
        {
            Some(t) => (t.unix_seconds, t.tz_offset_secs as i32),
            None => (0, 0),
        };
        Self {
            name,
            email,
            when_secs,
            when_offset_secs,
        }
    }
}

// AIDEV-NOTE: Split `Name <email> ...` from raw ident bytes. We locate the LAST `<` and the
// FIRST `>` at-or-after it (robust to a literal `<` inside a name). name = bytes before that
// `<` with exactly one trailing space trimmed; email = bytes strictly between `<` and `>`.
// On a malformed ident with no `<`/`>` pair, name = full input, email = empty.
fn split_name_email(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    if let Some(lt) = raw.iter().rposition(|&b| b == b'<') {
        if let Some(rel_gt) = raw[lt + 1..].iter().position(|&b| b == b'>') {
            let gt = lt + 1 + rel_gt;
            let mut name_end = lt;
            if name_end > 0 && raw[name_end - 1] == b' ' {
                name_end -= 1;
            }
            let name = raw[..name_end].to_vec();
            let email = raw[lt + 1..gt].to_vec();
            return (name, email);
        }
    }
    (raw.to_vec(), Vec::new())
}

// AIDEV-NOTE: `Commit` is a binding-layer typed view over grit_lib::objects::parse_commit.
// `frozen` (immutable). author/committer are wrapped Py<Signature>; message is the EXACT
// raw body bytes (see from_bytes). tree/parents are pylibgrit ObjectIds.
//
// AIDEV-NOTE: `id` makes a Commit self-describing — required because `revwalk` (Phase 4)
// yields bare `Commit` objects (no surrounding oid), so each must carry its own id. The
// caller of `from_bytes` supplies it (parse_commit does NOT compute the object's own oid).
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct Commit {
    id: ObjectId,
    tree: ObjectId,
    parents: Vec<ObjectId>,
    author: Py<Signature>,
    committer: Py<Signature>,
    message: Vec<u8>,
}

#[pymethods]
impl Commit {
    /// This commit's own object id.
    #[getter]
    fn id(&self) -> ObjectId {
        self.id.clone()
    }

    /// The tree this commit points to.
    #[getter]
    fn tree(&self) -> ObjectId {
        self.tree.clone()
    }

    /// Parent commit ids (empty for a root commit, 1 normally, 2+ for merges).
    #[getter]
    fn parents(&self) -> Vec<ObjectId> {
        self.parents.clone()
    }

    /// The author `Signature`.
    #[getter]
    fn author(&self, py: Python<'_>) -> Py<Signature> {
        self.author.clone_ref(py)
    }

    /// The committer `Signature`.
    #[getter]
    fn committer(&self, py: Python<'_>) -> Py<Signature> {
        self.committer.clone_ref(py)
    }

    /// The raw commit message bytes (the object body after the header blank line).
    #[getter]
    fn message_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.message)
    }

    /// The commit message decoded to `str` (default UTF-8/strict; caller-overridable).
    // AIDEV-NOTE: `py` is PyO3-injected (NOT part of the Python-visible signature), so the
    // stub stays `-> str`. We return the decoded Python str object so non-strict error
    // handlers (surrogateescape/replace) that yield lone surrogates work (see decode_bytes).
    #[pyo3(signature = (encoding="utf-8", errors="strict"))]
    fn message<'py>(
        &self,
        py: Python<'py>,
        encoding: &str,
        errors: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        decode_bytes(py, &self.message, encoding, errors)
    }
}

impl Commit {
    // AIDEV-NOTE: Parse a commit from the raw object payload (an odb read's `.data`).
    // tree/parents come straight from CommitData. author/committer Signatures are built
    // from the RAW header bytes (author_raw/committer_raw) for the name/email split, plus
    // the decoded String (author/committer) for time parsing.
    //
    // MESSAGE NEWLINE CONTRACT: CommitData.message is the EXACT decoded body after the
    // header blank line, INCLUDING its trailing newline. grit-lib sets raw_message to the
    // verbatim body bytes whenever it is non-empty AND (non-UTF-8 encoding OR not valid
    // UTF-8 OR not LF-terminated); otherwise raw_message is None and message.into_bytes()
    // IS the verbatim body. So `raw_message.unwrap_or_else(|| message.into_bytes())`
    // reproduces the exact body. We surface those bytes UNMODIFIED — so `message_bytes`
    // equals the commit
    // payload's message section, which equals `git log --format=%B` MINUS the single
    // trailing newline git appends to its own output (verified in tests/test_objects.py).
    pub fn from_bytes(py: Python<'_>, id: ObjectId, data: &[u8]) -> PyResult<Self> {
        let c = grit_lib::objects::parse_commit(data).map_err(map_err)?;
        let tree = ObjectId::from_inner(c.tree);
        let parents = c.parents.into_iter().map(ObjectId::from_inner).collect();
        let author = Py::new(py, Signature::parse(&c.author_raw, &c.author))?;
        let committer = Py::new(py, Signature::parse(&c.committer_raw, &c.committer))?;
        let message = c.raw_message.unwrap_or_else(|| c.message.into_bytes());
        Ok(Self {
            id,
            tree,
            parents,
            author,
            committer,
            message,
        })
    }
}

// AIDEV-NOTE: ObjectKind is a Python enum.IntEnum defined in python/pylibgrit/__init__.py.
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

/// Convert a grit-lib object kind into the public `pylibgrit.ObjectKind` IntEnum member.
pub fn kind_to_py(py: Python<'_>, k: grit_lib::objects::ObjectKind) -> PyResult<Py<PyAny>> {
    let cls = OBJECT_KIND_CLS.get_or_try_init(py, || -> PyResult<Py<PyAny>> {
        Ok(py.import("pylibgrit")?.getattr("ObjectKind")?.unbind())
    })?;
    let member = cls.bind(py).call1((object_kind_discriminant(k),))?;
    Ok(member.unbind())
}

// AIDEV-NOTE: `Object` is the value `Odb::read` returns, surfaced to Python. It is
// `frozen` (immutable). `kind` is stored as the already-constructed pylibgrit.ObjectKind
// IntEnum member (built once at read time via kind_to_py) so the getter can hand back
// the singleton (identity-comparable: `obj.kind is pylibgrit.ObjectKind.BLOB`). `data` is
// an `Arc<[u8]>` so the payload can later be shared with typed views without copying.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct Object {
    id: ObjectId,
    kind: Py<PyAny>,
    data: Arc<[u8]>,
}

#[pymethods]
impl Object {
    #[getter]
    fn id(&self) -> ObjectId {
        self.id.clone()
    }

    #[getter]
    fn kind(&self, py: Python<'_>) -> Py<PyAny> {
        self.kind.clone_ref(py)
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }
}

impl Object {
    pub fn new(id: ObjectId, kind: Py<PyAny>, data: Arc<[u8]>) -> Self {
        Self { id, kind, data }
    }
}

// AIDEV-NOTE: Git tree entry mode -> object kind. 0o040000=tree, 0o160000=gitlink
// (submodule, a commit), everything else (0o100644/0o100755 regular/exec, 0o120000
// symlink) is a blob. Derived in the binding layer because grit's TreeEntry has no kind.
fn mode_to_kind(mode: u32) -> grit_lib::objects::ObjectKind {
    match mode {
        0o040000 => grit_lib::objects::ObjectKind::Tree,
        0o160000 => grit_lib::objects::ObjectKind::Commit,
        _ => grit_lib::objects::ObjectKind::Blob,
    }
}

// AIDEV-NOTE: Owning-iterator design (design §6). grit's parse_tree returns an OWNED
// Vec<TreeEntry>, which we copy into Arc<[TreeEntryData]>. A `Tree` holds that Arc; its
// `__iter__` clones the Arc into a `TreeIter`, so the iterator owns its own reference to
// the entry data and stays valid after the parent `Tree` (and the `Repository`/`Odb` it
// came from) is dropped. Each yielded `TreeEntry` clones one `TreeEntryData`, so it too
// is self-contained. There are NO borrows back into grit-lib or the Python objects here.
#[derive(Clone)]
struct TreeEntryData {
    name: Vec<u8>,
    mode: u32,
    oid: grit_lib::objects::ObjectId,
}

/// A single entry in a Git tree (one name → object id, with a file mode).
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct TreeEntry {
    data: TreeEntryData,
}

#[pymethods]
impl TreeEntry {
    /// The entry name as raw bytes (no path separators; non-UTF-8 fidelity, design §5).
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data.name)
    }

    /// The Unix file mode (e.g. `0o100644` regular, `0o040000` tree, `0o120000` symlink).
    #[getter]
    fn mode(&self) -> u32 {
        self.data.mode
    }

    /// The object id of the referenced blob, sub-tree, or (for a gitlink) commit.
    #[getter]
    fn id(&self) -> ObjectId {
        ObjectId::from_inner(self.data.oid)
    }

    /// The `pylibgrit.ObjectKind` derived from the mode (see `mode_to_kind`).
    #[getter]
    fn kind(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        kind_to_py(py, mode_to_kind(self.data.mode))
    }
}

/// A parsed Git tree object: an iterable, len-able collection of `TreeEntry`.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct Tree {
    entries: Arc<[TreeEntryData]>,
}

#[pymethods]
impl Tree {
    fn __len__(&self) -> usize {
        self.entries.len()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> TreeIter {
        // Clone the Arc so the iterator owns its own reference -> outlives this Tree.
        TreeIter {
            entries: Arc::clone(&slf.entries),
            idx: 0,
        }
    }
}

impl Tree {
    // AIDEV-NOTE: Parse a tree from the raw object payload (an odb read's `.data`).
    // We copy grit's owned TreeEntry Vec into Arc<[TreeEntryData]> so the typed view
    // owns all its data independently of grit-lib / the odb buffer.
    pub fn from_bytes(data: &[u8]) -> PyResult<Self> {
        let entries = grit_lib::objects::parse_tree(data).map_err(map_err)?;
        let v: Vec<TreeEntryData> = entries
            .into_iter()
            .map(|e| TreeEntryData {
                name: e.name,
                mode: e.mode,
                oid: e.oid,
            })
            .collect();
        Ok(Self {
            entries: Arc::from(v),
        })
    }
}

/// Iterator over a `Tree`'s entries; owns its own `Arc` so it outlives the `Tree`.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct TreeIter {
    entries: Arc<[TreeEntryData]>,
    idx: usize,
}

#[pymethods]
impl TreeIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<TreeEntry> {
        let e = self.entries.get(self.idx)?.clone();
        self.idx += 1;
        Some(TreeEntry { data: e })
    }
}

// AIDEV-NOTE: `Blob` owns its bytes via `Arc<[u8]>` (shared with the odb read's payload,
// no copy). `frozen` (immutable). The blob payload is the raw object body verbatim.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct Blob {
    data: Arc<[u8]>,
}

#[pymethods]
impl Blob {
    /// The raw blob bytes (the object body, verbatim).
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }
}

impl Blob {
    pub fn new(data: Arc<[u8]>) -> Self {
        Self { data }
    }
}

// AIDEV-NOTE: `Tag` is a binding-layer typed view over grit_lib::objects::parse_tag.
// `frozen` (immutable). FIDELITY LIMITATION: grit-lib 0.4.1's `parse_tag` requires the
// WHOLE tag object to be valid UTF-8 (it errors otherwise) and TagData exposes tag/tagger/
// message ONLY as `String` — there are NO `*_raw: Vec<u8>` byte-fidelity fields like
// CommitData has. So, unlike Commit, a Tag cannot preserve non-UTF-8 bytes in its name,
// tagger ident, or message; we surface `String::into_bytes()` of those decoded values.
// `tagger` is parsed into a Signature from the (UTF-8) ident string's bytes.
//
// MESSAGE NEWLINE CONTRACT: grit's parse_tag accumulates the post-blank-line body and
// strips exactly ONE trailing '\n' that its `split('\n')` adds. So for a body of
// "release one\n" (git appends a trailing LF to `-m` messages), TagData.message ==
// "release one\n", i.e. it KEEPS the body's own trailing newline. We surface those bytes
// unmodified, so `tag.message_bytes` == the tag object's message section verbatim.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct Tag {
    target: ObjectId,
    name: Vec<u8>,
    tagger: Option<Py<Signature>>,
    message: Vec<u8>,
}

#[pymethods]
impl Tag {
    /// The object this tag points to (usually a commit).
    #[getter]
    fn target(&self) -> ObjectId {
        self.target.clone()
    }

    /// The short tag name as raw bytes (e.g. `b"v1"`).
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }

    /// The tagger `Signature`, or `None` for a tag with no tagger header.
    #[getter]
    fn tagger(&self, py: Python<'_>) -> Option<Py<Signature>> {
        self.tagger.as_ref().map(|s| s.clone_ref(py))
    }

    /// The raw tag message bytes (the object body after the header blank line).
    #[getter]
    fn message_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.message)
    }
}

impl Tag {
    // AIDEV-NOTE: Parse a tag from the raw object payload (an odb read's `.data`).
    // target/name/message come straight from TagData. The tagger Signature is built from
    // the ident string's bytes (Signature::parse splits name/email and parses the time);
    // since grit gives only a UTF-8 `String`, we pass `tagger.as_bytes()` for BOTH the raw
    // split and the time string (no separate raw-bytes header available — see fidelity note).
    pub fn from_bytes(py: Python<'_>, data: &[u8]) -> PyResult<Self> {
        let t = grit_lib::objects::parse_tag(data).map_err(map_err)?;
        let target = ObjectId::from_inner(t.object);
        let name = t.tag.into_bytes();
        let tagger = match t.tagger {
            Some(ident) => Some(Py::new(py, Signature::parse(ident.as_bytes(), &ident))?),
            None => None,
        };
        let message = t.message.into_bytes();
        Ok(Self {
            target,
            name,
            tagger,
            message,
        })
    }
}
