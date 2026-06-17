# pylibgrit

Native Python bindings for [`grit-lib`](https://crates.io/crates/grit-lib) — the
core Rust library of [gitbutlerapp/grit](https://github.com/gitbutlerapp/grit), a
from-scratch reimplementation of Git in Rust. pylibgrit is built with
[PyO3](https://pyo3.rs) and packaged as an `abi3` wheel with
[maturin](https://maturin.rs). pylibgrit is a thin Python façade over grit-lib
covering **reading** — discover/open repositories, read objects (commit/tree/blob/tag),
list and resolve references, walk history, diff commits, and read config — a **local
write surface** (since 0.2.0): write objects, stage an index, build trees, create
commit/tag objects, and mutate refs — **read-path networking** (since 0.3.0): clone,
fetch, and list remote refs over git:// and https — and **push** (since 0.4.0): push
refs to a remote over git:// and https, with force, delete, atomic, dry-run, and
force-with-lease (compare-and-swap) support. No system OpenSSL or libcurl required.
Everything runs in-process, with no external `git` binary required at runtime and no
system C libraries to build.

## Install / build from source

This project uses [`uv`](https://docs.astral.sh/uv/) for Python environment and
dependency management (no pip/poetry/requirements.txt), maturin for the build, and
a pinned Rust toolchain (`rust-toolchain.toml`, channel 1.94.1). There are **no
`-sys`/pkg-config dependencies** — grit-lib uses pure-Rust compression
(`miniz_oxide`) and hashing (`sha1`/`sha2`).

```bash
# 1. Create the venv and install dev dependencies (maturin, pytest, mypy, ruff).
uv venv
uv sync --group dev

# 2. Build the native extension and install it editable into the venv.
uv run maturin develop --uv

# 3. Run the tests.
uv run pytest tests/ -v
```

### Building a wheel / sdist

```bash
uv run maturin build --release --locked   # wheel -> target/wheels/
uv run maturin sdist                       # sdist -> target/wheels/
```

The wheel is tagged `cp311-abi3-<platform>` and works on CPython 3.11+.

## Quickstart

```python
import pylibgrit

# Discover the repository containing the given path (walks upward to find .git).
repo = pylibgrit.Repository.discover(".")
# Or open an explicit git dir:  pylibgrit.Repository.open("/path/to/.git")

# Resolve HEAD to an ObjectId, then read the commit it points at.
head = repo.resolve("HEAD")          # ObjectId  (also resolves "main", "HEAD~2", hex, ...)
commit = repo.commit(head)
print(commit.id.hex)                 # 40-char (SHA-1) or 64-char (SHA-256) hex
print(commit.author.name_str, commit.author.email_str)   # decoded str accessors
print(commit.message().splitlines()[0])                  # decoded subject line
print([p.hex for p in commit.parents])

# List the entries of the commit's tree.
tree = repo.tree(commit.tree)
for entry in tree:
    # entry.name is bytes (exact, git-faithful); mode is an int; id is an ObjectId.
    print(f"{entry.mode:06o} {entry.id.hex} {entry.name!r}")

# Walk history, newest first (like `git rev-list HEAD`); yields Commit objects.
for c in repo.revwalk(head):
    print(c.id.hex[:10], c.message().splitlines()[0])

# Diff a commit against its first parent (pass commit ids; tree extraction is internal).
if commit.parents:
    diff = repo.diff(commit.parents[0], head)
    s = diff.stats
    print(f"{s.files_changed} files changed, +{s.insertions} -{s.deletions}")
    for e in diff:
        # e.status is one of A/D/M/R/C/T/U; paths are bytes (or None).
        print(e.status, (e.new_path or e.old_path))

# Read config (last-wins, includes system config).
cfg = repo.config
print(cfg.get_bool("core.bare"))     # None if absent
print(cfg.get_str("user.email"))

# Iterate references; resolve a reference to a final ObjectId.
for ref in repo.references():
    print(ref.name, ref.peel().hex)

# Read raw object bytes straight from the object database.
obj = repo.odb.read(head)
print(obj.kind, len(obj.data))       # obj.kind is an ObjectKind; obj.data is bytes
```

## Writing (local write-core)

Since **0.2.0**, pylibgrit exposes a **local write surface** — enough to build commits
and move refs entirely in-process. It mirrors git's plumbing (write object → stage index
→ write tree → create commit → update ref), and `create_commit`/`create_tag` produce
**byte-identical object ids** to git.

```python
import pylibgrit
from pylibgrit import ObjectKind, Signature

repo = pylibgrit.Repository.open("/path/to/.git")     # or .discover(".")

# 1. Write a blob straight to the object database (== git hash-object -w).
blob = repo.odb.write(ObjectKind.BLOB, b"hello\n")
#    repo.odb.hash(kind, data) computes the oid WITHOUT writing it.

# 2. Stage entries into the index, then persist it.
idx = repo.index()
idx.add(b"greeting.txt", blob, 0o100644)   # stage a blob already in the odb
# idx.stage(b"path/in/worktree")           # OR hash a real working-tree file (needs a work tree)
idx.write()                                # write .git/index   (len(idx) / `for e in idx: ...`)

# 3. Build a tree object from the index (== git write-tree).
tree = idx.write_tree()                    # -> ObjectId

# 4. Create a commit object (== git commit-tree). Pure: returns the oid, moves no ref.
#    Signature is (name, email, (unix_seconds, tz_offset_seconds)).
me = Signature(b"Ada", b"ada@example.com", (1718000000, 0))
commit = repo.create_commit(tree, parents=[], author=me, committer=me, message=b"init\n")
#    For byte-exact ids with unusual identities, pass raw header bytes instead of a Signature:
#      repo.create_commit(tree, [], author_raw=b"Ada <ada@x> 1718000000 +0000",
#                         committer_raw=b"...", message=b"...")

# 5. Move a branch at the new commit, and point HEAD at it.
repo.update_ref(b"refs/heads/main", commit, create=True)   # create-only: fails if it exists
repo.set_head(b"refs/heads/main")
#    Other ref ops: expected_old= for compare-and-swap, message=/signer= to write a reflog,
#    delete_ref(...), set_symbolic_ref(...), append_reflog(...).

# Annotated tags create a tag OBJECT; point a ref at it separately.
tag = repo.create_tag(commit, ObjectKind.COMMIT, b"v1", message=b"release\n", tagger=me)
repo.update_ref(b"refs/tags/v1", tag, create=True)
```

**Ref update modes** (`update_ref`): the default overwrites; `create=True` is create-only
(fails if the ref exists); `expected_old=<oid>` is a compare-and-swap. Compare-and-swap is
**best-effort** — grit-lib 0.4.1 has no atomic CAS primitive, so it is a read→compare→write
without a held lock (it catches the common non-concurrent case but is not a hard guarantee
against another writer in the window).

**Write-input validation.** Constructing a `Signature` rejects `<`, `>`, NUL, or newline in
the name/email and out-of-range / non-minute timezone offsets; index paths must be clean
relative paths (no leading/trailing `/`, no `.`/`..` components); ref names are validated by
git's ref-format rules; reflog messages and tag names reject NUL/CR/LF. These prevent object/
record injection, path traversal, and a grit-lib stack-overflow on malformed index paths.

## Networking (clone / fetch / ls-remote / push)

Since **0.3.0**, pylibgrit exposes a **read-path networking surface** — clone from a
remote, fetch into an existing repository, or list a remote's refs without cloning —
over **git://** and **https** (the `http-ureq` / rustls stack is bundled by default; no
system OpenSSL or libcurl required). Since **0.4.0**, `repo.push` is also available over
the same transports. SSH transport, shallow/depth, bare/mirror clone, and submodules are
not yet supported.

### Entry points

```python
# Top-level function — no local repo needed.
pylibgrit.ls_remote(
    url: str,
    *,
    username: str | None = None,
    password: str | None = None,
    use_credential_helpers: bool = True,
    heads: bool = False,
    tags: bool = False,
) -> list[RemoteRef]

# Class method — init + origin config + fetch + checkout (worktree clone).
# Fetches ALL tags (tags="all"), like `git clone`.
# Sets branch.<name>.remote/merge upstream tracking.
pylibgrit.Repository.clone(
    url: str,
    path: str | bytes | os.PathLike[str],
    *,
    branch: str | None = None,
    username: str | None = None,
    password: str | None = None,
    use_credential_helpers: bool = True,
) -> Repository

# Instance method — fetch into an existing repo.
# Default refspec: +refs/heads/*:refs/remotes/origin/*
# tags ∈ {"none", "following", "all"}  (default "following")
repo.fetch(
    url: str,
    refspecs: list[str] | None = None,
    *,
    tags: str = "following",
    prune: bool = False,
    username: str | None = None,
    password: str | None = None,
    use_credential_helpers: bool = True,
) -> FetchReport

# Instance method — push refs to a remote (since 0.4.0).
repo.push(
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
) -> PushReport
```

### Value objects

| Class | Fields |
| --- | --- |
| `RemoteRef` | `.name: bytes`, `.oid: ObjectId`, `.symref_target: bytes \| None` |
| `RefUpdate` | `.remote_ref: bytes`, `.local_ref: bytes \| None`, `.old_oid: ObjectId \| None`, `.new_oid: ObjectId \| None`, `.mode: str`, `.note: str \| None` |
| `FetchReport` | `.updates: list[RefUpdate]`, `.default_branch: bytes \| None` |
| `PushSpec` | `dst: bytes` (remote ref), `src: ObjectId \| None` (None ⇒ delete), `force: bool`, `delete: bool`, `expected_old: ObjectId \| None`, `expect_absent: bool` |
| `PushRefResult` | `.local_ref: bytes \| None`, `.remote_ref: bytes`, `.old_oid: ObjectId \| None`, `.new_oid: ObjectId \| None`, `.forced: bool`, `.deletion: bool`, `.status: str`, `.message: str \| None` |
| `PushReport` | `.results: list[PushRefResult]`, `.ok: bool` |

### Exceptions

`NetworkError` and `AuthenticationError` are both subclasses of `GritError` (see
[Exception hierarchy](#exception-hierarchy)).

### Authentication

Credentials are resolved in this order of precedence:

1. **Explicit kwargs** — `username=` / `password=` passed directly.
2. **URL userinfo** — `https://<token>@host/path` (token-as-password style).
3. **Git credential helpers** — queried when `use_credential_helpers=True` (the
   default), using the standard git credential protocol.

### Supported transports

| Transport / feature | Status |
| --- | --- |
| `https://` fetch/clone/ls-remote | Supported (rustls bundled, no system OpenSSL) |
| `git://` fetch/clone/ls-remote | Supported |
| `https://` push | Supported (since 0.4.0) |
| `git://` push | Supported (since 0.4.0) |
| `ssh://` / `git@` | Not yet supported |
| Signed push (`--signed`) | Not yet supported |
| Submodule push | Not yet supported |
| Push protocol v2 | Not yet supported (grit rejects v2 push; falls back to v1) |
| Shallow / `--depth` | Not yet supported |
| Bare / mirror clone | Not yet supported |

### Example

```python
import pylibgrit

# Clone a public repo over https (the http stack is bundled).
repo = pylibgrit.Repository.clone("https://github.com/octocat/Hello-World.git", "/tmp/hello")
print(repo.head().peel().hex)

# List a remote's branches without cloning.
for ref in pylibgrit.ls_remote("https://github.com/octocat/Hello-World.git", heads=True):
    print(ref.oid.hex, ref.name.decode())

# Authenticated fetch (token via kwarg, or https://<token>@host/...).
report = repo.fetch("https://github.com/me/private.git", username="x", password="TOKEN")
for u in report.updates:
    print(u.mode, u.remote_ref.decode())
```

### Pushing

Since **0.4.0**, `repo.push` sends refs to a remote over **git://** or **https**.

#### refspecs

`refspecs` is a list that may contain:

- **Strings** — git-style refspec shorthand:
  - `"main"` → push `refs/heads/main` to `refs/heads/main` (the source's fully-qualified
    ref is used as the destination); a tag `"v1.0"` → `refs/tags/v1.0` likewise.
  - `"+a:b"` — force-push `a` to `b`.
  - `":refs/heads/old"` — delete `refs/heads/old` on the remote.
  - A bare object id with no explicit destination (e.g. `"abc123"`) raises `ValueError` —
    use a `PushSpec` instead.
  - A source that isn't a local branch/tag/remote ref — including `HEAD` — has no inferable
    destination and raises `ValueError`; give an explicit one, e.g. `"HEAD:refs/heads/main"`.
- **`PushSpec` objects** — for full control:
  `PushSpec(dst, *, src=None, force=False, delete=False, expected_old=None, expect_absent=False)`
  - `dst: bytes` — the remote ref to update.
  - `src: ObjectId | None` — the local object to push; `None` means delete.
  - `force: bool` — force overwrite (no fast-forward check).
  - `delete: bool` — delete the remote ref (equivalent to `src=None`).
  - `expected_old: ObjectId | None` + `expect_absent: bool` — **force-with-lease** (safe
    force / create-only): the push is accepted only if the remote ref currently points at
    `expected_old` (or is absent when `expect_absent=True`); a stale ref returns
    `status="reject-stale"`.

#### Results — returned, not raised

`repo.push` returns a `PushReport` (.`results: list[PushRefResult]`, `.ok: bool`).
Rejections (non-fast-forward, hook-declined, stale lease, etc.) are **returned as data**
in each `PushRefResult.status`, not raised as exceptions. `.ok` is `True` only when every
ref in `results` has `status` of either `"ok"` or `"up-to-date"`.

`PushRefResult.status` values:

| Status | Meaning |
| --- | --- |
| `"ok"` | Ref updated successfully |
| `"up-to-date"` | Remote already at the pushed value; no change |
| `"reject-non-fast-forward"` | Not a fast-forward; use `force=True` or `PushSpec(force=True)` |
| `"reject-already-exists"` | Remote ref exists and a create-only push was requested |
| `"reject-fetch-first"` | Remote requires a fetch before pushing |
| `"reject-needs-force"` | Remote requires an explicit force flag |
| `"reject-stale"` | Force-with-lease failed: remote ref is not at `expected_old` |
| `"remote-rejected"` | Remote hook or policy rejected the ref |
| `"atomic-push-failed"` | This ref failed because another ref in an atomic push was rejected |

Only transport/auth/protocol failures raise exceptions (`NetworkError` /
`AuthenticationError`).

#### Progress callback

The `progress` callback (`Callable[[bytes], None]`) receives the remote's side-band-2
output — the `remote: …` lines printed by server-side hooks and diagnostics. Unlike
fetch (where the callback never fires), push progress **does** fire.

#### Push example

```python
import pylibgrit

repo = pylibgrit.Repository.open("/path/to/repo/.git", "/path/to/repo")

# Push local 'main' over https (token via kwarg or https://<token>@host/...).
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

## Supported Python / platforms

- **CPython 3.11+** — wheels are `abi3-py311`, so a single wheel works on 3.11 and
  every newer 3.x. (Standard abi3 wheels do not target free-threaded / no-GIL
  CPython.)
- **Linux (glibc)** — `manylinux_2_17` wheels for x86_64 and aarch64.
- **Linux (musl)** — `musllinux_1_2` wheels for x86_64 and aarch64 (Alpine and other
  musl-based distros / containers).
- **macOS** — arm64 (Apple silicon) wheels. Intel (x86_64) Macs install from the sdist,
  which compiles cleanly (grit-lib is Unix-oriented and its dependencies are pure-Rust);
  no prebuilt Intel wheel is shipped because GitHub's macOS-13 Intel runners are
  deprecated and unreliable.
- **Windows** — **deferred** until grit-lib gains Windows support (it currently
  depends on `libc`/`nix` and is Unix-oriented).

## Byte / text policy

Git data is binary: paths, ref names, author/committer fields, and messages are
not guaranteed to be UTF-8. pylibgrit therefore returns git data as **`bytes`** by
default and offers **opt-in decoded accessors** so decoding is always your explicit
choice:

- `TreeEntry.name`, `Repository.git_dir`/`work_tree`, `Reference.name`,
  `DiffEntry.old_path`/`new_path`, `Commit.message_bytes`, `Signature.name`/`email`
  return `bytes` (exact, byte-faithful).
- Decoded counterparts: `Signature.name_str`/`email_str` (UTF-8), and
  `Commit.message(encoding="utf-8", errors="strict")`, which decodes the verbatim
  message bytes with the encoding/errors you supply (pass the commit's own declared
  `encoding` header if it is not UTF-8).

## Exception hierarchy

All pylibgrit errors derive from a single base so you can catch broadly or narrowly:

```
GritError                 (base — also the catch-all for unmapped grit-lib errors)
├── RepositoryError       open/discover/format-validation/ref failures
├── ObjectNotFoundError   a requested object is not in the object database
├── InvalidObjectError    an object is corrupt or cannot be parsed
├── RefMismatchError      a ref's value failed a compare-and-swap / create-only check
├── NetworkError          a network-level failure during fetch/clone/ls-remote
└── AuthenticationError   authentication failed or credentials were rejected
```

I/O failures surface as `OSError` (with `errno` where available), and the
originating grit-lib message is included in the raised exception's message text.

## Known limitations

This is honest about where grit-lib 0.4.1's API constrains byte-fidelity or behavior:

- **Diff paths are UTF-8-decoded by grit-lib** (`Option<String>`), so a non-UTF-8
  diff path is *not* byte-preserved — unlike `TreeEntry.name`, which is exact
  bytes. (Normal ASCII/UTF-8 paths are unaffected.)
- **Annotated tags:** grit-lib 0.4.1's `parse_tag` rejects non-UTF-8 tags and
  exposes the tag name / tagger / message as UTF-8 `String` only — so tag
  identities and messages are not byte-preserved the way commits are.
- **Diffstat line counts** use grit-lib's `count_changes` (the `similar` crate),
  which treats a bare `\r` as a line break, whereas `git --numstat` splits on `\n`
  only. Stats can therefore diverge from git for files containing bare-CR content;
  ordinary `\n`-terminated text matches git exactly.
- **`resolve()` on an unknown revision** raises the base `GritError` (grit-lib
  returns a generic "unknown revision or path" message, not a typed not-found
  error).
- **Reference names are decoded lossily:** grit-lib returns ref names as a UTF-8
  `String` (via `to_string_lossy`), so non-UTF-8 ref names are *not* byte-faithful —
  distinct non-UTF-8 names can collide on the U+FFFD replacement character. This is
  unlike `TreeEntry.name`, which is exact bytes.
- **Annotated-tag write fidelity:** grit-lib's `TagData` stores the tag name, tagger, and
  message as UTF-8 `String` (no raw-byte fields), so written tags must be UTF-8 and the tag
  message's trailing newline is normalized — unlike commits, whose ids are byte-exact via the
  `author_raw`/`committer_raw`/raw-message escape hatches.
- **Ref compare-and-swap is best-effort (TOCTOU):** grit-lib 0.4.1 exposes no atomic
  compare-and-swap primitive, so `expected_old=`/`create=` do a read→compare→write without a
  held lock — they catch the common non-concurrent case but are not a hard guarantee against a
  concurrent writer. Atomic ref updates are planned for a later release.
- **No fetch transfer progress (grit-lib 0.4.1):** grit-lib hard-codes `no-progress` in
  its fetch request, so there is no progress callback for fetch/clone and one cannot be
  added at the binding layer. (Push progress — remote hook/diagnostic output — does fire.)
- **`fetch(tags="following")` shared-oid quirk (grit-lib 0.4.1):** if a tag points at
  the same commit as a fetched branch tip, grit-lib 0.4.1's tag-following can skip that
  commit's objects; workaround: use `tags="all"` or `tags="none"`. `clone()` always uses
  `tags="all"` and is unaffected.
- **Push is v1 only (grit-lib 0.4.1):** grit-lib's push implementation uses Git protocol
  v1; the server's v2 advertisement is ignored, and the client always negotiates v1. This
  is transparent in practice (all public hosts support v1), but it means v2-specific push
  features are unavailable.
- **Push: no SSH, no signed push, no submodule push:** `repo.push` supports `git://` and
  `https` only; `ssh://` / `git@` are not yet supported. Signed push (`--signed`) and
  submodule-aware push are also not yet exposed.
- **Push: string refspecs cannot express force-with-lease.** String shorthand (e.g. `"main"`,
  `"+a:b"`) cannot encode a `expected_old` constraint. Use a `PushSpec` object for
  force-with-lease.
- **Still out of scope (planned later phases):** working-tree checkout, SSH
  transport, shallow/depth clone, bare/mirror clone, submodules, and `insteadOf` URL
  rewriting are not yet exposed.

## Security considerations / untrusted repositories

pylibgrit is a thin binding over grit-lib 0.4.1 and **inherits its behavior**. It is
intended for **trusted, local repositories** and is **not hardened against
adversarial repository content**. The caveats below are upstream characteristics of
grit-lib 0.4.1 that cannot be fixed in the binding layer; they are candidates for
future hardening. Do not point pylibgrit at repositories you do not control without the
external mitigations noted.

- **No resource limits on object reads (DoS).** grit-lib decompresses loose objects
  with unbounded reads and preallocates packed-object buffers from attacker-controlled
  size headers. A maliciously crafted repository (a decompression bomb, or huge
  declared object sizes) can exhaust memory and abort the host Python process — and an
  out-of-memory abort **cannot be caught as a Python exception**. Do not read objects
  from untrusted repositories in-process without external resource limits
  (`ulimit`/cgroups) or sandboxing.
- **`Repository.discover()` may change the process working directory.** For
  repositories with a relative `core.worktree`, grit-lib's discovery can `chdir()` the
  process (and set `GIT_PREFIX`) and may not restore the CWD on failure. Prefer
  `Repository.open(git_dir, work_tree)` for untrusted or concurrency-sensitive use, and
  treat `discover()` on untrusted repositories with caution.
- **Reference enumeration follows symlinks without containment.** grit-lib's loose-ref
  walk follows symlinked directories and lacks depth/containment/visited checks, so a
  crafted repository can make `references()` traverse outside the repository or loop
  through large trees. Avoid enumerating refs on untrusted repositories.

The **write surface** (0.2.0) validates its own inputs at the binding layer — ref names (git
ref-format rules), index paths (no `..`/absolute components, no leading-slash), `Signature`
fields, and reflog/tag text — which closes the path-traversal, object/record-injection, and
malformed-index crash (grit-lib stack-overflow) vectors those calls would otherwise inherit.
One upstream write caveat remains: grit-lib writes objects through a deterministic temp file,
so concurrent *identical* object writes can race (this needs a grit-lib-level fix); distinct
concurrent writes are unaffected.

## How it maps to grit-lib

pylibgrit is a documented Python **façade** over grit-lib, not a literal 1:1
re-export. grit-lib 0.4.1 exposes a free-function / data-struct style API (public
fields, free functions taking `&Repository`/`&Odb`/`git_dir`, and `parse_*`
functions over raw bytes); pylibgrit constructs the ergonomic Python classes
(`Repository`, typed object views, `Reference`, `Signature`) on top of those
primitives. The complete, verified mapping — exact module paths, signatures,
return/error types, and the error → exception table — lives in
[`docs/superpowers/api-matrix.md`](docs/superpowers/api-matrix.md).

## Version compatibility

pylibgrit pins grit-lib **exactly** (`=` pin) with a committed `Cargo.lock` and
`--locked` builds for reproducibility (the published crate fully exposes read-core,
so no git-revision fallback is used).

| pylibgrit | grit-lib | pyo3 | Rust toolchain | Python (abi3) | License | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 0.4.0 | `=0.4.1` (MIT) | `=0.23.3` | 1.94.1 | ≥ 3.11 | MIT | + push over git:// and https |
| 0.3.0 | `=0.4.1` (MIT) | `=0.23.3` | 1.94.1 | ≥ 3.11 | MIT | + read-path networking |
| 0.2.0 | `=0.4.1` (MIT) | `=0.23.3` | 1.94.1 | ≥ 3.11 | MIT | + local write-core |
| 0.1.0 | `=0.4.1` (MIT) | `=0.23.3` | 1.94.1 | ≥ 3.11 | MIT | read-core release |

## Releasing

pylibgrit publishes to PyPI via [trusted publishing](https://docs.pypi.org/trusted-publishers/)
(OpenID Connect) — no API tokens are stored in the repo. Publishing a GitHub Release
runs [`.github/workflows/release.yml`](.github/workflows/release.yml), which rebuilds
the wheels + sdist with the same build recipe CI uses (released glibc Linux wheels
target the broader `manylinux_2_17` tag; musl wheels use `musllinux_1_2`), re-smoke-tests
every artifact, checks the tag and provenance, and uploads to PyPI over OIDC.

### One-time setup (maintainer, manual)

These cannot be automated and must be done once before the first release:

1. **Register the PyPI "pending publisher"** at
   <https://pypi.org/manage/account/publishing/>:
   - PyPI Project Name: `pylibgrit`
   - Owner: `linsomniac`
   - Repository name: `pylibgrit`
   - Workflow name: `release.yml`
   - Environment name: `pypi`

   For the dry-run path, repeat at <https://test.pypi.org/manage/account/publishing/>
   with Environment name `testpypi`. A pending publisher does **not** reserve the
   name, so cut the first real release promptly to claim `pylibgrit`.

2. **Create the protected GitHub Environments** (Settings → Environments). GitHub
   silently auto-creates an *unprotected* environment if a workflow merely
   references one, so create them explicitly:
   - `pypi` — restrict deployments to protected `v*` tags (back it with a repository
     ruleset that protects `v*` tags). Required-reviewer protection is impractical
     for a solo maintainer (self-review is blocked); add a reviewer if the project
     gains maintainers.
   - `testpypi` — restrict deployments to the `main` branch.

### Cutting a release

1. Bump the version in **both** `Cargo.toml` (`[package] version`) **and**
   `Cargo.lock`: edit `Cargo.toml`, then run `cargo update -p pylibgrit` (or `cargo
   build` without `--locked`) so the lockfile matches. The workflow's `cargo
   metadata --locked` version guard fails if `Cargo.lock` is stale.
2. Commit to `main` and push.
3. Create a GitHub Release with tag **`vX.Y.Z`** (final releases only — the version
   guard rejects anything that is not `vX.Y.Z`). Publishing the release builds and
   smoke-tests the five wheels + sdist, verifies `tag == crate version` and that the
   commit is on `main`, and publishes to PyPI automatically.

### TestPyPI dry-run (optional)

Trigger the workflow manually (Actions → Release → "Run workflow") to build and
publish to **TestPyPI** instead of PyPI. Because PyPI/TestPyPI filenames are
immutable, a repeat dry-run needs a **unique version** (bump the patch). A green
dry-run validates the build/smoke/OIDC *mechanics*, but TestPyPI uses a separate
trusted-publisher registration, so it does **not** prove the real-PyPI config — the
first live release does.

## License

MIT — matching grit-lib (also MIT). See [`LICENSE`](LICENSE) if present, and the
license metadata in `pyproject.toml` / `Cargo.toml`.
