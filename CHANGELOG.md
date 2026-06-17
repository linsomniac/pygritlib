# Changelog

All notable changes to pylibgrit are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/), and this project adheres to
[Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-06-17

### Added

- **Read-path networking** — `ls_remote`, `fetch`, and `Repository.clone` over **git://**
  and **https** (bundled `http-ureq` / rustls; no system OpenSSL or libcurl required).
  All three entry points accept `username=` / `password=` kwargs, URL userinfo
  (`https://<token>@host/...`), or git credential helpers (`use_credential_helpers=True`,
  the default).
  - `pylibgrit.ls_remote(url, *, username, password, use_credential_helpers, heads,
    tags) -> list[RemoteRef]` — list a remote's refs without a local repo.
  - `Repository.clone(url, path, *, branch, username, password,
    use_credential_helpers) -> Repository` — worktree clone; fetches all tags (`tags="all"`,
    matching `git clone` behaviour); sets `branch.<name>.remote`/`merge` upstream tracking.
  - `repo.fetch(url, refspecs=None, *, tags="following", prune, username, password,
    use_credential_helpers) -> FetchReport` — default refspec
    `+refs/heads/*:refs/remotes/origin/*`; `tags` ∈ `{"none","following","all"}`.
  - New value objects: `RemoteRef` (`.name: bytes`, `.oid: ObjectId`, `.symref_target:
    bytes | None`), `RefUpdate` (`.remote_ref`, `.local_ref`, `.old_oid`, `.new_oid`,
    `.mode`, `.note`), and `FetchReport` (`.updates: list[RefUpdate]`,
    `.default_branch: bytes | None`).
  - New exceptions: `NetworkError` and `AuthenticationError` (both subclass `GritError`).

### Known limitations

- **No transfer progress:** grit-lib 0.4.1 hard-codes `no-progress` in its fetch
  request; a progress callback cannot be added at the binding layer.
- **`fetch(tags="following")` shared-oid quirk:** if a tag points at the same commit as
  a fetched branch tip, grit-lib 0.4.1's tag-following can skip that commit's objects.
  Workaround: `tags="all"` or `tags="none"`. `clone()` uses `tags="all"` and is
  unaffected.
- **Not yet supported:** push, SSH transport, shallow/depth clone, bare/mirror clone,
  submodules, `insteadOf` URL rewriting.

## [0.2.0] - 2026-06-16

### Added

- **Local write-core** — a write surface over grit-lib 0.4.1 plumbing, all in-process
  (no external `git` binary). `create_commit`/`create_tag` produce byte-identical object
  ids to git.
  - `Odb.write(kind, data)` / `Odb.hash(kind, data)` — write a loose object / compute its
    oid without writing.
  - Constructable `Signature(name, email, when)` with a `.raw` wire-bytes accessor.
  - `Index` (via `repo.index()`) with `add`, `stage`, `add_entry`, `remove`, `write`,
    `write_tree`, `len()` / iteration; plus a constructable `IndexEntry`.
  - `Repository.create_commit(...)` and `Repository.create_tag(...)` — a structured
    `Signature` or raw header bytes (`author_raw`/`committer_raw`/`tagger_raw`) for
    byte-exact ids.
  - Ref mutation: `update_ref` (overwrite / `create=` create-only / `expected_old=`
    compare-and-swap), `delete_ref`, `set_head`, `set_symbolic_ref`, `append_reflog`, and
    opt-in reflog (`message=` / `signer=`) on ref updates.
  - `RefMismatchError` exception for compare-and-swap / create-only failures.

### Security

- Write inputs are validated at the binding layer: `Signature` rejects `<`/`>`/NUL/newline
  and out-of-range or non-minute timezone offsets (closes an `i32::MIN` panic and ident
  injection); index paths reject `..` / absolute / leading-slash components (closes a
  grit-lib stack-overflow and a worktree escape); ref names are validated by git's
  ref-format rules (closes a path traversal); reflog messages and tag names reject
  NUL/CR/LF.

### Fixed

- A fresh SHA-256 repository now gets a correctly-typed SHA-256 index from `repo.index()`
  instead of a SHA-1 one.

### Known limitations

- Ref compare-and-swap is best-effort (TOCTOU) — grit-lib 0.4.1 has no atomic CAS
  primitive. Written annotated tags must be UTF-8 (grit-lib `TagData` is String-only).
  Worktree checkout, three-way merge, repository init, and networking remain out of scope
  (planned later phases).

## [0.1.0] - 2026-06-14

### Added

- Initial **read-core** release: discover/open repositories, read objects
  (commit/tree/blob/tag), list/resolve references, walk history, diff commits, and read
  config — a thin Python façade over grit-lib 0.4.1, shipped as an `abi3` (CPython 3.11+)
  wheel with no external `git` binary or system C libraries required.

[0.3.0]: https://github.com/linsomniac/pylibgrit/releases/tag/v0.3.0
[0.2.0]: https://github.com/linsomniac/pylibgrit/releases/tag/v0.2.0
[0.1.0]: https://github.com/linsomniac/pylibgrit/releases/tag/v0.1.0
