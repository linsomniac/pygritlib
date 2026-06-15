"""References + HEAD tests, oracled against real git."""

from __future__ import annotations

import subprocess
from pathlib import Path

from tests.gitlib import git_text, rev_parse, run_git


def test_references_match_git(simple_repo: Path) -> None:
    import pylibgrit

    # AIDEV-NOTE: git 2.53's `for-each-ref` does not accept `-z`, so we split on newline.
    # Ref names and the `%(objectname) %(refname)` format never contain newlines, so this
    # is unambiguous here.
    raw = run_git(simple_repo, "for-each-ref", "--format=%(objectname) %(refname)")
    expected = {}
    for rec in raw.split(b"\n"):
        if not rec:
            continue
        oid, name = rec.split(b" ", 1)
        expected[name] = oid.decode()
    repo = pylibgrit.Repository.discover(str(simple_repo))
    got = {r.name: r.target.hex for r in repo.references() if r.target is not None}
    assert got == expected


def test_head_symbolic(simple_repo: Path) -> None:
    import pylibgrit

    branch = git_text(simple_repo, "symbolic-ref", "HEAD")  # e.g. refs/heads/main
    repo = pylibgrit.Repository.discover(str(simple_repo))
    head = repo.head()
    assert head.is_symbolic is True
    assert head.symbolic_target == branch.encode()
    assert head.target is None
    assert head.peel().hex == git_text(simple_repo, "rev-parse", "HEAD")


def test_head_detached(simple_repo: Path) -> None:
    import pylibgrit

    head_oid = rev_parse(simple_repo, "HEAD")
    # detach
    subprocess.run(
        ["git", "-C", str(simple_repo), "checkout", "-q", "--detach", head_oid],
        check=True,
    )
    repo = pylibgrit.Repository.discover(str(simple_repo))
    head = repo.head()
    assert head.is_symbolic is False
    assert head.peel().hex == head_oid
