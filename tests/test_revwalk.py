"""Revwalk / rev-list: order + laziness, oracled against `git rev-list`.

AIDEV-NOTE: `repo.revwalk(start, *, order=None, first_parent=False)` precomputes the
ordered oids via grit-lib's batch `rev_list` and yields `Commit` objects lazily (see
src/revwalk.rs). `start` is an `ObjectId` (e.g. `repo.resolve("HEAD")`). Orderings map
onto grit-lib's native `OrderingMode` (all backed by grit-lib — none are xfail'd):
    None/"date" -> Default (committer-date, == `git rev-list HEAD`)
    "date-order" -> `git rev-list --date-order`
    "topo"       -> `git rev-list --topo-order`
    "reverse"    -> Default then reversed (== `git rev-list --reverse`)
and `first_parent=True` == `git rev-list --first-parent`.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from tests.gitlib import git_text


@pytest.fixture
def linear_repo(tmp_path: Path, git_env: dict[str, str]) -> Path:
    """A repo with four sequential commits c0..c3 on `main`."""
    repo = tmp_path / "lin"
    repo.mkdir()
    subprocess.run(
        ["git", "init", "-q", "-b", "main"], cwd=repo, env=git_env, check=True
    )
    for i in range(4):
        (repo / "f").write_text(f"{i}\n")
        subprocess.run(["git", "add", "-A"], cwd=repo, env=git_env, check=True)
        subprocess.run(
            ["git", "commit", "-q", "-m", f"c{i}"], cwd=repo, env=git_env, check=True
        )
    return repo


@pytest.fixture
def merge_repo(tmp_path: Path, git_env: dict[str, str]) -> Path:
    """A repo with a `--no-ff` merge so topo- and date-orderings can differ.

    AIDEV-NOTE: We stagger committer dates so `feat1` (the side branch) is committed
    BEFORE `main1` (the first-parent trunk). In default/date order `feat1` therefore sorts
    later than `main1`, but in topo order it must appear grouped relative to the merge.
    This makes the two orderings produce DIFFERENT sequences (verified: topo puts feat1
    right after the merge, date/default put main1 first), which the order tests rely on.
    """
    repo = tmp_path / "mg"
    repo.mkdir()

    def g(*a: str, date: str | None = None) -> None:
        env = dict(git_env)
        if date is not None:
            env["GIT_AUTHOR_DATE"] = date
            env["GIT_COMMITTER_DATE"] = date
        subprocess.run(["git", *a], cwd=repo, env=env, check=True)

    g("init", "-q", "-b", "main")
    (repo / "a").write_text("a\n")
    g("add", "-A")
    g("commit", "-q", "-m", "base", date="2005-04-07T22:13:13")
    g("checkout", "-q", "-b", "feat")
    (repo / "b").write_text("b\n")
    g("add", "-A")
    g("commit", "-q", "-m", "feat1", date="2005-04-07T22:14:00")
    g("checkout", "-q", "main")
    (repo / "c").write_text("c\n")
    g("add", "-A")
    g("commit", "-q", "-m", "main1", date="2005-04-07T22:15:00")
    g("merge", "-q", "--no-ff", "feat", "-m", "merge")
    return repo


def test_revwalk_matches_rev_list(linear_repo: Path) -> None:
    import pylibgrit

    expected = git_text(linear_repo, "rev-list", "HEAD").split("\n")
    repo = pylibgrit.Repository.discover(str(linear_repo))
    head = repo.resolve("HEAD")
    got = [c.id.hex for c in repo.revwalk(head)]
    assert got == expected


def test_revwalk_yields_commits(linear_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(linear_repo))
    head = repo.resolve("HEAD")
    commits = list(repo.revwalk(head))
    # newest commit first (default reverse-chronological order). grit-lib keeps the body's
    # own trailing LF, so a `-m "c3"` message surfaces as b"c3\n".
    assert commits[0].message_bytes == b"c3\n"
    assert commits[-1].message_bytes == b"c0\n"


def test_revwalk_default_matches_rev_list_on_merge(merge_repo: Path) -> None:
    """Default order must match `git rev-list HEAD` exactly, even across a merge."""
    import pylibgrit

    expected = git_text(merge_repo, "rev-list", "HEAD").split("\n")
    repo = pylibgrit.Repository.discover(str(merge_repo))
    head = repo.resolve("HEAD")
    got = [c.id.hex for c in repo.revwalk(head)]
    assert got == expected


def test_revwalk_reverse(linear_repo: Path) -> None:
    """order='reverse' matches `git rev-list --reverse HEAD`."""
    import pylibgrit

    expected = git_text(linear_repo, "rev-list", "--reverse", "HEAD").split("\n")
    repo = pylibgrit.Repository.discover(str(linear_repo))
    head = repo.resolve("HEAD")
    got = [c.id.hex for c in repo.revwalk(head, order="reverse")]
    assert got == expected
    # sanity: reverse is the exact inverse of default
    default = [c.id.hex for c in repo.revwalk(head)]
    assert got == list(reversed(default))


def test_revwalk_topo_order(merge_repo: Path) -> None:
    """order='topo' matches `git rev-list --topo-order HEAD` (differs from default here)."""
    import pylibgrit

    expected = git_text(merge_repo, "rev-list", "--topo-order", "HEAD").split("\n")
    default = git_text(merge_repo, "rev-list", "HEAD").split("\n")
    # The fixture is constructed so topo and default genuinely differ.
    assert expected != default
    repo = pylibgrit.Repository.discover(str(merge_repo))
    head = repo.resolve("HEAD")
    got = [c.id.hex for c in repo.revwalk(head, order="topo")]
    assert got == expected


def test_revwalk_date_order(merge_repo: Path) -> None:
    """order='date-order' matches `git rev-list --date-order HEAD`."""
    import pylibgrit

    expected = git_text(merge_repo, "rev-list", "--date-order", "HEAD").split("\n")
    repo = pylibgrit.Repository.discover(str(merge_repo))
    head = repo.resolve("HEAD")
    got = [c.id.hex for c in repo.revwalk(head, order="date-order")]
    assert got == expected


def test_revwalk_first_parent(merge_repo: Path) -> None:
    """first_parent=True matches `git rev-list --first-parent HEAD`."""
    import pylibgrit

    expected = git_text(merge_repo, "rev-list", "--first-parent", "HEAD").split("\n")
    repo = pylibgrit.Repository.discover(str(merge_repo))
    head = repo.resolve("HEAD")
    got = [c.id.hex for c in repo.revwalk(head, first_parent=True)]
    assert got == expected
    # first-parent must drop the side branch's feat1 commit
    feat1 = git_text(merge_repo, "rev-parse", "feat")
    assert feat1 not in got


def test_revwalk_unknown_order_raises(linear_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(linear_repo))
    head = repo.resolve("HEAD")
    with pytest.raises(ValueError, match="unknown order"):
        list(repo.revwalk(head, order="bogus"))
