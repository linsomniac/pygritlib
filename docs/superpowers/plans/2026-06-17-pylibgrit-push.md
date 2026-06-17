# pylibgrit Phase D — Push & Write-Networking Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `repo.push(url, refspecs, …) -> PushReport` over git:// and https, assembled from grit-lib 0.4.1 `push_remote`/`push_http`.

**Architecture:** A `src/push.rs` porcelain dispatches by URL scheme (reusing Phase C's `net_transport`/`net_credentials`), assembles `grit_lib::transfer::PushRefSpec`s from git-style string refspecs (parsed + local-ref-resolved) or structured `PushSpec` objects, runs the one-round-trip push under `allow_threads`, and maps the `PushOutcome` to a `PushReport`. Rejections are data (per-ref `status`); only transport/auth/protocol failures raise. A re-introduced `PyProgress` bridges a `bytes` callback — push's side-band-2 actually fires.

**Tech Stack:** Rust + PyO3 0.23 (abi3), grit-lib 0.4.1 (`http-ureq` already bundled), maturin, pytest with a receive-pack-enabled `git daemon` fixture (git://) and a receive-pack-enabled `git http-backend` fixture (https), plus a server-side hook for the progress test.

**Spec:** `docs/superpowers/specs/2026-06-17-pylibgrit-push-design.md`

## Build & gates (run after every code change)

```bash
uv run maturin develop --uv --locked
uv run pytest -q
uv run mypy python tests
uv run python -m mypy.stubtest pylibgrit      # NO allowlist
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
uv run ruff format --check
uv run ruff check
```
If `uv run` reinstalls a stale build: `uv pip install -e . --reinstall-package pylibgrit`.

**Imports note:** `clippy -D warnings` denies unused imports/dead code. Each task's code lists the symbols it uses; widen a `use` only when a symbol is first used, and don't pre-import.

## File structure

| File | Responsibility |
| --- | --- |
| `src/net_progress.rs` (re-create) | `PyProgress`: optional `Py<PyAny>` `bytes` callback → `grit_lib::fetch::Progress` (fires for push) |
| `src/push.rs` (new) | `PushSpec`/`PushRefResult`/`PushReport` pyclasses; refspec→`PushRefSpec` assembly; `PushRefStatus`→str; `push_raw`/`push_method` |
| `src/net_transport.rs` (modify) | add `git_connect_receive(url)` (ReceivePack connect) |
| `src/error.rs` (modify) | route `Error::PushOptionsUnsupported` → `NetworkError` |
| `src/repository.rs` (modify) | `Repository.push` (thin delegator) |
| `src/lib.rs` (modify) | `mod net_progress; mod push;`; register the three pyclasses |
| `python/pylibgrit/__init__.{py,pyi}` (modify) | export + stub `PushSpec`/`PushRefResult`/`PushReport` + `Repository.push` |
| `tests/conftest.py` (modify) | receive-pack `git daemon` + `git http-backend` fixtures (+ a local pusher clone) |
| `tests/test_push*.py` (new) | git:// push, semantics, https (anon+auth), progress |

---

## Task 1: Receive-pack `git daemon` fixture (git://)

**Files:** Modify `tests/conftest.py`. Test: `tests/test_push_fixture.py` (create).

- [ ] **Step 1: Write the failing test**

Create `tests/test_push_fixture.py`:

```python
"""The git_daemon_push fixture serves a receive-pack-enabled bare repo (oracle: git push works)."""

from __future__ import annotations

from tests.gitlib import run_git


def test_oracle_push_works(git_daemon_push) -> None:
    # The git CLI (oracle) can push a new commit to the served bare repo over git://.
    local = git_daemon_push.local_path
    env = git_daemon_push.env
    (local / "b.txt").write_text("two\n")
    run_git(local, "add", "-A", env=env)
    run_git(local, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", "c2", env=env)
    run_git(local, "push", "-q", git_daemon_push.repo_url, "main", env=env)
    server_main = run_git(git_daemon_push.server_path, "rev-parse", "refs/heads/main", env=env).decode().strip()
    local_main = run_git(local, "rev-parse", "refs/heads/main", env=env).decode().strip()
    assert server_main == local_main
```

- [ ] **Step 2: Run it to verify it fails**

Run: `uv run pytest tests/test_push_fixture.py -q`
Expected: FAIL with `fixture 'git_daemon_push' not found`.

- [ ] **Step 3: Add the receive-pack flag to `_serve_git_daemon` + the `git_daemon_push` fixture**

In `tests/conftest.py`, change the `_serve_git_daemon` signature and command to accept a `receive_pack` flag:

```python
@contextlib.contextmanager
def _serve_git_daemon(
    base: Path, git_env: dict[str, str], receive_pack: bool = False
) -> Iterator[int]:
    port = _free_port()
    args = [
        "git",
        "daemon",
        "--reuseaddr",
        "--listen=127.0.0.1",
        f"--port={port}",
        f"--base-path={base}",
        "--export-all",
    ]
    if receive_pack:
        # AIDEV-NOTE: git daemon refuses receive-pack (push) unless explicitly enabled.
        args.append("--enable=receive-pack")
    args.append(str(base))
    proc = subprocess.Popen(
        args, env=git_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    try:
        if not _wait_port("127.0.0.1", port, proc, timeout=5.0):
            proc.terminate()
            pytest.skip("git daemon unavailable")
        yield port
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
```

(The existing `git_daemon`/`git_daemon_shared_tag` callers pass no `receive_pack`, so they keep `False` — unchanged behavior.)

Then APPEND the push fixture:

```python
# AIDEV-NOTE: A receive-pack-enabled git:// server (bare) plus a local non-bare clone (the pusher).
# Push tests advance `local_path` (via the git oracle) and push to `repo_url`; the oracle for the
# result is the bare server's refs (`run_git(server_path, "rev-parse", <ref>)`). The bare server is
# safe to push any branch to (no checked-out worktree). `base_oid` is the server's initial main tip.
@pytest.fixture
def git_daemon_push(tmp_path: Path, git_env: dict[str, str]) -> Iterator[SimpleNamespace]:
    """git:// server with receive-pack enabled + a local clone to push from. Skips if no git daemon."""
    base = tmp_path / "psrv"
    base.mkdir()
    src = tmp_path / "psrc"
    src.mkdir()
    _git(src, git_env, "init", "-q", "-b", "main")
    (src / "a.txt").write_text("hello\n")
    _git(src, git_env, "add", "-A")
    _git(src, git_env, "commit", "-q", "-m", "c1")
    server = base / "server.git"
    _git(tmp_path, git_env, "clone", "-q", "--bare", str(src), str(server))
    local = tmp_path / "plocal"
    _git(tmp_path, git_env, "clone", "-q", str(server), str(local))
    base_oid = run_git(server, "rev-parse", "refs/heads/main", env=git_env).decode().strip()

    with _serve_git_daemon(base, git_env, receive_pack=True) as port:
        yield SimpleNamespace(
            repo_url=f"git://127.0.0.1:{port}/server.git",
            server_path=server,
            local_path=local,
            base_oid=base_oid,
            env=git_env,
        )
```

- [ ] **Step 4: Run + gates + commit**

```bash
uv run pytest tests/test_push_fixture.py -q -rs
uv run pytest -q
uv run mypy python tests && uv run ruff format --check && uv run ruff check
git add tests/conftest.py tests/test_push_fixture.py
git commit -m "test: receive-pack git daemon fixture (git:// push)"
```
Expected: the oracle push test PASSES (or SKIPS if `git daemon` is unavailable).

---

## Task 2: `repo.push` over git:// (core)

**Files:** Create `src/net_progress.rs`, `src/push.rs`. Modify `src/net_transport.rs`, `src/error.rs`, `src/repository.rs`, `src/lib.rs`, `python/pylibgrit/__init__.{py,pyi}`. Test: `tests/test_push.py` (create).

- [ ] **Step 1: Write the failing test**

Create `tests/test_push.py`:

```python
"""repo.push over git:// — new branch, fast-forward, and PushReport shape."""

from __future__ import annotations

import pylibgrit
from tests.gitlib import run_git


def _commit(local, env, name, content) -> str:
    (local / name).write_text(content)
    run_git(local, "add", "-A", env=env)
    run_git(local, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", name, env=env)
    return run_git(local, "rev-parse", "HEAD", env=env).decode().strip()


def test_push_fast_forward(git_daemon_push) -> None:
    local = git_daemon_push.local_path
    env = git_daemon_push.env
    new = _commit(local, env, "b.txt", "two\n")
    repo = pylibgrit.Repository.open(local / ".git", local)
    report = repo.push(git_daemon_push.repo_url, ["main"])
    assert report.ok
    server_main = run_git(git_daemon_push.server_path, "rev-parse", "refs/heads/main", env=env).decode().strip()
    assert server_main == new
    [res] = report.results
    assert res.remote_ref == b"refs/heads/main"
    assert res.status in {"ok", "up-to-date"}
    assert res.new_oid.hex == new


def test_push_new_branch(git_daemon_push) -> None:
    local = git_daemon_push.local_path
    env = git_daemon_push.env
    run_git(local, "checkout", "-q", "-b", "feature", env=env)
    new = _commit(local, env, "f.txt", "feat\n")
    repo = pylibgrit.Repository.open(local / ".git", local)
    report = repo.push(git_daemon_push.repo_url, ["feature"])
    assert report.ok
    server = run_git(git_daemon_push.server_path, "rev-parse", "refs/heads/feature", env=env).decode().strip()
    assert server == new
```

- [ ] **Step 2: Run it to verify it fails**

Run: `uv run pytest tests/test_push.py -q`
Expected: FAIL with `'Repository' object has no attribute 'push'`.

- [ ] **Step 3: Re-create `src/net_progress.rs`**

```rust
//! Bridge an optional Python `bytes` callable to grit's `fetch::Progress` (side-band-2 stream).

use pyo3::prelude::*;
use pyo3::types::PyBytes;

// AIDEV-NOTE: `grit_lib::fetch::Progress` has a single infallible method `message(&mut self, &[u8])`.
// `PyProgress` wraps an optional Python callable invoked once per side-band-2 chunk. The transfer
// runs under allow_threads (GIL released); `message` re-acquires the GIL via `Python::with_gil` for
// just the callback, so the callback never holds the GIL across the transfer. A Python exception is
// CAPTURED (grit's `message` cannot return an error / unwind through FFI) and re-raised by the caller
// via `take_error()` after the transfer returns. `Py<PyAny>` + `Option<PyErr>` are both Send, so
// `&mut PyProgress` may cross into allow_threads. For FETCH this never fires (grit forces no-progress)
// so fetch passes NoProgress directly; for PUSH the side-band-2 carries the remote's hook/diagnostic
// output and this DOES fire.
pub(crate) struct PyProgress {
    callback: Option<Py<PyAny>>,
    error: Option<PyErr>,
}

impl PyProgress {
    pub(crate) fn new(callback: Option<Py<PyAny>>) -> Self {
        Self {
            callback,
            error: None,
        }
    }
    pub(crate) fn take_error(&mut self) -> Option<PyErr> {
        self.error.take()
    }
}

impl grit_lib::fetch::Progress for PyProgress {
    fn message(&mut self, bytes: &[u8]) {
        if self.error.is_some() {
            return;
        }
        let Some(cb) = &self.callback else {
            return;
        };
        Python::with_gil(|py| {
            let arg = PyBytes::new(py, bytes);
            if let Err(e) = cb.call1(py, (arg,)) {
                self.error = Some(e);
            }
        });
    }
}
```

- [ ] **Step 4: Add `git_connect_receive` to `src/net_transport.rs`**

Append:

```rust
// AIDEV-NOTE: Connect a git:// service for PUSH (git-receive-pack). Forces protocol v0/v1
// (`protocol_version: 0`) because grit's push rejects v2. Like `git_connect`, the returned
// `Box<dyn Connection>` is `!Send` — construct + consume it inside one `allow_threads` closure.
pub(crate) fn git_connect_receive(
    url: &str,
) -> Result<Box<dyn Connection>, grit_lib::error::Error> {
    let opts = ConnectOptions {
        protocol_version: 0,
        server_options: Vec::new(),
    };
    GitDaemonTransport::new().connect(url, Service::ReceivePack, &opts)
}
```

- [ ] **Step 5: Route `PushOptionsUnsupported` → `NetworkError` in `src/error.rs`**

In `net_map_err`, extend the NetworkError arm:

```rust
        Error::Message(_) | Error::Io(_) | Error::PushOptionsUnsupported => {
            NetworkError::new_err(format!("{e}"))
        }
```

- [ ] **Step 6: Create `src/push.rs`**

```rust
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

// AIDEV-NOTE: Qualify a bare source name as a branch (refs/heads/<name>) for the default destination,
// matching `git push origin <name>`. A name already starting with refs/ is used as-is.
fn qualify_branch(s: &str) -> String {
    if s.starts_with("refs/") {
        s.to_owned()
    } else {
        format!("refs/heads/{s}")
    }
}

// AIDEV-NOTE: Parse ONE string refspec into a grit PushRefSpec. Uses grit's parse_push_refspec to
// split force/src/dst; an empty source ⇒ delete (dst required); otherwise resolve the source ref/rev
// to an oid (resolve_revision) and default dst to the qualified source ref. Lease fields are NOT
// expressible in a string (always None/false) — use a PushSpec for force-with-lease.
fn parse_one_refspec(
    repo: &grit_lib::repo::Repository,
    s: &str,
    force_kwarg: bool,
) -> PyResult<PushRefSpec> {
    let item = grit_lib::refspec::parse_push_refspec(s)
        .map_err(|e| PyValueError::new_err(format!("{e}")))?;
    // A bare object id as the source needs an explicit destination (we can't infer one).
    if item.exact_sha1 && item.dst.is_none() {
        return Err(PyValueError::new_err(format!(
            "refspec {s:?}: pushing a raw object id requires an explicit destination (<oid>:<dst>)"
        )));
    }
    let force = item.force || force_kwarg;
    let src = item.src.filter(|x| !x.is_empty());
    match src {
        None => {
            let dst = item.dst.ok_or_else(|| {
                PyValueError::new_err(format!("delete refspec {s:?} needs a destination"))
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
            let dst = item.dst.unwrap_or_else(|| qualify_branch(&src_name));
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
```

- [ ] **Step 7: Add `Repository.push` to `src/repository.rs`**

Inside `#[pymethods] impl Repository` (e.g. after `fetch`):

```rust
    // AIDEV-NOTE: Push to `url` (== `git push`). refspecs is a list of git-style strings ("main",
    // "+a:b", ":refs/heads/old" delete) and/or structured PushSpec objects (force-with-lease, raw
    // oids). Rejections come back as PushReport data (per-ref status); only transport/auth/protocol
    // failures raise. progress= receives the remote's side-band-2 (hook/diagnostic) output.
    #[pyo3(signature = (url, refspecs, *, force=false, atomic=false, dry_run=false,
                        push_options=None, username=None, password=None,
                        use_credential_helpers=true, progress=None))]
    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        py: Python<'_>,
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
    ) -> PyResult<crate::push::PushReport> {
        crate::push::push_method(
            py, &self.inner, url, refspecs, force, atomic, dry_run, push_options, username,
            password, use_credential_helpers, progress,
        )
    }
```

- [ ] **Step 8: Register in `src/lib.rs`**

Add the module declarations (with the others): `mod net_progress;` and `mod push;`. In `fn _pylibgrit`, register the three classes (after the existing `remote::` registrations):

```rust
    m.add_class::<push::PushSpec>()?;
    m.add_class::<push::PushRefResult>()?;
    m.add_class::<push::PushReport>()?;
```

- [ ] **Step 9: Export + stubs**

In `python/pylibgrit/__init__.py`: add `PushRefResult,`, `PushReport,`, `PushSpec,` to the import block and `__all__`.

In `python/pylibgrit/__init__.pyi`: add the three to `__all__`; re-add `Callable` to the typing import (`from typing import Callable, Iterator, final`); add the class stubs (near the other value objects):

```python
@final
class PushSpec:
    def __new__(
        cls,
        dst: bytes,
        *,
        src: ObjectId | None = None,
        force: bool = False,
        delete: bool = False,
        expected_old: ObjectId | None = None,
        expect_absent: bool = False,
    ) -> PushSpec: ...
    @property
    def dst(self) -> bytes: ...
    @property
    def src(self) -> ObjectId | None: ...
    @property
    def force(self) -> bool: ...
    @property
    def delete(self) -> bool: ...
    @property
    def expected_old(self) -> ObjectId | None: ...
    @property
    def expect_absent(self) -> bool: ...

@final
class PushRefResult:
    @property
    def local_ref(self) -> bytes | None: ...
    @property
    def remote_ref(self) -> bytes: ...
    @property
    def old_oid(self) -> ObjectId | None: ...
    @property
    def new_oid(self) -> ObjectId | None: ...
    @property
    def forced(self) -> bool: ...
    @property
    def deletion(self) -> bool: ...
    @property
    def status(self) -> str: ...
    @property
    def message(self) -> str | None: ...

@final
class PushReport:
    @property
    def results(self) -> list[PushRefResult]: ...
    @property
    def ok(self) -> bool: ...
```

and the `push` method stub inside `class Repository` (after `fetch`):

```python
    def push(
        self,
        url: str,
        refspecs: list[str | PushSpec],
        *,
        force: bool = False,
        atomic: bool = False,
        dry_run: bool = False,
        push_options: list[str] | None = None,
        username: str | None = None,
        password: str | None = None,
        use_credential_helpers: bool = True,
        progress: Callable[[bytes], None] | None = None,
    ) -> PushReport: ...
```

- [ ] **Step 10: Build, test, ALL gates, commit**

```bash
uv run maturin develop --uv --locked
uv run pytest tests/test_push.py -q
uv run pytest -q
uv run mypy python tests && uv run python -m mypy.stubtest pylibgrit
cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings
uv run ruff format --check && uv run ruff check
git add src/ python/pylibgrit/ tests/test_push.py
git commit -m "feat: repo.push over git:// (PushSpec/PushReport + progress bridge)"
```

## Before you begin (Task 2)
The trickiest bits: (1) the git:// `!Send` connection must be constructed AND consumed inside the one `allow_threads` closure; (2) `&mut prog` (PyProgress) crosses into `allow_threads` and `message` re-acquires the GIL; (3) `refspecs: Vec<Py<PyAny>>` is resolved to `Vec<PushRefSpec>` UNDER the GIL (via `build_push_specs`) before the closure. If a grit-lib signature differs from the plan (e.g. `PushRefSpec` fields, `push_remote` arg order, `resolve_revision`), STOP and report rather than guessing.

---

## Task 3: git:// push semantics (delete, force, non-ff, lease, dry-run, atomic)

**Files:** Test: `tests/test_push_semantics.py` (create). (No production code — Task 2 already handles these via grit; this task verifies each `PushRefStatus` path + the structured `PushSpec`/lease path.)

- [ ] **Step 1: Write the failing test**

Create `tests/test_push_semantics.py`:

```python
"""repo.push git:// semantics: delete, force, non-ff rejection, lease, dry-run, atomic."""

from __future__ import annotations

import pylibgrit
from tests.gitlib import run_git


def _commit(local, env, name, content) -> str:
    (local / name).write_text(content)
    run_git(local, "add", "-A", env=env)
    run_git(local, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", name, env=env)
    return run_git(local, "rev-parse", "HEAD", env=env).decode().strip()


def _server_ref(p, env, ref) -> str | None:
    out = run_git(p.server_path, "for-each-ref", "--format=%(objectname)", ref, env=env).decode().strip()
    return out or None


def _open(p):
    return pylibgrit.Repository.open(p.local_path / ".git", p.local_path)


def test_push_delete(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    # create a branch on the server first
    run_git(p.local_path, "push", "-q", p.repo_url, "main:refs/heads/doomed", env=env)
    assert _server_ref(p, env, "refs/heads/doomed") is not None
    report = _open(p).push(p.repo_url, [":refs/heads/doomed"])
    assert report.ok
    assert report.results[0].deletion is True
    assert _server_ref(p, env, "refs/heads/doomed") is None


def test_push_non_fast_forward_rejected(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    _commit(p.local_path, env, "b.txt", "two\n")
    _open(p).push(p.repo_url, ["main"])                      # server main advances
    run_git(p.local_path, "reset", "-q", "--hard", p.base_oid, env=env)
    diverged = _commit(p.local_path, env, "c.txt", "other\n")  # diverged history
    report = _open(p).push(p.repo_url, ["main"])             # no force
    assert not report.ok
    assert report.results[0].status == "reject-non-fast-forward"
    assert _server_ref(p, env, "refs/heads/main") != diverged


def test_push_force(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    _commit(p.local_path, env, "b.txt", "two\n")
    _open(p).push(p.repo_url, ["main"])
    run_git(p.local_path, "reset", "-q", "--hard", p.base_oid, env=env)
    diverged = _commit(p.local_path, env, "c.txt", "other\n")
    report = _open(p).push(p.repo_url, ["main"], force=True)
    assert report.ok
    assert _server_ref(p, env, "refs/heads/main") == diverged


def test_push_lease_stale_rejected(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    new = _commit(p.local_path, env, "b.txt", "two\n")
    # lease says the remote must currently be at a WRONG oid -> reject-stale
    wrong = pylibgrit.ObjectId.from_hex(new)  # any non-matching oid (server is at base_oid)
    spec = pylibgrit.PushSpec(b"refs/heads/main", src=pylibgrit.ObjectId.from_hex(new), expected_old=wrong)
    report = _open(p).push(p.repo_url, [spec])
    assert not report.ok
    assert report.results[0].status == "reject-stale"
    assert _server_ref(p, env, "refs/heads/main") == p.base_oid


def test_push_lease_fresh_accepted(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    new = _commit(p.local_path, env, "b.txt", "two\n")
    spec = pylibgrit.PushSpec(
        b"refs/heads/main",
        src=pylibgrit.ObjectId.from_hex(new),
        expected_old=pylibgrit.ObjectId.from_hex(p.base_oid),  # correct current value
    )
    report = _open(p).push(p.repo_url, [spec])
    assert report.ok
    assert _server_ref(p, env, "refs/heads/main") == new


def test_push_dry_run(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    new = _commit(p.local_path, env, "b.txt", "two\n")
    report = _open(p).push(p.repo_url, ["main"], dry_run=True)
    assert _server_ref(p, env, "refs/heads/main") == p.base_oid   # unchanged
    assert report.results[0].new_oid.hex == new                   # computed


def test_push_atomic_all_or_nothing(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    # Advance server main, then diverge local main (a non-ff) AND add a good feature branch.
    _commit(p.local_path, env, "b.txt", "two\n")
    _open(p).push(p.repo_url, ["main"])
    run_git(p.local_path, "reset", "-q", "--hard", p.base_oid, env=env)
    _commit(p.local_path, env, "c.txt", "other\n")               # main now diverged (non-ff)
    run_git(p.local_path, "checkout", "-q", "-b", "feature", env=env)
    feat = _commit(p.local_path, env, "f.txt", "feat\n")
    report = _open(p).push(p.repo_url, ["main", "feature"], atomic=True)  # no force
    assert not report.ok
    by_ref = {r.remote_ref: r.status for r in report.results}
    assert by_ref[b"refs/heads/main"] == "reject-non-fast-forward"
    assert by_ref[b"refs/heads/feature"] == "atomic-push-failed"
    # Neither landed: feature absent, main still at the earlier (non-diverged) push.
    assert _server_ref(p, env, "refs/heads/feature") is None
    assert _server_ref(p, env, "refs/heads/main") != feat


def test_push_raw_oid_without_dest_raises(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    head = run_git(p.local_path, "rev-parse", "HEAD", env=env).decode().strip()
    import pytest

    with pytest.raises(ValueError):
        _open(p).push(p.repo_url, [head])   # a bare oid with no destination
```

- [ ] **Step 2: Run + verify**

Run: `uv run pytest tests/test_push_semantics.py -q`
Expected: all PASS (Task 2's code already handles each path via grit; the bare-oid test exercises the `exact_sha1 && dst.is_none()` guard added in Task 2). If a status string mismatches grit's actual output (e.g. a fast-forward over an existing branch reports `"ok"` vs `"up-to-date"`, or `dry_run` does not populate `new_oid`), adjust THAT assertion to grit's real value — do NOT weaken a rejection/lease/atomic assertion. If a real bug surfaces (e.g. lease not enforced, or atomic still mutates the server), STOP and report.

- [ ] **Step 3: Gates + commit**

```bash
uv run pytest -q
uv run mypy python tests && uv run ruff format --check && uv run ruff check
git add tests/test_push_semantics.py
git commit -m "test: push semantics (delete, force, non-ff, lease, dry-run)"
```

---

## Task 4: HTTPS push (anonymous)

**Files:** Modify `tests/conftest.py`. Test: `tests/test_push_http.py` (create). (The https push code path is already in Task 2's `push_method`; this task adds the receive-pack http fixture + tests.)

- [ ] **Step 1: Write the failing test**

Create `tests/test_push_http.py`:

```python
"""repo.push over anonymous smart-HTTP (git http-backend with receive-pack enabled)."""

from __future__ import annotations

import pylibgrit
from tests.gitlib import run_git


def test_http_push(http_push_server) -> None:
    p, env = http_push_server, http_push_server.env
    (p.local_path / "b.txt").write_text("two\n")
    run_git(p.local_path, "add", "-A", env=env)
    run_git(p.local_path, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", "c2", env=env)
    new = run_git(p.local_path, "rev-parse", "HEAD", env=env).decode().strip()
    repo = pylibgrit.Repository.open(p.local_path / ".git", p.local_path)
    report = repo.push(p.repo_url, ["main"])
    assert report.ok
    assert run_git(p.server_path, "rev-parse", "refs/heads/main", env=env).decode().strip() == new
```

- [ ] **Step 2: Run it to verify it fails**

Run: `uv run pytest tests/test_push_http.py -q`
Expected: FAIL with `fixture 'http_push_server' not found`.

- [ ] **Step 3: Add a receive-pack option to `_make_http_server` + the `http_push_server` fixture**

In `tests/conftest.py`, change `_make_http_server` to accept `receive_pack` and enable it on the bare server, and add `env` to the returned namespace:

```python
def _make_http_server(
    tmp_path: Path,
    git_env: dict[str, str],
    auth: tuple[str, str] | None,
    receive_pack: bool = False,
):
    """Seed a bare server repo and serve it over smart-HTTP. Returns (namespace, shutdown)."""
    base = tmp_path / "httpsrv"
    base.mkdir()
    src = tmp_path / "httpsrc"
    src.mkdir()
    _git(src, git_env, "init", "-q", "-b", "main")
    (src / "a.txt").write_text("hello\n")
    _git(src, git_env, "add", "-A")
    _git(src, git_env, "commit", "-q", "-m", "initial commit")
    server = base / "server.git"
    _git(tmp_path, git_env, "clone", "-q", "--bare", str(src), str(server))
    if receive_pack:
        # AIDEV-NOTE: git http-backend serves git-receive-pack (push) only when the repo opts in.
        _git(server, git_env, "config", "http.receivepack", "true")
    head_oid = run_git(src, "rev-parse", "HEAD", env=git_env).decode().strip()

    try:
        httpd = githttp.serve(base, git_env, auth)
    except OSError:
        pytest.skip("could not start http server")
    port = httpd.server_address[1]
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()

    ns = SimpleNamespace(
        repo_url=f"http://127.0.0.1:{port}/server.git",
        head_oid=head_oid,
        server_path=server,
        env=git_env,
    )

    def shutdown() -> None:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)

    return ns, shutdown
```

(The existing `http_server`/`http_auth_server` callers pass no `receive_pack`, so it defaults `False` — unchanged. Adding `env=git_env` to the namespace is backward-compatible.)

Then APPEND the push fixture (it also makes a local pusher clone of the served bare repo):

```python
# AIDEV-NOTE: Anonymous smart-HTTP server with receive-pack ENABLED (http.receivepack=true) plus a
# local non-bare clone (the pusher). Skips if git http-backend is unavailable; tears down the thread.
@pytest.fixture
def http_push_server(tmp_path: Path, git_env: dict[str, str]) -> Iterator[SimpleNamespace]:
    """Anonymous receive-pack smart-HTTP server + a local clone to push from."""
    if not _git_http_backend_available(git_env):
        pytest.skip("git http-backend unavailable")
    ns, shutdown = _make_http_server(tmp_path, git_env, auth=None, receive_pack=True)
    local = tmp_path / "httppushlocal"
    _git(tmp_path, git_env, "clone", "-q", str(ns.server_path), str(local))
    ns.local_path = local
    try:
        yield ns
    finally:
        shutdown()
```

- [ ] **Step 4: Build/test/gates/commit**

```bash
uv run pytest tests/test_push_http.py -q -rs
uv run pytest -q
uv run mypy python tests && uv run python -m mypy.stubtest pylibgrit
cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings
uv run ruff format --check && uv run ruff check
git add tests/conftest.py tests/test_push_http.py
git commit -m "feat: https push (git http-backend receive-pack fixture + tests)"
```
Expected: the http push test PASSES (or SKIPS if `git http-backend` is unavailable).

---

## Task 5: HTTPS push with credentials (auth)

**Files:** Modify `tests/conftest.py`. Test: `tests/test_push_http_auth.py` (create). (Credential code already exists from Phase C; `push_http` uses the same client.)

- [ ] **Step 1: Write the failing test**

Create `tests/test_push_http_auth.py`:

```python
"""repo.push over Basic-auth smart-HTTP: correct creds succeed; missing creds raise."""

from __future__ import annotations

import pytest

import pylibgrit
from tests.gitlib import run_git

USER, PW = "alice", "s3cret"


def _advance(local, env) -> str:
    (local / "b.txt").write_text("two\n")
    run_git(local, "add", "-A", env=env)
    run_git(local, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", "c2", env=env)
    return run_git(local, "rev-parse", "HEAD", env=env).decode().strip()


def test_auth_push_with_kwargs(http_auth_push_server) -> None:
    p, env = http_auth_push_server, http_auth_push_server.env
    new = _advance(p.local_path, env)
    repo = pylibgrit.Repository.open(p.local_path / ".git", p.local_path)
    report = repo.push(p.repo_url, ["main"], username=USER, password=PW, use_credential_helpers=False)
    assert report.ok
    assert run_git(p.server_path, "rev-parse", "refs/heads/main", env=env).decode().strip() == new


def test_auth_push_missing_credentials_raises(http_auth_push_server) -> None:
    p, env = http_auth_push_server, http_auth_push_server.env
    _advance(p.local_path, env)
    repo = pylibgrit.Repository.open(p.local_path / ".git", p.local_path)
    with pytest.raises(pylibgrit.AuthenticationError):
        repo.push(p.repo_url, ["main"], use_credential_helpers=False)
```

- [ ] **Step 2: Run it to verify it fails**

Run: `uv run pytest tests/test_push_http_auth.py -q`
Expected: FAIL with `fixture 'http_auth_push_server' not found`.

- [ ] **Step 3: Add the `http_auth_push_server` fixture**

APPEND to `tests/conftest.py`:

```python
# AIDEV-NOTE: Basic-auth (alice/s3cret) receive-pack smart-HTTP server + a local pusher clone (cloned
# with creds in the URL so the local origin works, though tests push to the credential-less repo_url
# and supply creds via the API). Skips if git http-backend is unavailable.
@pytest.fixture
def http_auth_push_server(tmp_path: Path, git_env: dict[str, str]) -> Iterator[SimpleNamespace]:
    """Basic-auth receive-pack smart-HTTP server + a local clone to push from."""
    if not _git_http_backend_available(git_env):
        pytest.skip("git http-backend unavailable")
    ns, shutdown = _make_http_server(tmp_path, git_env, auth=("alice", "s3cret"), receive_pack=True)
    local = tmp_path / "httpauthpushlocal"
    auth_url = ns.repo_url.replace("http://", "http://alice:s3cret@")
    _git(tmp_path, git_env, "clone", "-q", auth_url, str(local))
    ns.local_path = local
    try:
        yield ns
    finally:
        shutdown()
```

- [ ] **Step 4: Build/test/gates/commit**

```bash
uv run pytest tests/test_push_http_auth.py -q -rs
uv run pytest -q
uv run mypy python tests && uv run python -m mypy.stubtest pylibgrit
cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings
uv run ruff format --check && uv run ruff check
git add tests/conftest.py tests/test_push_http_auth.py
git commit -m "test: https push with Basic auth (kwargs + AuthenticationError)"
```

---

## Task 6: Push progress (server hook)

**Files:** Test: `tests/test_push_progress.py` (create). (Progress is already wired in Task 2's `push_method` via `PyProgress`; this task proves it fires using a server-side hook.)

- [ ] **Step 1: Write the failing test**

Create `tests/test_push_progress.py`:

```python
"""The push progress callback receives the remote's side-band-2 (hook) output."""

from __future__ import annotations

import os
import stat

import pylibgrit
from tests.gitlib import run_git

MARKER = b"hello-from-hook"


def _install_hook(server_path) -> None:
    # post-receive runs after a successful update; receive-pack relays its stdout to the client on
    # side-band channel 2 (the "remote: ..." stream).
    hook = server_path / "hooks" / "post-receive"
    hook.write_text("#!/bin/sh\necho hello-from-hook\n")
    hook.chmod(hook.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def test_push_progress_receives_hook_output(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    _install_hook(p.server_path)
    (p.local_path / "b.txt").write_text("two\n")
    run_git(p.local_path, "add", "-A", env=env)
    run_git(p.local_path, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", "c2", env=env)
    chunks: list[bytes] = []
    repo = pylibgrit.Repository.open(p.local_path / ".git", p.local_path)
    report = repo.push(p.repo_url, ["main"], progress=chunks.append)
    assert report.ok
    # The hook's stdout is relayed on side-band-2 and delivered to our callback.
    assert any(MARKER in c for c in chunks), f"expected hook output in {chunks!r}"


def test_push_progress_callback_exception_propagates(git_daemon_push) -> None:
    p, env = git_daemon_push, git_daemon_push.env
    _install_hook(p.server_path)
    (p.local_path / "b.txt").write_text("two\n")
    run_git(p.local_path, "add", "-A", env=env)
    run_git(p.local_path, "-c", "user.name=T", "-c", "user.email=t@e", "commit", "-q", "-m", "c2", env=env)

    class Boom(Exception):
        pass

    def cb(_data: bytes) -> None:
        raise Boom("stop")

    repo = pylibgrit.Repository.open(p.local_path / ".git", p.local_path)
    with __import__("pytest").raises(Boom):
        repo.push(p.repo_url, ["main"], progress=cb)
```

- [ ] **Step 2: Run + verify**

Run: `uv run pytest tests/test_push_progress.py -q -rs`
Expected: PASS — the hook's `hello-from-hook` line is relayed on side-band-2 and delivered to the callback; a raising callback propagates as the push's failure. If NO chunks arrive (the server didn't relay the hook on band-2), STOP and report — the `quiet` cap should not suppress hook output, and grit appends `side-band-64k`, but confirm against the live server.

- [ ] **Step 3: Gates + commit**

```bash
uv run pytest -q
uv run mypy python tests && uv run ruff format --check && uv run ruff check
git add tests/test_push_progress.py
git commit -m "test: push progress callback receives remote hook output"
```

---

## Task 7: Docs + 0.4.0 staging

**Files:** Modify `README.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`.

- [ ] **Step 1: README "Pushing" subsection**

In `README.md`, extend the "Networking" section with a **"Pushing"** subsection documenting:
- `repo.push(url, refspecs, *, force=False, atomic=False, dry_run=False, push_options=None, username=None, password=None, use_credential_helpers=True, progress=None) -> PushReport`.
- `refspecs`: git-style strings (`"main"`, `"+a:b"`, `":refs/heads/old"` delete) and/or `PushSpec` objects.
- `PushSpec(dst, *, src=None, force=False, delete=False, expected_old=None, expect_absent=False)` — incl. force-with-lease.
- `PushReport` (`.results: list[PushRefResult]`, `.ok`) and `PushRefResult` (`.remote_ref`, `.status`, `.message`, …); statuses are returned, not raised.
- progress callback receives the remote's `remote: …` hook/diagnostic output.
- A runnable example:

````markdown
```python
import pylibgrit

repo = pylibgrit.Repository.open("/path/to/repo/.git", "/path/to/repo")

# Push the local 'main' to a remote over https (token via kwarg or https://<token>@host/...).
report = repo.push("https://github.com/me/repo.git", ["main"], username="x", password="TOKEN")
for r in report.results:
    print(r.status, r.remote_ref.decode(), r.message or "")
if not report.ok:
    raise SystemExit("push rejected")

# Force-with-lease (safe force) via a structured PushSpec:
tip = repo.resolve("refs/heads/main")
expected = repo.resolve("refs/remotes/origin/main")
spec = pylibgrit.PushSpec(b"refs/heads/main", src=tip, expected_old=expected)
repo.push("https://github.com/me/repo.git", [spec])

# Delete a remote branch:
repo.push("https://github.com/me/repo.git", [":refs/heads/old-feature"])
```
````

Update the "Known limitations" note: push is v0/v1 only (grit rejects v2); no ssh/signed/submodule push; string refspecs can't express force-with-lease (use `PushSpec`). Add a `0.4.0` row to the version-compatibility table. Add `PushSpec`/`PushRefResult`/`PushReport` wherever the value types are listed.

- [ ] **Step 2: CHANGELOG**

Add a `## [0.4.0]` section above `## [0.3.0]`: `repo.push` over git:// and https (`push_remote`/`push_http`); string refspecs + structured `PushSpec`; force / delete / force-with-lease (`expected_old`/`expect_absent`); `atomic` / `dry_run` / `push_options`; rejections returned as `PushReport` data; a working push progress callback; new types `PushSpec`/`PushRefResult`/`PushReport`. Deferred: ssh, signed push, submodule push, v2 push.

- [ ] **Step 3: Bump version to 0.4.0**

In `Cargo.toml`, change `version = "0.3.0"` to `version = "0.4.0"`. Refresh `Cargo.lock` (run `uv run maturin develop --uv --locked`). Confirm `grep -n '^version' Cargo.toml` is 0.4.0 and the `pylibgrit` package in `Cargo.lock` is 0.4.0.

- [ ] **Step 4: Full suite + gates**

```bash
uv run maturin develop --uv --locked
uv run pytest -q
uv run mypy python tests && uv run python -m mypy.stubtest pylibgrit
cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings
uv run ruff format --check && uv run ruff check
```

- [ ] **Step 5: Commit**

```bash
git add README.md CHANGELOG.md Cargo.toml Cargo.lock
git commit -m "docs: document push; stage 0.4.0"
```

---

## Definition of done

- `repo.push(url, refspecs, …) -> PushReport` over git:// and https; string refspecs + structured `PushSpec`; force / delete / force-with-lease / atomic / dry-run / push-options; rejections as data; a working progress callback.
- git:// tests cover new-branch / ff / delete / force / non-ff-reject / lease / dry-run; https covers anon + Basic-auth push; progress is proven via a server hook. All skip-if-unavailable.
- All 7 gates green; stub matches runtime with no allowlist; version staged at 0.4.0.
- Completes the A→B→C→D roadmap. Deferred: ssh, signed push, submodule push, v2 push.
