"""Type stubs for pylibgrit — Python bindings for grit-lib.

AIDEV-NOTE: Hand-written stub for the read-core public API. The concrete
implementation lives in the native extension `pylibgrit._pylibgrit`; `__init__.py`
re-exports those symbols and defines `ObjectKind`. Keep this stub in sync with
the Rust source (src/*.rs) and `__init__.py`. Verified against the runtime with
`uv run python -m mypy.stubtest pylibgrit` (no allowlist needed).
"""

import enum
import os
from typing import Iterator, final

__all__ = [
    "Blob",
    "Commit",
    "ConfigSet",
    "Diff",
    "DiffEntry",
    "DiffStats",
    "GritError",
    "InvalidObjectError",
    "Object",
    "ObjectId",
    "ObjectKind",
    "ObjectNotFoundError",
    "Odb",
    "Reference",
    "Repository",
    "RepositoryError",
    "Signature",
    "Tag",
    "Tree",
    "TreeEntry",
]

# --- Exceptions -----------------------------------------------------------

class GritError(Exception):
    """Base class for all pylibgrit errors."""

class RepositoryError(GritError):
    """Raised for repository-level failures (open/discover/etc.)."""

class ObjectNotFoundError(GritError):
    """Raised when a requested object is not present in the object database."""

class InvalidObjectError(GritError):
    """Raised when an object cannot be parsed or is otherwise malformed."""

# --- Object kind ----------------------------------------------------------

# AIDEV-NOTE: Real enum.IntEnum defined in __init__.py (NOT the native module).
# Members and values MUST match object_kind_discriminant() in src/objects.rs.
class ObjectKind(enum.IntEnum):
    COMMIT = 0
    TREE = 1
    BLOB = 2
    TAG = 3

# --- Core value types -----------------------------------------------------

@final
class ObjectId:
    @staticmethod
    def from_hex(hex: str) -> ObjectId: ...
    @property
    def hex(self) -> str: ...
    @property
    def raw(self) -> bytes: ...
    @property
    def hash_algorithm(self) -> str: ...
    def __eq__(self, other: object, /) -> bool: ...
    def __hash__(self) -> int: ...
    def __repr__(self) -> str: ...

@final
class Object:
    @property
    def id(self) -> ObjectId: ...
    @property
    def kind(self) -> ObjectKind: ...
    @property
    def data(self) -> bytes: ...

@final
class Signature:
    @property
    def name(self) -> bytes: ...
    @property
    def email(self) -> bytes: ...
    @property
    def when(self) -> tuple[int, int]: ...
    @property
    def name_str(self) -> str: ...
    @property
    def email_str(self) -> str: ...

@final
class Commit:
    @property
    def id(self) -> ObjectId: ...
    @property
    def tree(self) -> ObjectId: ...
    @property
    def parents(self) -> list[ObjectId]: ...
    @property
    def author(self) -> Signature: ...
    @property
    def committer(self) -> Signature: ...
    @property
    def message_bytes(self) -> bytes: ...
    def message(self, encoding: str = ..., errors: str = ...) -> str: ...

@final
class TreeEntry:
    @property
    def name(self) -> bytes: ...
    @property
    def mode(self) -> int: ...
    @property
    def id(self) -> ObjectId: ...
    @property
    def kind(self) -> ObjectKind: ...

@final
class Tree:
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[TreeEntry]: ...

@final
class Blob:
    @property
    def data(self) -> bytes: ...

@final
class Tag:
    @property
    def target(self) -> ObjectId: ...
    @property
    def name(self) -> bytes: ...
    @property
    def tagger(self) -> Signature | None: ...
    @property
    def message_bytes(self) -> bytes: ...

# --- Object database ------------------------------------------------------

@final
class Odb:
    def read(self, oid: ObjectId) -> Object: ...
    def exists(self, oid: ObjectId) -> bool: ...

# --- References -----------------------------------------------------------

@final
class Reference:
    @property
    def name(self) -> bytes: ...
    @property
    def target(self) -> ObjectId | None: ...
    @property
    def symbolic_target(self) -> bytes | None: ...
    @property
    def is_symbolic(self) -> bool: ...
    def peel(self) -> ObjectId: ...

# --- Diff -----------------------------------------------------------------

@final
class DiffEntry:
    @property
    def status(self) -> str: ...
    @property
    def old_path(self) -> bytes | None: ...
    @property
    def new_path(self) -> bytes | None: ...
    @property
    def old_id(self) -> ObjectId: ...
    @property
    def new_id(self) -> ObjectId: ...

@final
class DiffStats:
    @property
    def files_changed(self) -> int: ...
    @property
    def insertions(self) -> int: ...
    @property
    def deletions(self) -> int: ...

@final
class Diff:
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[DiffEntry]: ...
    @property
    def stats(self) -> DiffStats: ...

# --- Config ---------------------------------------------------------------

@final
class ConfigSet:
    def get_str(self, key: str) -> str | None: ...
    def get_bool(self, key: str) -> bool | None: ...
    def get_int(self, key: str) -> int | None: ...

# --- Repository -----------------------------------------------------------

@final
class Repository:
    @staticmethod
    def discover(path: str | bytes | os.PathLike[str]) -> Repository: ...
    @staticmethod
    def open(
        git_dir: str | bytes | os.PathLike[str],
        work_tree: str | bytes | os.PathLike[str] | None = ...,
    ) -> Repository: ...
    @property
    def git_dir(self) -> bytes: ...
    @property
    def work_tree(self) -> bytes | None: ...
    @property
    def is_bare(self) -> bool: ...
    @property
    def odb(self) -> Odb: ...
    @property
    def config(self) -> ConfigSet: ...
    def references(self) -> Iterator[Reference]: ...
    def head(self) -> Reference: ...
    def resolve(self, spec: str) -> ObjectId: ...
    def commit(self, oid: ObjectId) -> Commit: ...
    def tree(self, oid: ObjectId) -> Tree: ...
    def blob(self, oid: ObjectId) -> Blob: ...
    def tag(self, oid: ObjectId) -> Tag: ...
    def revwalk(
        self,
        start: ObjectId,
        *,
        order: str | None = ...,
        first_parent: bool = ...,
    ) -> Iterator[Commit]: ...
    def diff(self, a: ObjectId, b: ObjectId) -> Diff: ...
