"""Tests for `repo.config` (ConfigSet getters).

AIDEV-NOTE: The pylibgrit `repo.config` call runs IN-PROCESS with the real `$HOME`
and `include_system=true`, while the `git config` writes below go to the
repo-local `.git/config` via `git_env`. Repo-local (Local scope) values WIN LAST
over system/global, so the asserted keys are deterministic regardless of host
config. The `no.such.key` negatives assert absence (None), also host-independent.
"""

import subprocess
from pathlib import Path


def test_config_get(simple_repo: Path, git_env: dict[str, str]) -> None:
    import pylibgrit

    def cfg_set(k: str, v: str) -> None:
        subprocess.run(
            ["git", "config", k, v], cwd=simple_repo, env=git_env, check=True
        )

    cfg_set("user.name", "Alice")
    cfg_set("core.bare", "false")
    cfg_set("core.repositoryformatversion", "0")
    repo = pylibgrit.Repository.discover(str(simple_repo))
    cfg = repo.config
    assert cfg.get_str("user.name") == "Alice"
    assert cfg.get_bool("core.bare") is False
    assert cfg.get_int("core.repositoryformatversion") == 0
    assert cfg.get_str("no.such.key") is None
    assert cfg.get_bool("no.such.key") is None
    assert cfg.get_int("no.such.key") is None
