"""Concurrency / GIL-release regression test.

AIDEV-NOTE: `repo.odb.read` releases the GIL via `allow_threads` around grit-lib's
decompress + hash-verify (see src/odb.rs). This test runs many concurrent reads of
the SAME oid across 8 threads to prove the binding's concurrency model is sound:
the shared `Arc<Repository>` / `Arc<Mutex<..>>` odb must not deadlock, crash, or
return corrupt data under contention. It asserts CORRECTNESS only — there is NO
timing/speedup assertion (that would be flaky and environment-sensitive). If this
ever deadlocks or crashes, FIX the binding, do not weaken the test.
"""

import threading
from pathlib import Path

from tests.gitlib import rev_parse


def test_concurrent_reads(simple_repo: Path) -> None:
    import pylibgrit

    repo = pylibgrit.Repository.discover(str(simple_repo))
    oid = pylibgrit.ObjectId.from_hex(rev_parse(simple_repo, "HEAD:a.txt"))
    errors: list[BaseException] = []

    def worker() -> None:
        try:
            for _ in range(200):
                obj = repo.odb.read(oid)
                assert obj.id == oid
        except BaseException as e:  # noqa: BLE001
            errors.append(e)

    threads = [threading.Thread(target=worker) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert errors == []
