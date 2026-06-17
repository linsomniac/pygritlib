//! Write-path network porcelain: repo.push over git:// and https, plus the value-object pyclasses.

use std::sync::Arc;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use grit_lib::push_report::PushRefStatus;
use grit_lib::transfer::{PushOptions, PushOutcome, PushRefSpec};

use crate::error::net_map_err;
use crate::net_transport::{classify, git_connect_receive, Scheme};

// AIDEV-NOTE: A push ref update (constructable input). `dst` is bytes (house style: ref names are
// bytes); grit's PushRefSpec.dst is a String, so `dst` is converted to UTF-8 when building the spec
// (non-UTF-8 dst → ValueError). `src=None` means a deletion. `expected_old`/`expect_absent` are the
// force-with-lease knobs. Frozen + getters (immutable value object); `#[new]` is the constructor.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct PushSpec {
    src: Option<grit_lib::objects::ObjectId>,
    dst: Vec<u8>,
    force: bool,
    delete: bool,
    expected_old: Option<grit_lib::objects::ObjectId>,
    expect_absent: bool,
}

#[pymethods]
impl PushSpec {
    #[new]
    #[pyo3(signature = (dst, *, src=None, force=false, delete=false, expected_old=None, expect_absent=false))]
    fn new(
        dst: Vec<u8>,
        src: Option<PyRef<'_, crate::objects::ObjectId>>,
        force: bool,
        delete: bool,
        expected_old: Option<PyRef<'_, crate::objects::ObjectId>>,
        expect_absent: bool,
    ) -> Self {
        Self {
            src: src.map(|o| o.inner()),
            dst,
            force,
            delete,
            expected_old: expected_old.map(|o| o.inner()),
            expect_absent,
        }
    }
    #[getter]
    fn dst<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.dst)
    }
    #[getter]
    fn src(&self) -> Option<crate::objects::ObjectId> {
        self.src.map(crate::objects::ObjectId::from_inner)
    }
    #[getter]
    fn force(&self) -> bool {
        self.force
    }
    #[getter]
    fn delete(&self) -> bool {
        self.delete
    }
    #[getter]
    fn expected_old(&self) -> Option<crate::objects::ObjectId> {
        self.expected_old.map(crate::objects::ObjectId::from_inner)
    }
    #[getter]
    fn expect_absent(&self) -> bool {
        self.expect_absent
    }
}

impl PushSpec {
    // AIDEV-NOTE: Build the grit PushRefSpec. `force_kwarg` (the method-level force=) ORs with the
    // per-spec force. dst bytes → UTF-8 (grit's dst is String).
    fn to_ref_spec(&self, force_kwarg: bool) -> PyResult<PushRefSpec> {
        let dst = String::from_utf8(self.dst.clone())
            .map_err(|_| PyValueError::new_err("PushSpec.dst must be valid UTF-8"))?;
        Ok(PushRefSpec {
            src: self.src,
            dst,
            force: self.force || force_kwarg,
            delete: self.delete,
            expected_old: self.expected_old,
            expect_absent: self.expect_absent,
        })
    }
}

// AIDEV-NOTE: One per-ref push result (output, frozen). Ref names bytes; oids ObjectId; `status` is
// the lower-kebab PushRefStatus name; `message` is the server's `ng <ref> <reason>` text (remote
// rejections).
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct PushRefResult {
    local_ref: Option<Vec<u8>>,
    remote_ref: Vec<u8>,
    old_oid: Option<grit_lib::objects::ObjectId>,
    new_oid: Option<grit_lib::objects::ObjectId>,
    forced: bool,
    deletion: bool,
    status: String,
    message: Option<String>,
}

#[pymethods]
impl PushRefResult {
    #[getter]
    fn local_ref<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.local_ref.as_ref().map(|r| PyBytes::new(py, r))
    }
    #[getter]
    fn remote_ref<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.remote_ref)
    }
    #[getter]
    fn old_oid(&self) -> Option<crate::objects::ObjectId> {
        self.old_oid.map(crate::objects::ObjectId::from_inner)
    }
    #[getter]
    fn new_oid(&self) -> Option<crate::objects::ObjectId> {
        self.new_oid.map(crate::objects::ObjectId::from_inner)
    }
    #[getter]
    fn forced(&self) -> bool {
        self.forced
    }
    #[getter]
    fn deletion(&self) -> bool {
        self.deletion
    }
    #[getter]
    fn status(&self) -> &str {
        &self.status
    }
    #[getter]
    fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

// AIDEV-NOTE: The result of a push: per-ref results + an `ok` convenience (every ref ok/up-to-date).
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct PushReport {
    results: Vec<Py<PushRefResult>>,
    ok: bool,
}

