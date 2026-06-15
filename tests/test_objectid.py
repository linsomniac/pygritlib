import pytest

HEX = "0123456789abcdef0123456789abcdef01234567"  # 40 hex = SHA-1


def test_objectid_from_hex_roundtrip():
    import pylibgrit

    oid = pylibgrit.ObjectId.from_hex(HEX)
    assert oid.hex == HEX
    assert oid.raw == bytes.fromhex(HEX)
    assert oid.hash_algorithm == "sha1"


def test_objectid_equality_and_hash():
    import pylibgrit

    a = pylibgrit.ObjectId.from_hex(HEX)
    b = pylibgrit.ObjectId.from_hex(HEX)
    assert a == b
    assert hash(a) == hash(b)
    assert {a, b} == {a}


def test_objectid_repr():
    import pylibgrit

    assert HEX in repr(pylibgrit.ObjectId.from_hex(HEX))


def test_objectid_invalid_hex_raises():
    import pylibgrit

    with pytest.raises((ValueError, pylibgrit.InvalidObjectError)):
        pylibgrit.ObjectId.from_hex("xyz")
