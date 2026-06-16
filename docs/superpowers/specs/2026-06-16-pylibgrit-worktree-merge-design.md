# pylibgrit Phase B Design — Worktree & Merge

**Date:** 2026-06-16
**Type:** Design spec (drives an implementation plan)
**Status:** Approved — ready for writing-plans
**Phase:** B of A→B→C (Phase A "local write-core" shipped at 0.2.0; see the Phase A
spec `docs/superpowers/specs/2026-06-14-pylibgrit-write-core-design.md` §8 roadmap)

## Goal

Extend pylibgrit from a *local object/ref write surface* (Phase A) to a *scriptable
local git working environment*: create repositories, lay a tree down into a working
tree, three-way-merge trees and commits, record commits that advance the current branch,
point tag refs, and harden ref compare-and-swap from best-effort to truly atomic. All
local — no network (that is Phase C). Like Phase A, every multi-step workflow is
assembled in the binding over grit-lib 0.4.1 plumbing, because grit-lib ships **no
porcelain** (`git_lib::repo::Repository` has no `commit()`/`checkout()`/`merge()`).

## Background

The Phase A spec established the controlling fact (still true for B): grit-lib 0.4.1 is
write-capable plumbing with no porcelain — the binding assembles each workflow. Phase B
verified the exact 0.4.1 surface for every piece it needs:

- **init** — `repo::init_repository(path, bare, initial_branch, template_dir, ref_storage)
  -> Result<Repository>` (`src/repo.rs`).
- **checkout** — only *per-file* primitives exist in `porcelain::checkout`:
  `write_to_worktree(work_tree, rel_path, data, mode)`, `apply_index_file_mode`,
  `prepare_parent_dirs_for_checkout`, `remove_empty_parent_dirs`. There is **no**
  `checkout_tree`/`checkout_index` — the binding walks the tree and writes each blob.
- **merge** — `merge_trees::merge_trees_three_way(repo, base_tree, ours_tree, theirs_tree,
  favor, ws, diff_algorithm, presentation) -> Result<TreeMergeOutput>` where
  `TreeMergeOutput { index: Index, conflict_content: BTreeMap<Vec<u8>, ObjectId> }`. Merge
  is **tree/index-level**: it produces a (possibly unmerged) index plus conflict-marker
  blob oids; it does not touch the working tree or create a commit. Merge-base helpers
  live in `merge_base.rs` (`merge_bases_all`, `merge_bases_first_vs_rest`, `is_ancestor`,
  …); `MergeFavor` is in `merge_file.rs`.
- **atomic CAS** — `refs::lock_path_for_ref(path) -> PathBuf` is public and `refs::write_ref`
  already locks via `O_CREAT|O_EXCL` (`create_new`) on `<ref>.lock`, then renames. A
  binding-held lockfile (acquire `.lock` → read current under the lock → compare → write →
  rename) is therefore reachable and gives true atomicity against git and grit-lib, which
  use the same `.lock` protocol. Current value is read with `refs::read_raw_ref` /
  `refs::resolve_ref` (loose + packed).

The read+write façade is an OO surface (`Repository`, `Odb`, `Index`, `Signature`, …) with
a `GritError` hierarchy and GIL-releasing operations. Phase B extends that same façade.

## Scope and phasing

**In scope (Phase B):**

- **Worktree:** `Repository.init`; `repo.checkout_tree` (non-destructive overlay);
  `repo.write_to_worktree` (low-level escape hatch); bare-repo guards on worktree ops.
- **Merge:** `repo.merge_base`; `repo.merge_trees` (raw tree primitive); `repo.merge_commits`
  (convenience that computes the base); a `MergeResult` value-object.
- **Commit porcelain:** `repo.commit_index` (write tree from index → commit with the
  current branch tip as parent → atomically advance the branch → reflog).
- **Tag-ref porcelain:** `repo.create_lightweight_tag`; `repo.create_annotated_tag`.
- **Atomic-CAS upgrade:** `update_ref`/`delete_ref`'s `expected_old=`/`create=` move from
  Phase A's best-effort read-compare-write to a binding-held lockfile.

