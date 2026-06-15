"""FFI lifetime safety: children (Odb, TreeIter) must outlive their parents.

AIDEV-NOTE: These tests pin the ownership model from design §6. `repo.odb` clones an
Arc<Repository>, and `iter(tree)` clones an Arc<[TreeEntryData]>, so each child owns its
own reference to the underlying data. After `del parent; gc.collect()` the child must
remain usable with no crash/abort. If either test segfaults, the binding holds a borrow
instead of an Arc — FIX the binding, do NOT weaken the test.
"""

from __future__ import annotations

import gc
from pathlib import Path

from tests.gitlib import rev_parse


def test_odb_outlives_repository(simple_repo: Path) -> None:
    import pylibgrit

    oid = rev_parse(simple_repo, "HEAD:a.txt")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    odb = repo.odb
    del repo
    gc.collect()
    obj = odb.read(
        pylibgrit.ObjectId.from_hex(oid)
    )  # Arc keeps repo alive; must not crash
    assert obj.id.hex == oid


def test_tree_iter_outlives_tree(simple_repo: Path) -> None:
    import pylibgrit

    tree_oid = rev_parse(simple_repo, "HEAD^{tree}")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    tree = repo.tree(pylibgrit.ObjectId.from_hex(tree_oid))
    it = iter(tree)
    del tree
    gc.collect()
    names = {e.name for e in it}
    assert b"a.txt" in names


def test_reference_iter_outlives_repository(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    it = repo.references()
    del repo
    gc.collect()
    # ReferenceIter owns Arc<Repository> + Arc<[ReferenceData]>; must not crash.
    names = {r.name for r in it}
    assert b"refs/heads/main" in names


def test_head_reference_peel_outlives_repository(simple_repo: Path) -> None:
    import pylibgrit

    head_oid = rev_parse(simple_repo, "HEAD")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    head = repo.head()
    del repo
    gc.collect()
    # peel() of a symbolic HEAD dereferences the Reference's own Arc<Repository>.
    assert head.peel().hex == head_oid


def test_revwalk_outlives_repository(simple_repo: Path) -> None:
    import pylibgrit

    head_oid = rev_parse(simple_repo, "HEAD")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    head = repo.resolve("HEAD")
    walk = repo.revwalk(head)
    del repo
    gc.collect()
    # RevWalk owns Arc<Repository> + Arc<[ObjectId]>; per-step odb reads must still work
    # after the parent Repository is dropped.
    oids = [c.id.hex for c in walk]
    assert oids == [head_oid]
