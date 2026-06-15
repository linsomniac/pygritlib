"""Tests for typed object views: Commit + Signature (byte/text policy)."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

from tests.gitlib import cat_file_data, git_text, rev_parse


def test_commit_fields_match_git(simple_repo: Path) -> None:
    import pylibgrit

    oid = rev_parse(simple_repo, "HEAD")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    commit = repo.commit(pylibgrit.ObjectId.from_hex(oid))

    # the commit's own id (self-describing; needed by revwalk)
    assert commit.id.hex == oid

    # tree / parents oracled against git
    assert commit.tree.hex == rev_parse(simple_repo, "HEAD^{tree}")
    assert commit.parents == []

    # author / committer name+email as BYTES (design §5)
    assert (
        commit.author.name
        == git_text(simple_repo, "log", "-1", "--format=%an").encode()
    )
    assert (
        commit.author.email
        == git_text(simple_repo, "log", "-1", "--format=%ae").encode()
    )
    assert (
        commit.committer.name
        == git_text(simple_repo, "log", "-1", "--format=%cn").encode()
    )
    assert (
        commit.committer.email
        == git_text(simple_repo, "log", "-1", "--format=%ce").encode()
    )

    # decoded str accessors
    assert commit.author.name_str == git_text(simple_repo, "log", "-1", "--format=%an")
    assert commit.author.email_str == git_text(simple_repo, "log", "-1", "--format=%ae")

    # author/committer time (fixture is UTC, so offset is 0)
    assert commit.author.when[0] == int(
        git_text(simple_repo, "log", "-1", "--format=%at")
    )
    assert commit.author.when[1] == 0
    assert commit.committer.when[0] == int(
        git_text(simple_repo, "log", "-1", "--format=%ct")
    )
    assert commit.committer.when[1] == 0


def test_commit_message_contract(simple_repo: Path) -> None:
    import pylibgrit

    oid = rev_parse(simple_repo, "HEAD")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    commit = repo.commit(pylibgrit.ObjectId.from_hex(oid))

    # AIDEV-NOTE: message contract — grit-lib's CommitData.message is the EXACT body
    # after the header blank line, including its trailing newline(s). The raw object
    # payload (git cat-file) preserves this; `git log --format=%B` strips exactly one
    # trailing newline. So message_bytes == <payload-body> and equals `%B` + b"\n".
    body = _payload_message_body(simple_repo, oid)
    assert commit.message_bytes == body

    pretty = git_text(simple_repo, "log", "-1", "--format=%B")
    # %B (decoded, .strip()ped by git_text) plus the single newline grit-lib keeps.
    assert commit.message_bytes == pretty.encode() + b"\n"

    # message() decodes message_bytes with the given codec.
    assert commit.message() == commit.message_bytes.decode("utf-8")
    assert commit.message(encoding="utf-8") == "initial commit\n"


def _payload_message_body(repo: Path, oid: str) -> bytes:
    """The commit message section of the raw object payload (after the blank line)."""
    payload = cat_file_data(repo, oid)
    _headers, _, body = payload.partition(b"\n\n")
    return body


def test_tz_offset_parsing_non_utc(tmp_path: Path, git_env: dict[str, str]) -> None:
    """Build a repo with an explicit +0530 author/committer date to exercise tz parsing."""
    import pylibgrit

    repo = tmp_path / "tzrepo"
    repo.mkdir()
    env = dict(git_env)
    env["GIT_AUTHOR_DATE"] = "2005-04-07T22:13:13 +0530"
    env["GIT_COMMITTER_DATE"] = "2005-04-07T22:13:13 +0530"

    def g(*args: str) -> None:
        subprocess.run(
            ["git", *args],
            cwd=repo,
            env=env,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

    g("init", "-q", "-b", "main")
    (repo / "f.txt").write_text("hi\n")
    g("add", "-A")
    g("commit", "-q", "-m", "tz commit")

    oid = rev_parse(repo, "HEAD")
    pyrepo = pylibgrit.Repository.discover(str(repo))
    commit = pyrepo.commit(pylibgrit.ObjectId.from_hex(oid))

    # +0530 == 5*3600 + 30*60 == 19800 seconds
    assert commit.author.when[1] == 19800
    assert commit.committer.when[1] == 19800
    # unix seconds still oracled against git
    assert commit.author.when[0] == int(git_text(repo, "log", "-1", "--format=%at"))


def test_commit_multiparent_parents(tmp_path: Path, git_env: dict[str, str]) -> None:
    """A merge commit has two parents; oracle against git."""
    import pylibgrit

    repo = tmp_path / "mergerepo"
    repo.mkdir()
    env = dict(git_env)

    def g(*args: str) -> bytes:
        return subprocess.run(
            ["git", *args],
            cwd=repo,
            env=env,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ).stdout

    g("init", "-q", "-b", "main")
    (repo / "base.txt").write_text("base\n")
    g("add", "-A")
    g("commit", "-q", "-m", "base")
    g("checkout", "-q", "-b", "feature")
    (repo / "feature.txt").write_text("feature\n")
    g("add", "-A")
    g("commit", "-q", "-m", "feature work")
    g("checkout", "-q", "main")
    (repo / "main.txt").write_text("main\n")
    g("add", "-A")
    g("commit", "-q", "-m", "main work")
    g("merge", "-q", "--no-ff", "-m", "merge feature", "feature")

    merge_oid = rev_parse(repo, "HEAD")
    pyrepo = pylibgrit.Repository.discover(str(repo))
    commit = pyrepo.commit(pylibgrit.ObjectId.from_hex(merge_oid))

    expected = git_text(repo, "log", "-1", "--format=%P").split()
    assert [p.hex for p in commit.parents] == expected
    assert len(commit.parents) == 2


def test_commit_non_utf8_author_name(tmp_path: Path, git_env: dict[str, str]) -> None:
    """A Latin-1 author name round-trips as raw bytes; name_str raises under strict utf-8.

    AIDEV-NOTE: git re-encodes env-supplied identities from the locale into UTF-8, so we
    cannot smuggle a raw 0xE9 byte into a commit via `git commit`. Instead we write the
    commit object DIRECTLY with `git hash-object -w -t commit --stdin`, which stores the
    payload verbatim — exercising the binding's raw-byte name/email split (design §5).
    """
    import pytest

    import pylibgrit

    repo = tmp_path / "latin1repo"
    repo.mkdir()
    env = dict(git_env)
    # 0xE9 is 'e-acute' in Latin-1, invalid as standalone UTF-8.
    latin1_name = b"Jos\xe9"

    def g(*args: str, stdin: bytes | None = None) -> bytes:
        return subprocess.run(
            ["git", *args],
            cwd=repo,
            env=env,
            check=True,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ).stdout

    g("init", "-q", "-b", "main")
    tree = g("mktree", stdin=b"").strip().decode()
    payload = (
        f"tree {tree}\n".encode()
        + b"author "
        + latin1_name
        + b" <author@example.com> 1112911993 +0000\n"
        + b"committer "
        + latin1_name
        + b" <committer@example.com> 1112911993 +0000\n"
        + b"\nlatin1 author\n"
    )
    oid = (
        g("hash-object", "-w", "-t", "commit", "--stdin", stdin=payload)
        .strip()
        .decode()
    )

    # Confirm the raw object actually contains the Latin-1 byte.
    assert latin1_name in cat_file_data(repo, oid)

    pyrepo = pylibgrit.Repository.discover(str(repo))
    commit = pyrepo.commit(pylibgrit.ObjectId.from_hex(oid))

    # .name returns the exact bytes (non-UTF-8 fidelity).
    assert commit.author.name == latin1_name
    assert commit.author.email == b"author@example.com"
    # name_str under strict utf-8 raises (policy: caller chooses error handling).
    with pytest.raises(UnicodeDecodeError):
        _ = commit.author.name_str
    # caller-overridable decode via message() codec on the commit; name decodes under latin-1.
    assert commit.author.name.decode("latin-1") == "José"


def test_commit_message_error_handlers(tmp_path: Path, git_env: dict[str, str]) -> None:
    """message(errors=...) must support non-strict handlers (surrogateescape/replace).

    AIDEV-NOTE: A non-UTF-8 message decoded with errors="surrogateescape" yields a str
    containing lone surrogates, which a Rust `String` cannot hold. The decode path must
    therefore return the Python str object directly rather than round-tripping through a
    Rust String. We build a commit with a raw 0xE9 byte in the message body by writing the
    commit object directly (git re-encodes -m messages from the locale, so we cannot smuggle
    a raw byte via `git commit`).
    """
    import pylibgrit

    repo = tmp_path / "msgrepo"
    repo.mkdir()
    env = dict(git_env)

    def g(*args: str, stdin: bytes | None = None) -> bytes:
        return subprocess.run(
            ["git", *args],
            cwd=repo,
            env=env,
            check=True,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ).stdout

    g("init", "-q", "-b", "main")
    g("config", "core.autocrlf", "false")
    tree = g("mktree", stdin=b"").strip().decode()
    # 0xE9 is 'e-acute' in Latin-1, invalid as standalone UTF-8, placed in the body.
    body = b"caf\xe9 message\n"
    payload = (
        f"tree {tree}\n".encode()
        + b"author Test Author <author@example.com> 1112911993 +0000\n"
        + b"committer Test Committer <committer@example.com> 1112911993 +0000\n"
        + b"\n"
        + body
    )
    oid = (
        g("hash-object", "-w", "-t", "commit", "--stdin", stdin=payload)
        .strip()
        .decode()
    )

    pyrepo = pylibgrit.Repository.discover(str(repo))
    commit = pyrepo.commit(pylibgrit.ObjectId.from_hex(oid))

    # The raw 0xe9 byte survives in message_bytes.
    assert b"\xe9" in commit.message_bytes
    assert commit.message_bytes == body

    # Default strict decode raises on the invalid UTF-8 byte.
    with pytest.raises(UnicodeDecodeError):
        _ = commit.message()

    # surrogateescape returns a str round-tripping back to the original bytes.
    escaped = commit.message(errors="surrogateescape")
    assert isinstance(escaped, str)
    assert escaped.encode("utf-8", "surrogateescape") == body

    # replace yields a str containing the U+FFFD replacement char.
    replaced = commit.message(errors="replace")
    assert isinstance(replaced, str)
    assert "�" in replaced


def test_tree_entries_match_git(simple_repo: Path) -> None:
    import pylibgrit

    from tests.gitlib import rev_parse, run_git

    tree_oid = rev_parse(simple_repo, "HEAD^{tree}")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    tree = repo.tree(pylibgrit.ObjectId.from_hex(tree_oid))
    raw = run_git(
        simple_repo, "ls-tree", "-z", tree_oid
    )  # "<mode> <type> <oid>\t<name>\0"
    expected_names = {rec.split(b"\t", 1)[1] for rec in raw.split(b"\0") if rec}
    assert {e.name for e in tree} == expected_names
    a = next(e for e in tree if e.name == b"a.txt")
    assert a.mode == 0o100644
    assert a.kind is pylibgrit.ObjectKind.BLOB
    assert a.id.hex == rev_parse(simple_repo, "HEAD:a.txt")
    d = next(e for e in tree if e.name == b"dir")
    assert d.kind is pylibgrit.ObjectKind.TREE


def test_blob_data_matches_git(simple_repo: Path) -> None:
    import pylibgrit

    from tests.gitlib import cat_file_data, rev_parse

    blob_oid = rev_parse(simple_repo, "HEAD:a.txt")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    blob = repo.blob(pylibgrit.ObjectId.from_hex(blob_oid))
    assert blob.data == cat_file_data(simple_repo, blob_oid)


def test_blob_on_non_blob_raises(simple_repo: Path) -> None:
    import pylibgrit

    from tests.gitlib import rev_parse

    repo = pylibgrit.Repository.discover(str(simple_repo))
    tree_oid = rev_parse(simple_repo, "HEAD^{tree}")
    with pytest.raises(pylibgrit.InvalidObjectError):
        repo.blob(pylibgrit.ObjectId.from_hex(tree_oid))


def test_typed_accessors_verify_object_kind(simple_repo: Path) -> None:
    """commit()/tree()/tag() must reject an oid whose object kind does not match.

    AIDEV-NOTE: A blob whose bytes happen to parse as a valid commit/tree/tag payload
    must not be silently accepted by the typed accessor — the accessor verifies the
    ODB object's kind BEFORE parsing (mirroring blob()'s existing kind check).
    """
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    blob_oid = pylibgrit.ObjectId.from_hex(rev_parse(simple_repo, "HEAD:a.txt"))
    tree_oid = pylibgrit.ObjectId.from_hex(rev_parse(simple_repo, "HEAD^{tree}"))
    commit_oid = pylibgrit.ObjectId.from_hex(rev_parse(simple_repo, "HEAD"))

    # Mismatched kinds raise InvalidObjectError (kind verified before parsing).
    with pytest.raises(pylibgrit.InvalidObjectError):
        repo.commit(tree_oid)
    with pytest.raises(pylibgrit.InvalidObjectError):
        repo.tree(commit_oid)
    with pytest.raises(pylibgrit.InvalidObjectError):
        repo.tag(commit_oid)
    with pytest.raises(pylibgrit.InvalidObjectError):
        repo.commit(blob_oid)

    # Positive cases still work.
    assert repo.commit(commit_oid).id == commit_oid
    assert len(repo.tree(tree_oid)) >= 1


def test_typed_accessors_reject_blob_with_parseable_payload(
    tmp_path: Path, git_env: dict[str, str]
) -> None:
    """A blob whose bytes ARE a valid commit payload must not pass commit().

    AIDEV-NOTE: This is the core bug FIX 1 addresses. Without a kind check before
    parsing, a blob storing a valid commit-object payload would be wrongly accepted
    by commit(). We store such a payload as a BLOB and assert commit() rejects it on
    kind, raising InvalidObjectError.
    """
    import pylibgrit

    repo_dir = tmp_path / "kindrepo"
    repo_dir.mkdir()
    env = dict(git_env)

    def g(*args: str, stdin: bytes | None = None) -> bytes:
        return subprocess.run(
            ["git", *args],
            cwd=repo_dir,
            env=env,
            check=True,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        ).stdout

    g("init", "-q", "-b", "main")
    tree = g("mktree", stdin=b"").strip().decode()
    # A perfectly valid commit payload...
    commit_payload = (
        f"tree {tree}\n".encode()
        + b"author Test Author <author@example.com> 1112911993 +0000\n"
        + b"committer Test Committer <committer@example.com> 1112911993 +0000\n"
        + b"\nhi\n"
    )
    # ...stored as a BLOB object (kind=blob, content=commit payload).
    blob_oid = (
        g("hash-object", "-w", "-t", "blob", "--stdin", stdin=commit_payload)
        .strip()
        .decode()
    )

    pyrepo = pylibgrit.Repository.discover(str(repo_dir))
    # commit() must reject the blob on KIND, even though its bytes parse as a commit.
    with pytest.raises(pylibgrit.InvalidObjectError):
        pyrepo.commit(pylibgrit.ObjectId.from_hex(blob_oid))


def test_tag_fields_match_git(tmp_path: Path, git_env: dict[str, str]) -> None:
    import pylibgrit

    from tests.gitlib import rev_parse

    repo = tmp_path / "tagrepo"
    repo.mkdir()
    subprocess.run(
        ["git", "init", "-q", "-b", "main"], cwd=repo, env=git_env, check=True
    )
    (repo / "f").write_text("x\n")
    subprocess.run(["git", "add", "-A"], cwd=repo, env=git_env, check=True)
    subprocess.run(
        ["git", "commit", "-q", "-m", "c"], cwd=repo, env=git_env, check=True
    )
    subprocess.run(
        ["git", "tag", "-a", "v1", "-m", "release one"],
        cwd=repo,
        env=git_env,
        check=True,
    )
    tag_oid = rev_parse(repo, "v1")  # annotated tag object
    pyrepo = pylibgrit.Repository.discover(str(repo))
    tag = pyrepo.tag(pylibgrit.ObjectId.from_hex(tag_oid))
    assert tag.name == b"v1"
    assert tag.target.hex == rev_parse(repo, "v1^{commit}")
    assert tag.message_bytes == b"release one\n"  # grit keeps the body's trailing LF
    assert tag.tagger is not None
    assert tag.tagger.name == b"Test Committer"