**Out of scope (Phase B, deferred to C or later):** fetch/push/clone and any network;
destructive "make the worktree exactly match the tree" (delete extras / `read-tree -u`
removal); recursive merge with a *virtual* merge base (criss-cross); rename/copy detection
tuning beyond grit-lib defaults; index-level conflict *resolution* helpers beyond reporting;
the `Odb` mem-overlay dry-run; signing.

## Design decisions

Settled during brainstorming and binding for the plan:

| Decision | Choice | Rationale |
| --- | --- | --- |
| **Architecture** | Methods on `Repository`; `MergeResult` is a returned value-object. | Identical in style to the Phase A façade; one coherent object model. |
| **Checkout safety** | **Non-destructive overlay**: `checkout_tree` writes/overwrites the tree's blobs but never deletes files absent from the tree; refuses to clobber an existing file unless `force=True`; `update_index=True` by default. | Never silently destroys untracked work; plumbing-faithful. A destructive "match" mode can come later. |
| **Merge surface** | **Both levels**: `merge_base` + raw `merge_trees(base, ours, theirs)` + `merge_commits(ours, theirs)` convenience. Returns a `MergeResult`. | Covers both the `git merge` mental model and the raw primitive. |
| **Commit porcelain** | **Index-based** `commit_index(*, message, parents=None, …)`: tree from the index, parent = current HEAD branch tip (`[]` if unborn), atomic-CAS branch advance, reflog. `parents=` adds extra parents for merge commits. | Mirrors `git commit`. `repo.commit` is already the read accessor, so the name is `commit_index`. |
| **Atomic CAS** | `update_ref`/`create=` and `commit_index`'s branch advance are **truly atomic** via a binding-held `<ref>.lock`. `delete_ref(expected_old=)` is atomic for loose refs; the packed-refs case delegates after the verify (documented residual window). | Branch advancement — where races matter — is fully atomic without an upstream primitive. |
| **Merge base** | First merge base only (no virtual/recursive base). | YAGNI for B; criss-cross divergence from git documented and asserted only on clean / simple-conflict oracle cases. |
| **init default** | `initial_branch=b"main"`. | Matches this project's own default branch; tests pass an explicit branch to match the git oracle. |

## 1. Architecture & module layout

No new dependencies, no Cargo features, no network. Each new method assembles a grit-lib
plumbing workflow in Rust with GIL release and error mapping.

| File | Change | Adds |
| --- | --- | --- |
| `src/checkout.rs` | **new** | tree-walk → `write_to_worktree` assembly; overlay policy (no-delete, no-clobber-without-force); mode application; gitlink skip |
| `src/merge.rs` | **new** | `MergeResult` pyclass (wraps the merged `Index` + `conflict_content`); merge assembly (base computation, tree lookups, `favor` mapping, conflict extraction) |
| `src/refs.rs` | extend | `atomic_cas_write` / `atomic_cas_delete` (binding-held `lock_path_for_ref` + `read_raw_ref`/`resolve_ref`); rewire `update_ref`/`delete_ref` CAS paths to use them |
| `src/repository.rs` | extend | wire `init` (staticmethod), `checkout_tree`, `write_to_worktree`, `commit_index`, `create_lightweight_tag`, `create_annotated_tag`, `merge_base`, `merge_trees`, `merge_commits` |
| `src/lib.rs` | extend | register the `MergeResult` pyclass |
| `python/pylibgrit/__init__.py` | extend | re-export `MergeResult` and the new methods |
| `python/pylibgrit/__init__.pyi` | extend | stubs for everything in §2 (kept in sync; `stubtest` gate, no allowlist) |

