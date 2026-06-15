import os

import pytest


def test_discover_returns_repository(simple_repo):
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    assert repo.git_dir == os.fsencode(simple_repo / ".git")
    assert repo.work_tree == os.fsencode(simple_repo)
    assert repo.is_bare is False


def test_discover_accepts_pathlike(simple_repo):
    import pylibgrit

    # simple_repo is a pathlib.Path (an os.PathLike).
    repo = pylibgrit.Repository.discover(simple_repo)
    assert repo.git_dir == os.fsencode(simple_repo / ".git")


def test_discover_accepts_bytes_path(simple_repo):
    import pylibgrit

    # design §5: path inputs accept str | bytes | os.PathLike. On Unix, bytes map
    # to the OS path 1:1 (exact, surrogate-free byte fidelity).
    repo = pylibgrit.Repository.discover(os.fsencode(str(simple_repo)))
    assert repo.git_dir == os.fsencode(simple_repo / ".git")
    assert repo.work_tree == os.fsencode(simple_repo)


def test_discover_accepts_pathlike_returning_bytes(simple_repo):
    import pylibgrit

    # AIDEV-NOTE: design §5 — path inputs accept os.PathLike whose __fspath__ returns
    # bytes (not just str). PyO3's PathBuf extractor rejects bytes from __fspath__, so
    # extract_path falls back to os.fspath() and handles a bytes result via OsString.
    class _BytesPath:
        def __fspath__(self):
            return os.fsencode(str(simple_repo))

    repo = pylibgrit.Repository.discover(_BytesPath())
    assert repo.git_dir == os.fsencode(simple_repo / ".git")


def test_discover_rejects_invalid_path_type():
    import pylibgrit

    with pytest.raises(TypeError):
        pylibgrit.Repository.discover(1234)


def test_discover_missing_repo_raises(tmp_path):
    import pylibgrit

    with pytest.raises(pylibgrit.RepositoryError):
        pylibgrit.Repository.discover(str(tmp_path))


def test_open_explicit_dirs(simple_repo):
    import pylibgrit

    repo = pylibgrit.Repository.open(str(simple_repo / ".git"), str(simple_repo))
    assert repo.is_bare is False


def test_open_accepts_bytes_paths(simple_repo):
    import pylibgrit

    repo = pylibgrit.Repository.open(
        os.fsencode(str(simple_repo / ".git")), os.fsencode(str(simple_repo))
    )
    assert repo.git_dir == os.fsencode(simple_repo / ".git")
    assert repo.work_tree == os.fsencode(simple_repo)
    assert repo.is_bare is False
