//! Read-path network porcelain: ls_remote / fetch / clone, plus the value-object pyclasses.

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use grit_lib::transfer::{FetchOptions, FetchOutcome, TagMode, UpdateMode};

use crate::error::{map_err, net_map_err, network_err};
use crate::net_transport::{classify, git_connect, Scheme};

// AIDEV-NOTE: One advertised remote ref. `name`/`symref_target` are bytes (house style: ref names
// are bytes everywhere in the binding); `oid` is an ObjectId. HEAD is synthesized from the
// connection's head_symref + the symref target's advertised oid (advertised_refs excludes HEAD).
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct RemoteRef {
    name: Vec<u8>,
    oid: grit_lib::objects::ObjectId,
    symref_target: Option<Vec<u8>>,
}

#[pymethods]
impl RemoteRef {
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }
    #[getter]
    fn oid(&self) -> crate::objects::ObjectId {
        crate::objects::ObjectId::from_inner(self.oid)
    }
    #[getter]
    fn symref_target<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.symref_target.as_ref().map(|t| PyBytes::new(py, t))
    }
    fn __repr__(&self) -> String {
        format!(
            "RemoteRef(name={:?}, oid='{}')",
            String::from_utf8_lossy(&self.name),
            self.oid.to_hex()
        )
    }
}

// AIDEV-NOTE: Read the v0/v1 ref advertisement from a freshly-opened connection. Returns owned
// (name, oid) pairs plus the HEAD symref target — everything is cloned out so the `!Send` connection
// is dropped inside the caller's allow_threads closure. `protocol_version: 1` forces the
// advertisement (v2 advertises nothing on connect).
#[allow(clippy::type_complexity)]
fn read_advertisement(
    url: &str,
) -> Result<(Vec<(String, grit_lib::objects::ObjectId)>, Option<String>), grit_lib::error::Error> {
    let conn = git_connect(url, 1)?;
    let refs = conn.advertised_refs().to_vec();
    let head = conn.head_symref().map(str::to_owned);
    Ok((refs, head))
}

// AIDEV-NOTE: List remote refs (== `git ls-remote`), built from the connection advertisement (grit's
// `ls_remote` is local-only). `heads`/`tags` restrict to those namespaces and drop the synthesized
// HEAD row (matching `git ls-remote --heads/--tags`). Peeled `^{}` rows are not surfaced (grit's
// advertised_refs omits them) — a documented limitation.
#[pyfunction]
#[pyo3(signature = (url, *, username=None, password=None, use_credential_helpers=true, heads=false, tags=false))]
pub fn ls_remote(
    py: Python<'_>,
    url: String,
    username: Option<String>,
    password: Option<String>,
    use_credential_helpers: bool,
    heads: bool,
    tags: bool,
) -> PyResult<Vec<RemoteRef>> {
    let (advertised, head_target) = match classify(&url)? {
        Scheme::Git => py
            .allow_threads(|| read_advertisement(&url))
            .map_err(net_map_err)?,
        Scheme::Http => crate::net_credentials::http_advertisement(
            py,
            &url,
            username,
            password,
            use_credential_helpers,
        )?,
    };

    let mut out: Vec<RemoteRef> = Vec::new();

    // Synthesized HEAD row (only in the unfiltered default, like `git ls-remote`).
    if !heads && !tags {
        if let Some(target) = &head_target {
            if let Some((_, oid)) = advertised.iter().find(|(n, _)| n == target) {
                out.push(RemoteRef {
                    name: b"HEAD".to_vec(),
                    oid: *oid,
                    symref_target: Some(target.clone().into_bytes()),
                });
            }
        }
    }

    for (name, oid) in advertised {
        let keep = if heads {
            name.starts_with("refs/heads/")
        } else if tags {
            name.starts_with("refs/tags/")
        } else {
            true
        };
        if keep {
            out.push(RemoteRef {
                name: name.into_bytes(),
                oid,
                symref_target: None,
            });
        }
    }
    Ok(out)
}

// AIDEV-NOTE: One applied ref update from a fetch. Ref names are bytes; oids are ObjectId; `mode` is
// the lower-kebab `UpdateMode` name; `note` is grit's human-readable annotation.
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct RefUpdate {
    remote_ref: Vec<u8>,
    local_ref: Option<Vec<u8>>,
    old_oid: Option<grit_lib::objects::ObjectId>,
    new_oid: Option<grit_lib::objects::ObjectId>,
    mode: String,
    note: Option<String>,
}

