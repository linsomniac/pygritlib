# pygrit 6-Wheel Platform Coverage — Design

> **Update (2026-06-15): shipped as a 5-wheel grid.** During CI validation, GitHub's
> hosted **macOS-13 (Intel) runner never picked up the job** (stuck `queued` ~53 min vs
> 2–3 min for every other leg) — Intel macOS runners are on GitHub's deprecation path and
> are unreliable, and the same stall would block `release.yml`. Per maintainer decision the
> macOS Intel (x86_64) leg was **dropped**: the shipped grid is **5 wheels** (glibc
> x86_64/aarch64, musl x86_64/aarch64, macOS arm64), and Intel Macs install from the sdist
> (CI verifies the sdist compiles + imports). `EXPECTED_WHEELS` is 5. The rest of this
> document describes the original 6-target design; mentally substitute "5 / no macOS Intel".

## Goal

Widen pygrit's binary-wheel coverage from 3 targets to a full 6-target grid so
`pip install pygrit` ships a wheel for the common deployment platforms instead of
falling back to a source compile. No binding code (`src/`, `python/`) changes —
this is entirely CI / packaging / docs.

## Context (current state)

Both `ci.yml` (`build-wheels`) and `release.yml` (`build`) build **3** wheels:

| Leg | os | target | maturin `manylinux` |
| --- | --- | --- | --- |
| linux glibc x86_64 | ubuntu-latest | x86_64 | release `2014`, ci `auto` |
| linux glibc aarch64 | ubuntu-latest | aarch64 | release `2014`, ci `auto` |
| macOS arm64 | macos-14 | aarch64 | — (Linux-only setting) |

The publish gate `.github/scripts/check_release_inventory.py` hardcodes
`EXPECTED_WHEELS = 3`. `release.yml`'s header comment and both publish jobs' inventory
step names say "3 wheels". The README "Supported Python / platforms" table lists only
those three.

Gaps: **macOS Intel (x86_64)** and **musl/Alpine (musllinux) x86_64 + aarch64** users
compile the sdist instead of getting a wheel.

## Target grid (decided)

Full 6-wheel grid. `release.yml` builds all 6; `ci.yml` mirrors the 5 cheap legs
(everything except the emulated musl-aarch64), so per-PR CI gains early breakage
detection without a second slow QEMU build.

| Leg | os | target | maturin `manylinux` | smoke (release) | in ci.yml? |
| --- | --- | --- | --- | --- | --- |
| linux glibc x86_64 | ubuntu-latest | x86_64 | `2014` (ci `auto`) | native | ✓ |
| linux glibc aarch64 | ubuntu-latest | aarch64 | `2014` (ci `auto`) | QEMU container (`python:3.11-slim`) | ✓ |
| linux musl x86_64 | ubuntu-latest | x86_64 | `musllinux_1_2` | Alpine container (`python:3.11-alpine`, native docker) | ✓ |
| linux musl aarch64 | ubuntu-latest | aarch64 | `musllinux_1_2` | QEMU Alpine container | ✗ (release-only) |
| macOS x86_64 | **macos-13** | x86_64 | — | native | ✓ |
| macOS arm64 | macos-14 | aarch64 | — | native | ✓ |

Resulting wheel filenames (one per leg, all `cp311-abi3`):

```
pygrit-<ver>-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
pygrit-<ver>-cp311-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl
pygrit-<ver>-cp311-abi3-musllinux_1_2_x86_64.whl
pygrit-<ver>-cp311-abi3-musllinux_1_2_aarch64.whl
pygrit-<ver>-cp311-abi3-macosx_10_12_x86_64.whl
pygrit-<ver>-cp311-abi3-macosx_11_0_arm64.whl
```

All 6 platform tags are distinct, so the inventory checker's "count ⇒ distinct
platforms" property still holds.

## Components / changes

### 1. `release.yml` `build` matrix → 6 legs + smoke refactor

The matrix carries per-leg parameters: `target`, `manylinux` (the maturin-action
input — `2014` on glibc legs, `musllinux_1_2` on musl legs, and `auto` on the macOS
legs, where it is a no-op but keeps the input valid; defining it on *every* leg avoids
passing an empty string to maturin-action), `smoke` (`native` | `container`), and for
container legs `image` + `platform`.

Smoke modes collapse from two (`native`, `emulated`) to two generalized ones:

- **`native`** — existing setup-python@v5 (3.11) + venv install + import-smoke. Now
  also covers the macOS x86_64 leg (macos-13 is a native Intel runner).
- **`container`** — one parameterized step replacing the three near-duplicate
  container cases (glibc-arm, musl-x64, musl-arm). It runs
  `docker run --platform <matrix.platform> -v dist:/dist:ro <matrix.image> sh -c
  'pip install /dist/<wheel> && python -c "import pygrit; pygrit.Repository"'`.
  A QEMU setup step (`docker/setup-qemu-action@v3`) runs only when
  `matrix.platform == 'linux/arm64'`. `sh -c` works on both `slim` (bash present) and
  `alpine` (sh only).

