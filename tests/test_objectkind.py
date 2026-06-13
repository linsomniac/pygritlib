def test_objectkind_members():
    import pygrit

    assert {k.name for k in pygrit.ObjectKind} >= {"COMMIT", "TREE", "BLOB", "TAG"}


def test_objectkind_distinct():
    import pygrit

    assert pygrit.ObjectKind.COMMIT != pygrit.ObjectKind.TREE


def test_objectkind_values_are_stable():
    import pygrit

    assert (
        pygrit.ObjectKind.COMMIT,
        pygrit.ObjectKind.TREE,
        pygrit.ObjectKind.BLOB,
        pygrit.ObjectKind.TAG,
    ) == (0, 1, 2, 3)