#[pymethods]
impl RefUpdate {
    #[getter]
    fn remote_ref<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.remote_ref)
    }
    #[getter]
    fn local_ref<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.local_ref.as_ref().map(|r| PyBytes::new(py, r))
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
    fn mode(&self) -> &str {
        &self.mode
    }
    #[getter]
    fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
}

// AIDEV-NOTE: The result of a fetch: the applied ref updates + the remote's default branch (HEAD
// symref). Shallow fields are intentionally omitted (shallow deferred).
#[pyclass(frozen, module = "pylibgrit._pylibgrit")]
pub struct FetchReport {
    updates: Vec<Py<RefUpdate>>,
    default_branch: Option<Vec<u8>>,
}

#[pymethods]
impl FetchReport {
    #[getter]
    fn updates(&self, py: Python<'_>) -> Vec<Py<RefUpdate>> {
        self.updates.iter().map(|u| u.clone_ref(py)).collect()
    }
    #[getter]
    fn default_branch<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.default_branch.as_ref().map(|b| PyBytes::new(py, b))
    }
}

// AIDEV-NOTE: grit's UpdateMode -> the lower-kebab string surfaced as RefUpdate.mode.
fn update_mode_str(m: UpdateMode) -> &'static str {
    match m {
        UpdateMode::New => "new",
        UpdateMode::FastForward => "fast-forward",
        UpdateMode::Forced => "forced",
        UpdateMode::UpToDate => "up-to-date",
        UpdateMode::NoChangeNeeded => "no-change-needed",
        UpdateMode::NonFastForwardRejected => "non-fast-forward-rejected",
        UpdateMode::TagUpdateRejected => "tag-update-rejected",
        UpdateMode::SourceObjectNotFound => "source-object-not-found",
        UpdateMode::Unborn => "unborn",
        UpdateMode::DeletedMissing => "deleted-missing",
    }
}

// AIDEV-NOTE: Core fetch: dispatch by scheme, return the raw FetchOutcome. BOTH
// `grit_lib::fetch::fetch_remote` (git://) and `grit_lib::transport::http::http_fetch` (https) unpack
// objects AND write the New/FastForward/Forced tracking refs (and prune) internally — verified
// against grit-lib-0.4.1 fetch.rs:1327-1360 — so the binding does NO ref application; it only maps
// the FetchOutcome to a FetchReport. The git:// connection is `!Send`, so it is constructed and
// consumed entirely inside one allow_threads closure (never crossing the boundary).
//
// Progress is unconditionally NoProgress: grit-lib 0.4.1 sends `no-progress` in every upload-pack
// request, so the Progress::message hook never fires over the network. The progress= parameter was
// dropped from the public API to avoid a misleading dead knob.
pub(crate) fn fetch_raw(
    py: Python<'_>,
    git_dir: &std::path::Path,
    url: &str,
    opts: &FetchOptions,
    username: Option<String>,
    password: Option<String>,
    use_credential_helpers: bool,
) -> PyResult<FetchOutcome> {
    let outcome = match classify(url)? {
        Scheme::Git => py
            .allow_threads(|| -> Result<FetchOutcome, grit_lib::error::Error> {
                let mut conn = git_connect(url, 0)?;
                let mut np = grit_lib::fetch::NoProgress;
                grit_lib::fetch::fetch_remote(git_dir, &mut *conn, opts, &mut np)
            })
            .map_err(net_map_err)?,
        Scheme::Http => {
            let (clean_url, userinfo) = crate::net_transport::split_userinfo(url);
            let client = crate::net_credentials::build_http_client(
                py,
                Some(git_dir),
                merge_user(username, &userinfo),
                merge_pass(password, &userinfo),
                use_credential_helpers,
            )?;
            py.allow_threads(|| {
                let mut np = grit_lib::fetch::NoProgress;
                grit_lib::transport::http::http_fetch(&client, git_dir, &clean_url, opts, &mut np)
            })
            .map_err(net_map_err)?
        }
    };
    Ok(outcome)
}

// AIDEV-NOTE: Credentials precedence: explicit kwargs win, else fall back to URL userinfo.
fn merge_user(
    explicit: Option<String>,
    userinfo: &Option<(String, Option<String>)>,
) -> Option<String> {
    explicit.or_else(|| userinfo.as_ref().map(|(u, _)| u.clone()))
}
fn merge_pass(
    explicit: Option<String>,
    userinfo: &Option<(String, Option<String>)>,
) -> Option<String> {
    explicit.or_else(|| userinfo.as_ref().and_then(|(_, p)| p.clone()))
}

