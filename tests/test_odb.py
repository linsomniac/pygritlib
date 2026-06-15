import pytest

from tests.gitlib import cat_file_data, rev_parse


def test_odb_read_blob_matches_git(simple_repo):
    import pylibgrit

    blob_oid = rev_parse(simple_repo, "HEAD:a.txt")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    obj = repo.odb.read(pylibgrit.ObjectId.from_hex(blob_oid))
    assert obj.id.hex == blob_oid
    assert obj.kind is pylibgrit.ObjectKind.BLOB
    assert obj.data == cat_file_data(simple_repo, blob_oid)


def test_odb_read_commit_matches_git(simple_repo):
    import pylibgrit

    commit_oid = rev_parse(simple_repo, "HEAD")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    obj = repo.odb.read(pylibgrit.ObjectId.from_hex(commit_oid))
    assert obj.kind is pylibgrit.ObjectKind.COMMIT
    assert obj.data == cat_file_data(simple_repo, commit_oid)


def test_odb_exists(simple_repo):
    import pylibgrit

    commit_oid = rev_parse(simple_repo, "HEAD")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    assert repo.odb.exists(pylibgrit.ObjectId.from_hex(commit_oid)) is True


def test_odb_read_missing_raises(simple_repo):
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    missing = pylibgrit.ObjectId.from_hex("0" * 40)
    with pytest.raises(pylibgrit.ObjectNotFoundError):
        repo.odb.read(missing)
