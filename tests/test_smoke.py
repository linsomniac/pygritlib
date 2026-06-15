def test_native_module_imports():
    import pylibgrit

    assert hasattr(pylibgrit, "Repository")
    assert hasattr(pylibgrit, "ObjectId")
    assert "Repository" in pylibgrit.__all__
