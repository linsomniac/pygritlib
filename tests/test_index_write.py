import subprocess


def _init(repo, env):
    subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], env=env, check=True)


def _ls_files_stage(repo, env):
    return subprocess.run(
        ["git", "ls-files", "--stage"], cwd=repo, env=env,
        stdout=subprocess.PIPE, check=True,
    ).stdout.decode()


def test_index_add_and_write_persists(tmp_path, git_env):
    import pylibgrit

    repo = tmp_path / "r"
    repo.mkdir()
    _init(repo, git_env)
    pg = pylibgrit.Repository.open(str(repo / ".git"))
    blob = pg.odb.write(pylibgrit.ObjectKind.BLOB, b"hello\n")

    idx = pg.index()
    idx.add(b"a.txt", blob, 0o100644)
    idx.write()

    staged = _ls_files_stage(repo, git_env)
    assert blob.hex in staged
    assert "a.txt" in staged
    assert staged.startswith("100644 ")


def test_index_remove(tmp_path, git_env):
    import pylibgrit

    repo = tmp_path / "r"
    repo.mkdir()
    _init(repo, git_env)
    pg = pylibgrit.Repository.open(str(repo / ".git"))
    blob = pg.odb.write(pylibgrit.ObjectKind.BLOB, b"x\n")
    idx = pg.index()
    idx.add(b"a.txt", blob, 0o100644)
    assert idx.remove(b"a.txt") is True
    assert idx.remove(b"a.txt") is False
    idx.write()
    assert _ls_files_stage(repo, git_env).strip() == ""


def test_index_add_entry_raw(tmp_path, git_env):
    import pylibgrit

    repo = tmp_path / "r"
    repo.mkdir()
    _init(repo, git_env)
    pg = pylibgrit.Repository.open(str(repo / ".git"))
    blob = pg.odb.write(pylibgrit.ObjectKind.BLOB, b"y\n")
    idx = pg.index()
    idx.add_entry(pylibgrit.IndexEntry(b"b.txt", blob, 0o100644))
    idx.write()
    assert "b.txt" in _ls_files_stage(repo, git_env)


def test_write_tree_matches_git(tmp_path, git_env):
    import pylibgrit

    repo = tmp_path / "r"
    repo.mkdir()
    _init(repo, git_env)
    pg = pylibgrit.Repository.open(str(repo / ".git"))
    blob = pg.odb.write(pylibgrit.ObjectKind.BLOB, b"hello\n")

    idx = pg.index()
    idx.add(b"a.txt", blob, 0o100644)
    idx.write()
    tree = idx.write_tree()

    git_tree = subprocess.run(
        ["git", "write-tree"], cwd=repo, env=git_env,
        stdout=subprocess.PIPE, check=True,
    ).stdout.decode().strip()
    assert tree.hex == git_tree
