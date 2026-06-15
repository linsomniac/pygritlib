def test_signature_wire_format():
    import pylibgrit

    sig = pylibgrit.Signature(b"Ada Lovelace", b"ada@example.com", (1718000000, 0))
    assert sig.name == b"Ada Lovelace"
    assert sig.email == b"ada@example.com"
    assert sig.when == (1718000000, 0)
    assert sig.raw == b"Ada Lovelace <ada@example.com> 1718000000 +0000"


def test_signature_positive_and_negative_tz():
    import pylibgrit

    east = pylibgrit.Signature(b"E", b"e@x", (1, 19800))   # +05:30
    west = pylibgrit.Signature(b"W", b"w@x", (1, -28800))   # -08:00
    assert east.raw == b"E <e@x> 1 +0530"
    assert west.raw == b"W <w@x> 1 -0800"
