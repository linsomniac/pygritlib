import os

import pytest


def test_discover_returns_repository(simple_repo):
    import pygrit

    repo = pygrit.Repository.discover(str(simple_repo))
    assert repo.git_dir == os.fsencode(simple_repo / ".git")
    assert repo.work_tree == os.fsencode(simple_repo)
    assert repo.is_bare is False


def test_discover_accepts_pathlike(simple_repo):
    import pygrit

    # simple_repo is a pathlib.Path (an os.PathLike).
    repo = pygrit.Repository.discover(simple_repo)
    assert repo.git_dir == os.fsencode(simple_repo / ".git")


def test_discover_accepts_bytes_path(simple_repo):
    import pygrit

    # design §5: path inputs accept str | bytes | os.PathLike. On Unix, bytes map
    # to the OS path 1:1 (exact, surrogate-free byte fidelity).
    repo = pygrit.Repository.discover(os.fsencode(str(simple_repo)))
    assert repo.git_dir == os.fsencode(simple_repo / ".git")
    assert repo.work_tree == os.fsencode(simple_repo)


def test_discover_rejects_invalid_path_type():
    import pygrit

    with pytest.raises(TypeError):
        pygrit.Repository.discover(1234)


def test_discover_missing_repo_raises(tmp_path):
    import pygrit

    with pytest.raises(pygrit.RepositoryError):
        pygrit.Repository.discover(str(tmp_path))


def test_open_explicit_dirs(simple_repo):
    import pygrit

    repo = pygrit.Repository.open(str(simple_repo / ".git"), str(simple_repo))
    assert repo.is_bare is False


def test_open_accepts_bytes_paths(simple_repo):
    import pygrit

    repo = pygrit.Repository.open(
        os.fsencode(str(simple_repo / ".git")), os.fsencode(str(simple_repo))
    )
    assert repo.git_dir == os.fsencode(simple_repo / ".git")
    assert repo.work_tree == os.fsencode(simple_repo)
    assert repo.is_bare is False