**Concurrency model (unchanged from Phase A):** `Odb` writes through `&self`; the `Index`
inside a `MergeResult` is a binding-owned value behind a `Mutex`. Worktree and ref writes
release the GIL via `allow_threads` except where a `!Send` `MutexGuard` must be held (then
the guarded section runs under the GIL, exactly as Phase A's `Index` methods do). On-disk
effects are immediate and atomic per object/ref (temp-file + rename; `<ref>.lock` + rename).

## 2. Public API surface (additions to `__init__.pyi`)

```python
class Repository:
    @staticmethod
    def init(path: str | bytes | os.PathLike[str], *,
             bare: bool = False, initial_branch: bytes = b"main") -> Repository: ...

    # --- worktree (require a non-bare repo with a work_tree; else RepositoryError) ---
    def checkout_tree(self, tree: ObjectId, *,
                      force: bool = False, update_index: bool = True) -> None: ...
        # Overlay: writes every blob in `tree` to the work tree (creating parent dirs),
        # applying the entry's mode. Never deletes files absent from `tree`. Refuses to
        # overwrite an existing file unless force=True (FileExistsError otherwise).
        # Gitlink (submodule, mode 0o160000) entries are skipped. When update_index=True
        # the repo index is updated to match the written entries and persisted.

    def write_to_worktree(self, rel_path: bytes, data: bytes, mode: int) -> None: ...
        # Low-level single-file escape hatch over porcelain::checkout::write_to_worktree.

    # --- merge (tree/index-level; does NOT touch the work tree or create a commit) ---
    def merge_base(self, a: ObjectId, b: ObjectId) -> ObjectId | None: ...   # None if unrelated
    def merge_trees(self, base: ObjectId, ours: ObjectId, theirs: ObjectId, *,
                    favor: str | None = None) -> MergeResult: ...   # favor: None|"ours"|"theirs"|"union"
    def merge_commits(self, ours: ObjectId, theirs: ObjectId, *,
                      favor: str | None = None) -> MergeResult: ...  # base = first merge_base

    # --- commit porcelain ---
    def commit_index(self, *, message: bytes,
                     parents: list[ObjectId] | None = None,        # extra parents (merge); base parent
                                                                    # = current branch tip is implicit
                     author: Signature | None = None, committer: Signature | None = None,
                     author_raw: bytes | None = None, committer_raw: bytes | None = None,
                     encoding: bytes | None = None) -> ObjectId: ...

    # --- tag-ref porcelain ---
    def create_lightweight_tag(self, name: bytes, target: ObjectId, *,
                               force: bool = False) -> None: ...
    def create_annotated_tag(self, name: bytes, target: ObjectId, target_kind: ObjectKind, *,
                             message: bytes,
                             tagger: Signature | None = None, tagger_raw: bytes | None = None,
                             force: bool = False) -> ObjectId: ...   # returns the tag-object oid

@final
class MergeResult:
    @property
    def index(self) -> Index: ...                       # merged Index (may hold unmerged stages)
    @property
    def has_conflicts(self) -> bool: ...
    @property
    def conflicts(self) -> list[bytes]: ...             # conflicted paths (sorted)
    def conflict_blob(self, path: bytes) -> ObjectId | None: ...   # conflict-marker blob oid, if any
    def write_tree(self) -> ObjectId: ...               # RepositoryError if has_conflicts
```

### grit-lib primitives each method wraps

| pylibgrit method | grit-lib plumbing |
| --- | --- |
| `Repository.init` | `repo::init_repository(path, bare, initial_branch, None, "files")` |
| `checkout_tree` | recursively `objects::parse_tree` from `tree`; for each blob: `odb` read → `porcelain::checkout::{prepare_parent_dirs_for_checkout, write_to_worktree, apply_index_file_mode}`; gitlink skipped; optional index update via the Phase A `Index` |
| `write_to_worktree` | `porcelain::checkout::write_to_worktree(work_tree, rel, data, mode)` |
| `merge_base` | first of `merge_base::merge_bases_all(repo, &[a, b])` (None if empty) |
| `merge_trees` | `merge_trees::merge_trees_three_way(repo, base, ours, theirs, favor, WhitespaceMergeOptions::default(), None, default presentation)` |
| `merge_commits` | resolve each commit's tree; `base = merge_base(ours, theirs)` (empty tree if None) → `merge_trees` |
| `commit_index` | `write_tree::write_tree_from_index` → `objects::serialize_commit` (parents = `[branch tip?]` + extra) → `Odb::write(Commit, …)` → atomic-CAS advance `refs/heads/<branch>` → `refs::append_reflog` |
| `create_lightweight_tag` | atomic `update_ref(refs/tags/<name>, target, create=!force)` |
| `create_annotated_tag` | Phase A `create_tag` (object) → atomic `update_ref(refs/tags/<name>, tag_oid, create=!force)` |
| `update_ref`/`delete_ref` CAS | `atomic_cas_write` / `atomic_cas_delete` (§4) |

## 3. Data flow

### Lay a tree into a fresh repo and commit it

```python
repo = pylibgrit.Repository.init("/tmp/demo", initial_branch=b"main")     # 1. init
blob = repo.odb.write(ObjectKind.BLOB, b"hello\n")
idx  = repo.index()
idx.add(b"greeting.txt", blob, mode=0o100644); idx.write()
sig  = pylibgrit.Signature(b"Ada", b"ada@x.io", (1718000000, 0))
oid  = repo.commit_index(message=b"init\n", author=sig, committer=sig)    # 2. commit (advances main)
repo.checkout_tree(repo.commit(oid).tree)                                  # 3. materialize work tree
```

### Three-way merge two branches

```python
mb   = repo.merge_base(ours_commit, theirs_commit)
res  = repo.merge_commits(ours_commit, theirs_commit)                      # tree-level merge
if res.has_conflicts:
    for path in res.conflicts:
        ...  # caller resolves: edit res.index, or write resolved blob + res.index.add(...)
tree = res.write_tree()                                                    # errors if still conflicted
merge_oid = repo.commit_index(message=b"merge\n", parents=[theirs_commit], # ours = branch tip (implicit)
                              author=sig, committer=sig)
```

`merge_commits` itself touches no ref and no worktree; the caller advances the branch with
`commit_index` (which supplies the current branch tip as the first parent and `theirs` as
the extra parent), exactly as Phase A keeps `create_commit` pure.

## 4. Atomic-CAS upgrade

Phase A implemented `expected_old=`/`create=` as best-effort read-compare-write (a
documented TOCTOU window). Phase B replaces that with a binding-held lockfile.

**`atomic_cas_write(git_dir, refname, new_oid, expected_old, create)`** (used by
`update_ref`'s CAS/create paths and by `commit_index`'s branch advance):

1. Resolve the loose ref path; `lock = refs::lock_path_for_ref(path)`.
2. Acquire the lock: open `lock` with `create_new` (`O_CREAT|O_EXCL`). If it already
   exists, another writer holds it → `RepositoryError` with git's wording
   `cannot lock ref '<name>'`.
3. **Under the lock**, read the current value via `refs::read_raw_ref` / `refs::resolve_ref`
   (consulting loose **and** packed-refs).
4. Compare: `create=True` requires the ref to be absent; `expected_old=<oid>` requires the
   current value to equal `<oid>`. On mismatch, remove the lock file and raise
   `RefMismatchError` (message carries ref name + expected vs actual).
5. On match, write `<new_oid hex><LF>` (hex width per the repo hash algo, SHA-1 40 /
   SHA-256 64) into the lock file, then `rename(lock, path)`.

The plain default path (`expected_old=None, create=False`) is unchanged — it keeps calling
grit-lib's `refs::write_ref`, which locks atomically on its own.

**`atomic_cas_delete(git_dir, refname, expected_old)`** (used by `delete_ref`'s CAS path):
acquire `<ref>.lock`, verify the current value equals `expected_old` under the lock
(else `RefMismatchError`), then remove the loose ref file. If the ref *also* exists in
packed-refs, removal delegates to grit-lib's `refs::delete_ref` after the verify — a small,
**documented residual TOCTOU window for the packed-only case only**. Loose-ref deletion is
fully atomic; update/create (the cases that matter for branch advancement) are fully atomic.

**Implementation note for the plan:** confirm the exact lock-acquire / current-value-read
calls against grit-lib 0.4.1 before finalizing (the §6 Phase A caveat carries over). If a
held-lock read helper richer than `read_raw_ref`/`resolve_ref` exists, prefer it. Never
leave a stale `<ref>.lock` on any error path — always clean up the lock before raising.

## 5. Error handling & guards

The Phase A hierarchy (`GritError` → `RepositoryError`, `ObjectNotFoundError`,
`InvalidObjectError`, `RefMismatchError`) is reused. No new exception type is introduced.

- **Conflicts are not errors.** A conflicted merge returns a normal `MergeResult` with
  `has_conflicts=True`; only `MergeResult.write_tree()` raises (`RepositoryError`) when
  called on a conflicted result.
- **Bare-repo / no-worktree guard.** `checkout_tree` and `write_to_worktree` raise
  `RepositoryError` when `repo.is_bare` or `work_tree is None`. (Phase A's `Index.stage`
  already guards this way.) `commit_index` needs only the index + odb + refs and is allowed
  on a bare repo.
- **Clobber guard.** `checkout_tree` raises `FileExistsError` (an `OSError` subclass) when an
  existing work-tree file would be overwritten and `force=False`.
- **Lock contention** surfaces as `RepositoryError` (`cannot lock ref '<name>'`).
- **Validation (binding-layer, pre-write).** Reuse the Phase A validators: `commit_index`
  requires `author` XOR `author_raw` (committer likewise) and a non-empty `message`; ref
  names (`refs/tags/<name>`) pass `check_ref_format`; tag names and any reflog message reject
  NUL/CR/LF. `merge_trees`/`merge_commits` reject an unknown `favor` string with `ValueError`.
- **`OSError`** — grit-lib `Error::Io` maps to `OSError` with errno, as in the read/write core.

Because object and ref writes are individually atomic, a mid-sequence failure (e.g. a
conflict found after some blobs were written during an aborted operation) leaves orphaned
loose objects (harmless, reclaimed by `git gc`) but never a corrupt ref or index. A partial
`checkout_tree` may leave already-written files on disk (overlay semantics; no rollback) —
documented.

## 6. Testing strategy

The git-oracle approach (mirror each operation against real `git`, reuse `tests/gitlib.py`,
pin timestamps with `GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE` for byte-exact parity) extends to
Phase B. All tests run in tempdirs.

| New test file | Asserts |
| --- | --- |
| `tests/test_init.py` | `Repository.init(tmp)` → `git rev-parse --git-dir` recognizes it; HEAD is symbolic → `refs/heads/<branch>`; bare variant has no work tree; `initial_branch` honored |
| `tests/test_checkout.py` | after `checkout_tree`, each file's bytes + mode match the tree (vs `git checkout-index`); a pre-existing untracked file survives (overlay); `force=False` over an existing file → `FileExistsError`, `force=True` overwrites; executable bit applied; gitlink entry skipped; bare repo → `RepositoryError`; `update_index=True` makes `git ls-files --stage` match |
| `tests/test_merge.py` | `merge_base(a, b)` == `git merge-base`; a clean three-way merge tree oid == `git merge-tree --write-tree`; a conflicting merge → `has_conflicts=True` and `conflicts` matches the oracle's unmerged paths; `favor="ours"/"theirs"` resolves as git does; `merge_trees` with an empty base behaves; `write_tree()` on a conflicted result raises |
| `tests/test_commit_index.py` | `commit_index` advances `refs/heads/<branch>` (`git rev-parse`); commit oid == oracle (`git commit-tree` with pinned dates); parent linkage correct; unborn-branch first commit (parents `[]`); merge commit via `parents=[...]`; a reflog entry appears (`git reflog`); CAS-loss (tip moved underneath) → `RefMismatchError` |
| `tests/test_atomic_cas.py` | CAS mismatch → `RefMismatchError`, ref unchanged; create-only on an existing ref → raise; a pre-existing `<ref>.lock` → `RepositoryError` (contention); no stale `.lock` after any failure; **threaded race advancing one branch → exactly one winner, ref ends at one of the two values** |
| `tests/test_tag_ref.py` | lightweight tag → `git rev-parse refs/tags/<n>` == target; annotated tag → tag object (`git cat-file -p`) + `refs/tags/<n>` points at it; `force=False` over an existing tag → `RefMismatchError`, `force=True` moves it |

Quality gates stay green and identical to Phase A: `ruff format`/`check`, `mypy python tests`,
`python -m mypy.stubtest pylibgrit` (no allowlist), `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, `pytest`.

> **Oracle caveat:** `git merge-tree --write-tree` requires git ≥ 2.38. The merge tests
> guard on git version and `pytest.skip` below it. Criss-cross (multiple-merge-base) cases
> are intentionally *not* oracle-compared (Phase B uses the first base only).

## 7. File/responsibility summary

- **Working-tree materialization** lives in `src/checkout.rs` — the tree-walk + overlay
  policy, isolated from ref/merge logic.
- **Three-way merge** lives in `src/merge.rs` — base computation, the `merge_trees_three_way`
  call, `favor` mapping, and the `MergeResult` value-object.
- **Atomic CAS** lives with the other ref logic in `src/refs.rs`.
- **Porcelain wiring** (`init`, `commit_index`, the tag and merge methods) is surfaced on
  `Repository` in `src/repository.rs`, delegating to the units above.

Each unit has one responsibility and a Python-facing interface testable against the git
oracle independently.

## 8. Known limitations & risks

- **Merge is tree/index-level only.** It computes a merged index/tree; it does not modify
  the working tree or create the merge commit. The caller resolves conflicts in the returned
  index, calls `write_tree()`, then `commit_index(parents=[theirs])`.
- **First merge base only.** No recursive/virtual base for criss-cross histories; results
  may differ from git's recursive strategy there. Documented; oracle parity asserted only on
  single-base (clean / simple-conflict) cases.
- **`checkout_tree` is overlay-only.** It never deletes files absent from the tree, and a
  partial failure leaves already-written files in place (no rollback). A destructive
  "match the tree exactly" mode is deferred.
- **Atomic-CAS residual window (delete, packed-only).** `update_ref`/`create=` and
  `commit_index` are fully atomic. `delete_ref(expected_old=)` is atomic for loose refs;
  a ref existing only in packed-refs has a small documented residual window during deletion.
- **No transactional multi-ref update.** Each ref op is independent (carried over from
  Phase A). Building a merge commit and advancing a branch are still two observable steps.

## 9. Load-bearing references

grit-lib 0.4.1: `repo::init_repository` (`src/repo.rs`); `porcelain::checkout::{write_to_worktree,
apply_index_file_mode, prepare_parent_dirs_for_checkout}` (`src/porcelain/checkout.rs`);
`merge_trees::{merge_trees_three_way, TreeMergeOutput, WhitespaceMergeOptions,
TreeMergeConflictPresentation}` (`src/merge_trees.rs`); `merge_file::MergeFavor`;
`merge_base::{merge_bases_all, is_ancestor}` (`src/merge_base.rs`);
`refs::{lock_path_for_ref, write_ref, delete_ref, read_raw_ref, resolve_ref, append_reflog}`
(`src/refs.rs`); `objects::{parse_tree, serialize_commit, serialize_tag}`,
`write_tree::write_tree_from_index`. Phase A spec:
`docs/superpowers/specs/2026-06-14-pylibgrit-write-core-design.md` (§8 roadmap, byte-exact OID
rules, PyO3 binding gotchas). Canonical assembly pattern: grit-lib's `examples/commit_tree.rs`.
