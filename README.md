# pylibgrit

Native Python bindings for [`grit-lib`](https://crates.io/crates/grit-lib) — the
core Rust library of [gitbutlerapp/grit](https://github.com/gitbutlerapp/grit), a
from-scratch reimplementation of Git in Rust. pylibgrit is built with
[PyO3](https://pyo3.rs) and packaged as an `abi3` wheel with
[maturin](https://maturin.rs). This first release is a thin, **read-core** Python
façade over grit-lib: discover/open repositories, read objects
(commit/tree/blob/tag), list and resolve references, walk history, diff commits,
and read config — all in-process, with no external `git` binary required at
runtime and no system C libraries to build.

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
└── InvalidObjectError    an object is corrupt or cannot be parsed
```

I/O failures surface as `OSError` (with `errno` where available), and the
originating grit-lib message is included in the raised exception's message text.

## Known limitations (v1)

This release is honest about where grit-lib 0.4.1's API constrains byte-fidelity or
behavior:

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
- **Mutating operations are out of scope** for this read-core release: writing
  objects/refs/index, commit creation, merge, and any networking are not exposed.

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
   - Repository name: `pygrit`
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