// AIDEV-NOTE: Build FetchOptions from the Python kwargs (default refspec fetches all heads into
// refs/remotes/origin/*). `tags` maps to grit's TagMode.
pub(crate) fn build_fetch_options(
    refspecs: Option<Vec<String>>,
    tags: &str,
    prune: bool,
) -> PyResult<FetchOptions> {
    let tagmode = match tags {
        "none" => TagMode::None,
        "following" => TagMode::Following,
        "all" => TagMode::All,
        other => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "tags must be 'none', 'following', or 'all' (got {other:?})"
            )))
        }
    };
    let refspecs =
        refspecs.unwrap_or_else(|| vec!["+refs/heads/*:refs/remotes/origin/*".to_owned()]);
    Ok(FetchOptions {
        refspecs,
        tags: tagmode,
        prune,
        ..Default::default()
    })
}

// AIDEV-NOTE: Build a FetchReport (Python objects) from a raw FetchOutcome.
fn build_report(py: Python<'_>, outcome: FetchOutcome) -> PyResult<FetchReport> {
    let mut updates = Vec::with_capacity(outcome.updates.len());
    for u in outcome.updates {
        let ru = RefUpdate {
            remote_ref: u.remote_ref.into_bytes(),
            local_ref: u.local_ref.map(String::into_bytes),
            old_oid: u.old_oid,
            new_oid: u.new_oid,
            mode: update_mode_str(u.mode).to_owned(),
            note: u.note,
        };
        updates.push(Py::new(py, ru)?);
    }
    Ok(FetchReport {
        updates,
        default_branch: outcome.default_branch.map(String::into_bytes),
    })
}

// AIDEV-NOTE: Repository.fetch entry point (called from src/repository.rs).
#[allow(clippy::too_many_arguments)]
pub(crate) fn fetch_method(
    py: Python<'_>,
    repo: &Arc<grit_lib::repo::Repository>,
    url: String,
    refspecs: Option<Vec<String>>,
    tags: &str,
    prune: bool,
    username: Option<String>,
    password: Option<String>,
    use_credential_helpers: bool,
) -> PyResult<FetchReport> {
    let opts = build_fetch_options(refspecs, tags, prune)?;
    let git_dir = repo.git_dir.clone();
    let outcome = fetch_raw(
        py,
        &git_dir,
        &url,
        &opts,
        username,
        password,
        use_credential_helpers,
    )?;
    build_report(py, outcome)
}

// AIDEV-NOTE: Write the `[remote "origin"]` stanza into a freshly-init'd repo's .git/config (url +
// the standard fetch refspec), so the result is a git-recognized clone. Uses grit's round-trip
// ConfigFile editor (preserves existing entries). The config file exists post-init.
fn write_origin_config(git_dir: &std::path::Path, url: &str) -> Result<(), grit_lib::error::Error> {
    let path = git_dir.join("config");
    let mut cf =
        grit_lib::config::ConfigFile::from_path(&path, grit_lib::config::ConfigScope::Local)?
            .ok_or_else(|| {
                grit_lib::error::Error::Message(format!("config missing at {}", path.display()))
            })?;
    cf.set("remote.origin.url", url)?;
    cf.set("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")?;
    cf.write()?;
    Ok(())
}

// AIDEV-NOTE: Write the per-branch upstream stanza (branch.<name>.remote = origin,
// branch.<name>.merge = refs/heads/<name>) that `git clone` writes for the checked-out branch, so a
// bare `git pull`/`git push` in the clone knows its upstream. Done AFTER the branch is resolved
// (the name is only known post-fetch). `merge` is the REMOTE-side ref name (refs/heads/<name>), per
// git's branch upstream convention. Round-trip editor, so it preserves the [remote "origin"] stanza.
fn write_branch_upstream(
    git_dir: &std::path::Path,
    name: &str,
    remote_head: &str,
) -> Result<(), grit_lib::error::Error> {
    let path = git_dir.join("config");
    let mut cf =
        grit_lib::config::ConfigFile::from_path(&path, grit_lib::config::ConfigScope::Local)?
            .ok_or_else(|| {
                grit_lib::error::Error::Message(format!("config missing at {}", path.display()))
            })?;
    cf.set(&format!("branch.{name}.remote"), "origin")?;
    cf.set(&format!("branch.{name}.merge"), remote_head)?;
    cf.write()?;
    Ok(())
}

