def test_exception_hierarchy():
    import pylibgrit

    assert issubclass(pylibgrit.RepositoryError, pylibgrit.GritError)
    assert issubclass(pylibgrit.ObjectNotFoundError, pylibgrit.GritError)
    assert issubclass(pylibgrit.InvalidObjectError, pylibgrit.GritError)
    assert pylibgrit.GritError is not pylibgrit.RepositoryError
    assert not issubclass(pylibgrit.ObjectNotFoundError, pylibgrit.RepositoryError)