The "Verify exactly one cp311-abi3 wheel" step is unchanged (each leg still produces
exactly one wheel).

**Artifact-name disambiguation:** the two `ubuntu-latest` / `x86_64` legs (glibc vs
musl) collide on the current `wheels-${{ matrix.os }}-${{ matrix.target }}` name. Since
every leg now defines `manylinux`, the upload name becomes
`wheels-${{ matrix.os }}-${{ matrix.target }}-${{ matrix.manylinux }}`, which is unique
across all 6 legs. `publish-pypi` / `publish-testpypi` already download with
`pattern: "*"` + `merge-multiple: true`, so they need no change.

### 2. `ci.yml` `build-wheels` matrix → 5 legs

Add two legs to the existing 3: macOS x86_64 (macos-13) and linux musl x86_64
(`musllinux_1_2`). The glibc legs keep `manylinux: auto` (existing intentional
divergence from release's `2014`).

- macOS x86_64 leg: `smoke: true` — runs the full installed-wheel suite (pytest +
  stubtest), exactly like the existing macos-14 leg.
- musl x86_64 leg: **import-smoke only** inside `python:3.11-alpine` (native docker, no
  QEMU). The full pytest+mypy suite already runs on glibc-x86_64 and both macOS legs;
  staging git + deps inside Alpine for the full suite adds cost for marginal value, so
  this leg just proves the wheel loads and links against musl libc.

**ci.yml disambiguation:** ci's artifact name is `wheels-${{ matrix.os }}-${{
matrix.target }}`; the two ubuntu-x86_64 legs now collide, so add a `manylinux`
discriminator to the matrix and to the artifact name. The one-shot sdist step is
currently gated `if: matrix.os == 'ubuntu-latest' && matrix.target == 'x86_64'`, which
would now fire on BOTH ubuntu-x86_64 legs — tighten its guard to the glibc leg only so
the sdist is built/uploaded once.

### 3. Inventory gate `check_release_inventory.py`

- `EXPECTED_WHEELS = 3` → `6`.
- Module docstring "three CPython abi3 wheels" → "six".
- Add an `AIDEV-NOTE` tying `EXPECTED_WHEELS` to the build-matrix leg count, so a
  future leg add/remove updates both together (sibling to the existing cp311 note).
- Per-wheel `cp311` / `abi3` checks and the single-version check are unchanged.

### 4. Test `test_check_release_inventory.py`

- Valid-set fixture: build 6 distinct-platform wheels (was 3) → still passes.
- Wrong-count case: assert that 5 (or 7) wheels fails the count check.
- Other cases (missing sdist, extra sdist, non-abi3 wheel, version mismatch) keep their
  shape, scaled to the 6-wheel baseline where they construct a wheel set.
- Runs offline via the existing `sys.path` import shim; stays outside `tests/` so CI's
  `pytest tests/` does not collect it.

### 5. Docs / comment sweep

- `release.yml` header comment "3 targets (linux x86_64/aarch64, macOS arm64)" → the
  6-leg description.
- Both publish jobs' inventory step name "Inventory — exactly 3 wheels + 1 sdist" → "6
  wheels".
- README "Supported Python / platforms" section: add the musllinux (x86_64 + aarch64)
  and macOS x86_64 rows; keep the abi3 / `requires-python >= 3.11` framing.

## Testing strategy

- **Offline / unit:** `test_check_release_inventory.py` proves the 6-wheel contract
  (valid passes; 5 and 7 fail; non-abi3 fails; version mismatch fails). This is the only
  part verifiable without a live build.
- **CI (PR):** the `wheel-coverage` branch push runs `ci.yml`'s 5 build legs +
  lint/test — green confirms the new Intel-Mac and musl-x64 wheels actually build,
  link, and import. (musl-aarch64 is not exercised until release.)
- **Release:** `release.yml`'s 6 legs build + smoke all wheels; the inventory gate
  enforces exactly 6 + 1 sdist before any publish.

## Out of scope (YAGNI)

- **Windows** — still deferred; grit-lib is Unix-oriented and may not build. A separate
  spike, not part of this change.
- **Running the full pytest suite under musl** — import-smoke is sufficient; the suite
  runs on glibc + macOS.
- **musllinux on ci.yml's aarch64** — release-only, to keep per-PR CI from carrying a
  second emulated build.
- Any change to binding code, the public API, or the test suite under `tests/`.

## Risks / notes

- maturin-action accepts musllinux values in its `manylinux` input and builds inside a
  musllinux container — this is the documented mechanism, not a hack.
- The macOS x86_64 wheel's exact platform tag (`macosx_10_12` vs `10_13`…) depends on
  the runner's deployment target; the inventory checker does not assert platform tags,
  so tag drift across runner images cannot break the gate.
- Emulated musl-aarch64 build + smoke under QEMU is the slowest leg; isolating it to
  release-only keeps everyday CI fast.