// AIDEV-NOTE: clone = init (non-bare) + origin config + fetch ALL heads & tags + materialize ONE
// branch (explicit `branch=`, else the remote default) as refs/heads/<name> + HEAD + checkout +
// branch.<name>.{remote,merge} upstream config. Uses tags="all": git clone fetches all tags, AND it
// avoids a grit-lib 0.4.1 tags="following" bug that drops a head's objects when a tag shares that
// head's oid (see spec §8). grit's fetch writes the refs/remotes/origin/* tracking refs internally;
// we then create the local branch + HEAD and check out (empty worktree ⇒ overlay == full checkout).
// ERROR MAPPING: only fetch_raw (step 3) is a network op; the local init/config/ref/odb steps use
// map_err (GritError/PyOSError) so a local failure is not mis-surfaced as NetworkError. Bare/shallow
// clone are deferred (spec §1). LIMITATION: clone into a non-empty existing path RE-INITS over it
// (init_repository has no already-exists guard) rather than refusing like `git clone` — deferred.
pub(crate) fn clone_impl(
    py: Python<'_>,
    url: String,
    path: std::path::PathBuf,
    branch: Option<String>,
    username: Option<String>,
    password: Option<String>,
    use_credential_helpers: bool,
) -> PyResult<crate::repository::Repository> {
    classify(&url)?; // fail fast on an unsupported scheme before touching the filesystem

    // 1. init a non-bare repo.
    let repo = py
        .allow_threads(|| grit_lib::repo::init_repository(&path, false, "main", None, "files"))
        .map_err(map_err)?;
    let repo = std::sync::Arc::new(repo);
    let git_dir = repo.git_dir.clone();
    let work_tree = repo
        .work_tree
        .clone()
        .ok_or_else(|| network_err("clone target has no work tree (internal error)"))?;

    // 2. origin config.
    py.allow_threads(|| write_origin_config(&git_dir, &url))
        .map_err(map_err)?;

    // 3. fetch all heads + tags into refs/remotes/origin/* (+ refs/tags/*). grit writes the refs.
    let opts = build_fetch_options(None, "all", false)?;
    let outcome = fetch_raw(
        py,
        &git_dir,
        &url,
        &opts,
        username,
        password,
        use_credential_helpers,
    )?;

    // 4. resolve which branch to check out.
    let name = match branch {
        Some(b) => b.strip_prefix("refs/heads/").unwrap_or(&b).to_owned(),
        None => {
            let db = outcome.default_branch.as_deref().ok_or_else(|| {
                network_err("remote did not advertise a default branch; pass branch=")
            })?;
            db.strip_prefix("refs/heads/").unwrap_or(db).to_owned()
        }
    };
    let local_head = format!("refs/heads/{name}");
    let tracking = format!("refs/remotes/origin/{name}");

    // 5. create local branch = tracking oid; point HEAD at it; write the upstream config.
    let tip = py
        .allow_threads(|| grit_lib::refs::resolve_ref(&git_dir, &tracking))
        .map_err(|_| network_err(&format!("branch {name:?} not found on remote")))?;
    py.allow_threads(|| grit_lib::refs::write_ref(&git_dir, &local_head, &tip))
        .map_err(map_err)?;
    py.allow_threads(|| grit_lib::refs::write_symbolic_ref(&git_dir, "HEAD", &local_head))
        .map_err(map_err)?;
    py.allow_threads(|| write_branch_upstream(&git_dir, &name, &local_head))
        .map_err(map_err)?;

    // 6. checkout the tip commit's tree (overlay == full checkout into the empty worktree).
    let tree_oid = py
        .allow_threads(
            || -> Result<grit_lib::objects::ObjectId, grit_lib::error::Error> {
                let obj = repo.odb.read(&tip)?;
                let commit = grit_lib::objects::parse_commit(&obj.data)?;
                Ok(commit.tree)
            },
        )
        .map_err(map_err)?;
    py.allow_threads(|| crate::checkout::checkout_tree(&repo, &work_tree, &tree_oid, false, true))
        .map_err(crate::checkout::to_pyerr)?;

    Ok(crate::repository::Repository { inner: repo })
}