#[pymethods]
impl PushReport {
    #[getter]
    fn results(&self, py: Python<'_>) -> Vec<Py<PushRefResult>> {
        self.results.iter().map(|r| r.clone_ref(py)).collect()
    }
    #[getter]
    fn ok(&self) -> bool {
        self.ok
    }
}

// AIDEV-NOTE: grit's PushRefStatus -> the lower-kebab string surfaced as PushRefResult.status.
fn push_status_str(s: &PushRefStatus) -> &'static str {
    match s {
        PushRefStatus::UpToDate => "up-to-date",
        PushRefStatus::Ok => "ok",
        PushRefStatus::RejectNonFastForward => "reject-non-fast-forward",
        PushRefStatus::RejectAlreadyExists => "reject-already-exists",
        PushRefStatus::RejectFetchFirst => "reject-fetch-first",
        PushRefStatus::RejectNeedsForce => "reject-needs-force",
        PushRefStatus::RejectStale => "reject-stale",
        PushRefStatus::RemoteRejected => "remote-rejected",
        PushRefStatus::AtomicPushFailed => "atomic-push-failed",
    }
}

// AIDEV-NOTE: Default a push destination by DWIM-resolving the SOURCE name to its fully-qualified
// local ref (git's precedence: refs/<n>, refs/tags/<n>, refs/heads/<n>, refs/remotes/<n>,
// refs/remotes/<n>/HEAD), then pushing to that same name — so `push(url, ["main"])` -> refs/heads/main
// and `push(url, ["v1.0"])` (a tag) -> refs/tags/v1.0. A source already starting with `refs/` is used
// verbatim. If no candidate ref exists (e.g. the source is a bare oid or a non-ref rev), the
// destination cannot be inferred and we error — the caller must give an explicit <src>:<dst>.
fn default_push_dst(repo: &grit_lib::repo::Repository, src: &str) -> PyResult<String> {
    if src.starts_with("refs/") {
        return Ok(src.to_owned());
    }
    for cand in [
        format!("refs/{src}"),
        format!("refs/tags/{src}"),
        format!("refs/heads/{src}"),
        format!("refs/remotes/{src}"),
        format!("refs/remotes/{src}/HEAD"),
    ] {
        if grit_lib::refs::resolve_ref(&repo.git_dir, &cand).is_ok() {
            return Ok(cand);
        }
    }
    Err(PyValueError::new_err(format!(
        "refspec {src:?}: cannot infer a destination ref (the source is not a local ref); \
         specify an explicit destination as <src>:<dst>"
    )))
}

// AIDEV-NOTE: Parse ONE string refspec into a grit PushRefSpec. Uses grit's parse_push_refspec to
// split force/src/dst; an empty source ⇒ delete (dst required); otherwise resolve the source ref/rev
// to an oid (resolve_revision) and, when no explicit dst was given, DWIM-default the dst from the
// source's fully-qualified local ref (default_push_dst) — so a tag source lands in refs/tags/* and a
// branch in refs/heads/*. A bare oid / non-ref source with no dst cannot infer a destination and
// errors (the caller must give <src>:<dst>). Lease fields are NOT expressible in a string (always
// None/false) — use a PushSpec for force-with-lease.
fn parse_one_refspec(
    repo: &grit_lib::repo::Repository,
    s: &str,
    force_kwarg: bool,
) -> PyResult<PushRefSpec> {
    let item = grit_lib::refspec::parse_push_refspec(s)
        .map_err(|e| PyValueError::new_err(format!("{e}")))?;
    let force = item.force || force_kwarg;
    let src = item.src.filter(|x| !x.is_empty());
    match src {
        None => {
            // src is empty: a delete (`:dst`) needs a destination; a bare `:` (push all matching
            // refs) is a distinct, unsupported git feature — report each accurately.
            let dst = item.dst.ok_or_else(|| {
                if item.matching {
                    PyValueError::new_err(format!(
                        "the matching refspec {s:?} (push all matching refs) is not supported; \
                         specify explicit refspecs"
                    ))
                } else {
                    PyValueError::new_err(format!(
                        "delete refspec {s:?} needs a destination (e.g. \":refs/heads/<name>\")"
                    ))
                }
            })?;
            Ok(PushRefSpec {
                src: None,
                dst,
                force,
                delete: true,
                expected_old: None,
                expect_absent: false,
            })
        }
        Some(src_name) => {
            let oid = grit_lib::rev_parse::resolve_revision(repo, &src_name)
                .map_err(crate::error::map_err)?;
            let dst = match item.dst {
                Some(d) => d,
                None => default_push_dst(repo, &src_name)?,
            };
            Ok(PushRefSpec {
                src: Some(oid),
                dst,
                force,
                delete: false,
                expected_old: None,
                expect_absent: false,
            })
        }
    }
}

