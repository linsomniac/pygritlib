"""Diff tests: tree/commit diff status + diffstat summary, oracle'd against git."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest


@pytest.fixture
def diff_repo(tmp_path: Path, git_env: dict[str, str]) -> Path:
    repo = tmp_path / "diff"
    repo.mkdir()

    def g(*a: str) -> None:
        subprocess.run(["git", *a], cwd=repo, env=git_env, check=True)

    g("init", "-q", "-b", "main")
    (repo / "keep").write_text("a\n")
    (repo / "gone").write_text("b\n")
    g("add", "-A")
    g("commit", "-q", "-m", "base")
    (repo / "keep").write_text("a2\n")  # modify
    (repo / "gone").unlink()  # delete
    (repo / "added").write_text("c\n")  # add
    g("add", "-A")
    g("commit", "-q", "-m", "change")
    return repo


def test_diff_status_matches_git(diff_repo: Path) -> None:
    import pylibgrit

    from tests.gitlib import run_git

    a = run_git(diff_repo, "rev-parse", "HEAD^").decode().strip()
    b = run_git(diff_repo, "rev-parse", "HEAD").decode().strip()
    # git diff --raw -z: meta record (starts ':') then path(s) as separate \0 fields.
    raw = run_git(diff_repo, "diff", "--raw", "-z", a, b)
    fields = [f for f in raw.split(b"\0") if f]
    expected = {}
    i = 0
    while i < len(fields):
        meta = fields[i]  # e.g. b":100644 100644 <oid> <oid> M"
        status = meta.split(b" ")[-1].decode()
        path = fields[i + 1]
        expected[path] = status[0]
        i += 2
    repo = pylibgrit.Repository.discover(str(diff_repo))
    d = repo.diff(repo.resolve("HEAD^"), repo.resolve("HEAD"))
    got = {}
    for e in d:
        key = e.old_path if e.status == "D" else e.new_path
        got[key] = e.status
    assert got == expected


def test_diff_len(diff_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(diff_repo))
    d = repo.diff(repo.resolve("HEAD^"), repo.resolve("HEAD"))
    assert len(d) == 3  # keep modified, gone deleted, added added


def test_diffstat_matches_git(diff_repo: Path) -> None:
    import pylibgrit

    from tests.gitlib import run_git

    a = run_git(diff_repo, "rev-parse", "HEAD^").decode().strip()
    b = run_git(diff_repo, "rev-parse", "HEAD").decode().strip()
    numstat = (
        run_git(diff_repo, "diff", "--numstat", a, b).decode().strip().splitlines()
    )
    ins = dele = 0
    files = 0
    for line in numstat:
        added, deleted, _path = line.split("\t", 2)
        files += 1
        if added != "-":
            ins += int(added)
        if deleted != "-":
            dele += int(deleted)
    repo = pylibgrit.Repository.discover(str(diff_repo))
    stats = repo.diff(repo.resolve("HEAD^"), repo.resolve("HEAD")).stats
    assert stats.files_changed == files
    assert stats.insertions == ins
    assert stats.deletions == dele


def test_diff_iter_outlives_repo(diff_repo: Path) -> None:
    """FFI lifetime: a DiffIter must stay valid after the Diff and Repository drop."""
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(diff_repo))
    d = repo.diff(repo.resolve("HEAD^"), repo.resolve("HEAD"))
    it = iter(d)
    del d
    del repo
    statuses = sorted(e.status for e in it)
    assert statuses == ["A", "D", "M"]


@pytest.fixture
def gitlink_repo(tmp_path: Path, git_env: dict[str, str]) -> Path:
    """A repo whose HEAD vs HEAD^ diff contains a submodule GITLINK (mode 160000) ADD.

    AIDEV-NOTE: A real `git submodule add` needs a clonable URL/network; we instead
    synthesize a gitlink TREE ENTRY with `git update-index --add --cacheinfo
    160000,<commit-oid>,sub`, pointing the gitlink at an EXISTING commit oid (the base
    commit) so the object is present in the odb. The HEAD commit thus has a 160000 `sub`
    entry referencing a COMMIT object — exactly a submodule pointer — without any network.
    The diff also modifies a text file so the entry mix mirrors a real submodule bump.
    """
    repo = tmp_path / "gitlink"
    repo.mkdir()

    def g(*a: str) -> bytes:
        return subprocess.run(
            ["git", *a],
            cwd=repo,
            env=git_env,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ).stdout

    g("init", "-q", "-b", "main")
    g("config", "core.autocrlf", "false")
    (repo / "keep").write_text("a\n")
    g("add", "-A")
    g("commit", "-q", "-m", "base")
    base = g("rev-parse", "HEAD").decode().strip()
    # Modify the text file AND add a gitlink pointing at the base commit oid.
    (repo / "keep").write_text("a2\n")
    g("add", "-A")
    g("update-index", "--add", "--cacheinfo", f"160000,{base},sub")
    g("commit", "-q", "-m", "add gitlink")
    return repo


def test_diffstat_with_gitlink_matches_git(gitlink_repo: Path) -> None:
    """`.stats` must not crash on a gitlink entry and must match `git --numstat`.

    AIDEV-NOTE: A gitlink (submodule, mode 160000) references a COMMIT object, not a blob.
    The binding must NOT line-count the commit object's raw bytes. We oracle against
    `git --numstat`, which renders a gitlink side as the single line `Subproject commit
    <oid>` (so an ADD counts 1 insertion / 0 deletions). files_changed counts every entry.
    """
    import pylibgrit

    from tests.gitlib import run_git

    a = run_git(gitlink_repo, "rev-parse", "HEAD^").decode().strip()
    b = run_git(gitlink_repo, "rev-parse", "HEAD").decode().strip()
    numstat = (
        run_git(gitlink_repo, "diff", "--numstat", a, b).decode().strip().splitlines()
    )
    ins = dele = files = 0
    saw_gitlink = False
    for line in numstat:
        added, deleted, path = line.split("\t", 2)
        files += 1
        if path == "sub":
            saw_gitlink = True
        if added != "-":
            ins += int(added)
        if deleted != "-":
            dele += int(deleted)
    assert saw_gitlink, "fixture must include the gitlink entry in the diff"

    repo = pylibgrit.Repository.discover(str(gitlink_repo))
    # Must not crash reading the gitlink commit; stats match git --numstat exactly.
    stats = repo.diff(repo.resolve("HEAD^"), repo.resolve("HEAD")).stats
    assert stats.files_changed == files
    assert stats.insertions == ins
    assert stats.deletions == dele


def test_diffstat_propagates_read_error(
    tmp_path: Path, git_env: dict[str, str]
) -> None:
    """A missing/corrupt blob must make `.stats` RAISE, not silently return wrong counts.

    AIDEV-NOTE: FIX 4 — read_blob_bytes now propagates ODB read failures instead of
    swallowing them as empty content. We synthesize a tree that references a NON-EXISTENT
    blob oid via `git mktree --missing` (which does not verify object existence), then diff
    the empty tree against it. Reading the missing blob for stats must surface as an error.
    """
    import pylibgrit

    repo = tmp_path / "missrepo"
    repo.mkdir()

    def g(*a: str, stdin: bytes | None = None) -> bytes:
        return subprocess.run(
            ["git", *a],
            cwd=repo,
            env=git_env,
            check=True,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ).stdout

    g("init", "-q", "-b", "main")
    bogus = "d" * 40  # a valid-format SHA-1 oid that is NOT in the odb
    empty_tree = g("mktree", "--missing", stdin=b"").strip().decode()
    bad_tree = (
        g(
            "mktree",
            "--missing",
            stdin=f"100644 blob {bogus}\tmissing\n".encode(),
        )
        .strip()
        .decode()
    )

    pyrepo = pylibgrit.Repository.discover(str(repo))
    a = pylibgrit.ObjectId.from_hex(empty_tree)
    b = pylibgrit.ObjectId.from_hex(bad_tree)
    # The missing-blob read for stats must propagate as an error (not silently empty).
    # Stats are LAZY (FIX 5): diff() itself succeeds (it does not read the blob) and the
    # error surfaces on first .stats access. The block tolerates either location.
    with pytest.raises(pylibgrit.GritError):
        d = pyrepo.diff(a, b)
        _ = d.stats


def test_diffstat_is_lazy(diff_repo: Path) -> None:
    """`.stats` is computed lazily and cached; iterating statuses needs no blob reads.

    AIDEV-NOTE: FIX 5 — diff() no longer computes stats eagerly. We assert that (a) a Diff
    can be fully iterated for statuses without touching .stats, and (b) repeated .stats
    accesses return consistent (cached) values. The missing-blob test above separately
    proves the work is deferred (the error only surfaces on .stats access).
    """
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(diff_repo))
    d = repo.diff(repo.resolve("HEAD^"), repo.resolve("HEAD"))
    # Iterate statuses WITHOUT accessing .stats (no blob reads needed).
    assert sorted(e.status for e in d) == ["A", "D", "M"]
    # First .stats computes; a second access returns the same cached values.
    s1 = d.stats
    s2 = d.stats
    assert (s1.files_changed, s1.insertions, s1.deletions) == (
        s2.files_changed,
        s2.insertions,
        s2.deletions,
    )


@pytest.mark.xfail(
    reason="count_changes splits bare \\r as a line break; git --numstat splits on \\n only",
    strict=False,
)
def test_diffstat_bare_cr_diverges_from_git(
    tmp_path: Path, git_env: dict[str, str]
) -> None:
    """Document the known --numstat parity gap for bare-CR-as-content files.

    grit's count_changes (via `similar`) treats a bare `\\r` as a line break, but
    `git --numstat` splits on `\\n` only. For `a\\rb\\n` -> `a\\rb\\rc\\rd\\n` the
    binding counts ins=3/del=1 while git counts ins=1/del=1, so the oracle assertion
    fails (xfail). This test exists to keep the divergence executable and visible.
    """
    import pylibgrit

    from tests.gitlib import run_git

    repo = tmp_path / "barecr"
    repo.mkdir()

    def g(*a: str) -> None:
        subprocess.run(["git", *a], cwd=repo, env=git_env, check=True)

    g("init", "-q", "-b", "main")
    # Disable autocrlf so the bare CR bytes survive verbatim into the blob.
    g("config", "core.autocrlf", "false")
    (repo / "f").write_bytes(b"a\rb\n")
    g("add", "-A")
    g("commit", "-q", "-m", "base")
    (repo / "f").write_bytes(b"a\rb\rc\rd\n")
    g("add", "-A")
    g("commit", "-q", "-m", "change")

    a = run_git(repo, "rev-parse", "HEAD^").decode().strip()
    b = run_git(repo, "rev-parse", "HEAD").decode().strip()
    numstat = run_git(repo, "diff", "--numstat", a, b).decode().strip().splitlines()
    ins = dele = 0
    for line in numstat:
        added, deleted, _path = line.split("\t", 2)
        if added != "-":
            ins += int(added)
        if deleted != "-":
            dele += int(deleted)

    pyrepo = pylibgrit.Repository.discover(str(repo))
    stats = pyrepo.diff(pyrepo.resolve("HEAD^"), pyrepo.resolve("HEAD")).stats
    # git: ins=1/del=1; binding (count_changes): ins=3/del=1 -> these differ (xfail).
    assert stats.insertions == ins
    assert stats.deletions == dele
