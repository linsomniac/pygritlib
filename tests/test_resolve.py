"""resolve(spec) tests, oracled against `git rev-parse`."""

from __future__ import annotations

from pathlib import Path

import pytest

from tests.gitlib import rev_parse


def test_resolve_head(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    assert repo.resolve("HEAD").hex == rev_parse(simple_repo, "HEAD")


def test_resolve_full_hex_roundtrip(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    h = rev_parse(simple_repo, "HEAD")
    assert repo.resolve(h).hex == h


def test_resolve_peel_to_tree(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    assert repo.resolve("HEAD^{tree}").hex == rev_parse(simple_repo, "HEAD^{tree}")


def test_resolve_path_lookup(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    assert repo.resolve("HEAD:a.txt").hex == rev_parse(simple_repo, "HEAD:a.txt")


def test_resolve_unknown_raises(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    # AIDEV-NOTE: Empirically, grit-lib 0.4.1 `resolve_revision` does NOT return
    # Error::ObjectNotFound for a bare unknown ref. `resolve_base` wraps it in
    # Error::Message with git's "fatal: ambiguous argument ... unknown revision or path"
    # diagnostic (rev_parse.rs ~line 2492), which our error map routes to the base
    # GritError catch-all (src/error.rs). So we assert GritError here. (ObjectNotFoundError
    # is reserved for odb reads of a missing oid, e.g. Repository.commit/tree/blob.)
    with pytest.raises(pylibgrit.GritError):
        repo.resolve("definitely-no-such-ref-xyz")
