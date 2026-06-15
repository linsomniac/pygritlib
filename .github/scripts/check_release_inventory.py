#!/usr/bin/env python3
"""Validate the release artifact set before publishing to PyPI.

Asserts that a ``dist/`` directory contains exactly the artifacts the release
workflow is expected to produce: five CPython ``abi3`` wheels (one per target
platform) and one sdist, every file carrying the same project version. Any
deviation -- a missing or extra artifact, a version disagreement, a non-abi3
wheel -- is a hard error, so a malformed upload set can never reach PyPI.

Because each target platform yields a uniquely named wheel, "exactly 5 wheels"
already implies five distinct platforms; there is no separate platform-tag
check (the exact tag strings shift with runner images and would be brittle).

Usage: ``python check_release_inventory.py <dist-dir>``
"""

from __future__ import annotations

import sys
from pathlib import Path

DIST_NAME = "pylibgrit"
# AIDEV-NOTE: EXPECTED_WHEELS must equal the number of build legs in the release
# workflow's `build` matrix (.github/workflows/release.yml). Bump both together when
# adding or removing a wheel target.
EXPECTED_WHEELS = 5
SDIST_SUFFIX = ".tar.gz"
WHEEL_SUFFIX = ".whl"


def check_inventory(dist_dir: Path) -> list[str]:
    """Return a list of human-readable problems with ``dist_dir`` (empty == OK)."""
    errors: list[str] = []
    versions: set[str] = set()

    sdists = sorted(dist_dir.glob(f"*{SDIST_SUFFIX}"))
    if len(sdists) != 1:
        names = [p.name for p in sdists]
        errors.append(
            f"expected exactly 1 sdist (*{SDIST_SUFFIX}), found {len(sdists)}: {names}"
        )
    for sdist in sdists:
        stem = sdist.name[: -len(SDIST_SUFFIX)]
        parts = stem.split("-")
        if len(parts) != 2 or parts[0] != DIST_NAME:
            errors.append(f"unexpected sdist filename: {sdist.name}")
            continue
        versions.add(parts[1])

    wheels = sorted(dist_dir.glob(f"*{WHEEL_SUFFIX}"))
    if len(wheels) != EXPECTED_WHEELS:
        names = [p.name for p in wheels]
        errors.append(
            f"expected exactly {EXPECTED_WHEELS} wheels, found {len(wheels)}: {names}"
        )
    for wheel in wheels:
        stem = wheel.name[: -len(WHEEL_SUFFIX)]
        parts = stem.split("-")
        # Expected: name-version-pythontag-abitag-platformtag (no build tag).
        if len(parts) != 5 or parts[0] != DIST_NAME:
            errors.append(f"unexpected wheel filename: {wheel.name}")
            continue
        _, version, python_tag, abi_tag, _platform = parts
        versions.add(version)
        # AIDEV-NOTE: cp311 is the project's minimum Python (abi3 floor); update this
        # tag here when bumping requires-python in pyproject.toml / Cargo.toml abi3-py*.
        if python_tag != "cp311" or abi_tag != "abi3":
            errors.append(f"wheel is not cp311-abi3: {wheel.name}")

    if len(versions) > 1:
        errors.append(f"artifacts disagree on version: {sorted(versions)}")

    return errors


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} <dist-dir>", file=sys.stderr)
        return 2
    dist_dir = Path(argv[1])
    if not dist_dir.is_dir():
        print(f"not a directory: {dist_dir}", file=sys.stderr)
        return 2
    errors = check_inventory(dist_dir)
    if errors:
        for err in errors:
            print(f"::error::release inventory: {err}", file=sys.stderr)
        return 1
    print(f"release inventory OK: {EXPECTED_WHEELS} wheels + 1 sdist, single version")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
