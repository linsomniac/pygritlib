"""Tests for the release-inventory checker.

Release tooling — intentionally outside ``tests/`` so the binding suite (run by
CI's ``pytest tests/``) does not collect it.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from check_release_inventory import check_inventory  # noqa: E402


def _touch(directory: Path, name: str) -> None:
    (directory / name).write_bytes(b"")


def _good_dist(directory: Path, version: str = "0.1.0") -> None:
    _touch(directory, f"pylibgrit-{version}.tar.gz")
    _touch(
        directory,
        f"pylibgrit-{version}-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
    )
    _touch(
        directory,
        f"pylibgrit-{version}-cp311-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl",
    )
    _touch(directory, f"pylibgrit-{version}-cp311-abi3-musllinux_1_2_x86_64.whl")
    _touch(directory, f"pylibgrit-{version}-cp311-abi3-musllinux_1_2_aarch64.whl")
    _touch(directory, f"pylibgrit-{version}-cp311-abi3-macosx_11_0_arm64.whl")


def test_valid_inventory_passes(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    assert check_inventory(tmp_path) == []


def test_missing_sdist_fails(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    (tmp_path / "pylibgrit-0.1.0.tar.gz").unlink()
    errors = check_inventory(tmp_path)
    assert any("sdist" in e for e in errors), errors


def test_extra_sdist_fails(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    _touch(tmp_path, "pylibgrit-0.1.0.zip.tar.gz")
    errors = check_inventory(tmp_path)
    assert any("sdist" in e for e in errors), errors


def test_too_few_wheels_fails(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    (tmp_path / "pylibgrit-0.1.0-cp311-abi3-macosx_11_0_arm64.whl").unlink()
    errors = check_inventory(tmp_path)
    assert any("5 wheels" in e for e in errors), errors


def test_too_many_wheels_fails(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    # A 6th valid abi3 wheel (distinct platform, same version) — only the count check
    # should fire, proving the gate rejects over-count as well as under-count.
    _touch(tmp_path, "pylibgrit-0.1.0-cp311-abi3-macosx_14_0_arm64.whl")
    errors = check_inventory(tmp_path)
    assert any("5 wheels" in e for e in errors), errors


def test_non_abi3_wheel_fails(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    (tmp_path / "pylibgrit-0.1.0-cp311-abi3-macosx_11_0_arm64.whl").unlink()
    _touch(tmp_path, "pylibgrit-0.1.0-cp311-cp311-macosx_11_0_arm64.whl")
    errors = check_inventory(tmp_path)
    assert any("abi3" in e for e in errors), errors


def test_version_mismatch_fails(tmp_path: Path) -> None:
    _good_dist(tmp_path)
    (tmp_path / "pylibgrit-0.1.0-cp311-abi3-macosx_11_0_arm64.whl").unlink()
    _touch(tmp_path, "pylibgrit-0.2.0-cp311-abi3-macosx_11_0_arm64.whl")
    errors = check_inventory(tmp_path)
    assert any("version" in e for e in errors), errors
