def test_objectkind_members():
    import pylibgrit

    assert {k.name for k in pylibgrit.ObjectKind} >= {"COMMIT", "TREE", "BLOB", "TAG"}


def test_objectkind_distinct():
    import pylibgrit

    assert pylibgrit.ObjectKind.COMMIT != pylibgrit.ObjectKind.TREE


def test_objectkind_values_are_stable():
    import pylibgrit

    assert (
        pylibgrit.ObjectKind.COMMIT,
        pylibgrit.ObjectKind.TREE,
        pylibgrit.ObjectKind.BLOB,
        pylibgrit.ObjectKind.TAG,
    ) == (0, 1, 2, 3)