// AIDEV-NOTE: Build the Vec<PushRefSpec> from a heterogeneous Python list of str | PushSpec. Runs
// under the GIL (resolves local refs via the repo). A str is parsed/resolved; a PushSpec is converted
// directly; anything else is a TypeError.
fn build_push_specs(
    py: Python<'_>,
    repo: &grit_lib::repo::Repository,
    refspecs: Vec<Py<PyAny>>,
    force_kwarg: bool,
) -> PyResult<Vec<PushRefSpec>> {
    let mut out = Vec::with_capacity(refspecs.len());
    for item in refspecs {
        let bound = item.bind(py);
        if let Ok(s) = bound.extract::<String>() {
            out.push(parse_one_refspec(repo, &s, force_kwarg)?);
        } else if let Ok(spec) = bound.extract::<PyRef<'_, PushSpec>>() {
            out.push(spec.to_ref_spec(force_kwarg)?);
        } else {
            return Err(PyTypeError::new_err(
                "each refspec must be a str or a PushSpec",
            ));
        }
    }
    Ok(out)
}

// AIDEV-NOTE: Map a PushOutcome to a PushReport (and compute `ok` = all refs ok/up-to-date).
fn build_push_report(py: Python<'_>, outcome: PushOutcome) -> PyResult<PushReport> {
    let mut results = Vec::with_capacity(outcome.results.len());
    let mut ok = true;
    for r in outcome.results {
        if !matches!(r.status, PushRefStatus::Ok | PushRefStatus::UpToDate) {
            ok = false;
        }
        let prr = PushRefResult {
            local_ref: r.local_ref.map(String::into_bytes),
            remote_ref: r.remote_ref.into_bytes(),
            old_oid: r.old_oid,
            new_oid: r.new_oid,
            forced: r.forced,
            deletion: r.deletion,
            status: push_status_str(&r.status).to_owned(),
            message: r.message,
        };
        results.push(Py::new(py, prr)?);
    }
    Ok(PushReport { results, ok })
}

// AIDEV-NOTE: Repository.push entry point. Resolves refspecs under the GIL, then dispatches by scheme.
// git:// connects (ReceivePack) + pushes inside one allow_threads closure (the `Box<dyn Connection>`
// is !Send); https builds the credential-bearing UreqHttpClient (reused from Phase C) and uses
// push_http. Push's side-band-2 (remote hook/diagnostic output) flows to the optional progress
// callback via PyProgress; a callback exception is surfaced after the transfer. Rejections are NOT
// raised — they come back as PushRefResult.status. Only transport/auth/protocol failures raise.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_method(
    py: Python<'_>,
    repo: &Arc<grit_lib::repo::Repository>,
    url: String,
    refspecs: Vec<Py<PyAny>>,
    force: bool,
    atomic: bool,
    dry_run: bool,
    push_options: Option<Vec<String>>,
    username: Option<String>,
    password: Option<String>,
    use_credential_helpers: bool,
    progress: Option<Py<PyAny>>,
) -> PyResult<PushReport> {
    let specs = build_push_specs(py, repo, refspecs, force)?;
    let opts = PushOptions {
        atomic,
        dry_run,
        push_options: push_options.unwrap_or_default(),
    };
    let git_dir = repo.git_dir.clone();
    let mut prog = crate::net_progress::PyProgress::new(progress);

    let outcome = match classify(&url)? {
        Scheme::Git => {
            let result = py.allow_threads(|| -> Result<PushOutcome, grit_lib::error::Error> {
                let mut conn = git_connect_receive(&url)?;
                grit_lib::push::push_remote(&git_dir, &mut *conn, &specs, &opts, &mut prog)
            });
            if let Some(e) = prog.take_error() {
                return Err(e);
            }
            result.map_err(net_map_err)?
        }
        Scheme::Http => {
            let (clean_url, userinfo) = crate::net_transport::split_userinfo(&url);
            let user = username.or_else(|| userinfo.as_ref().map(|(u, _)| u.clone()));
            let pass = password.or_else(|| userinfo.as_ref().and_then(|(_, p)| p.clone()));
            let client = crate::net_credentials::build_http_client(
                py,
                Some(&git_dir),
                user,
                pass,
                use_credential_helpers,
            )?;
            let result = py.allow_threads(|| {
                grit_lib::push::push_http(&client, &git_dir, &clean_url, &specs, &opts, &mut prog)
            });
            if let Some(e) = prog.take_error() {
                return Err(e);
            }
            result.map_err(net_map_err)?
        }
    };
    build_push_report(py, outcome)
}
