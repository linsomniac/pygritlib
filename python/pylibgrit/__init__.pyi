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
    "AuthenticationError",
    "Blob",
    "Commit",
    "ConfigSet",
    "Diff",
    "DiffEntry",
    "DiffStats",
    "FetchReport",
    "GritError",
    "Index",
    "IndexEntry",
    "InvalidObjectError",
    "MergeResult",
    "NetworkError",
    "Object",
    "ObjectId",
    "ObjectKind",
    "ObjectNotFoundError",
    "Odb",
    "Reference",
    "RefMismatchError",
    "RefUpdate",
    "RemoteRef",
    "Repository",
    "RepositoryError",
    "Signature",
    "Tag",
    "Tree",
    "TreeEntry",
    "ls_remote",
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

class RefMismatchError(GritError):
    """Raised when a ref's current value fails a compare-and-swap / create-only check."""

class NetworkError(GritError):
    """Raised for transport/protocol/transfer failures talking to a remote."""

class AuthenticationError(GritError):
    """Raised when a remote rejects the supplied (or absent) credentials."""

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
    def __new__(cls, name: bytes, email: bytes, when: tuple[int, int]) -> Signature: ...
    @property
    def raw(self) -> bytes: ...
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
class IndexEntry:
    def __new__(
        cls,
        path: bytes,
        oid: ObjectId,
        mode: int,
        *,
        ctime: tuple[int, int] = ...,
        mtime: tuple[int, int] = ...,
        dev: int = 0,
        ino: int = 0,
        uid: int = 0,
        gid: int = 0,
        size: int = 0,
        flags: int = 0,
    ) -> IndexEntry: ...
    @property
    def path(self) -> bytes: ...
    @property
    def oid(self) -> ObjectId: ...
    @property
    def mode(self) -> int: ...
    @property
    def ctime(self) -> tuple[int, int]: ...
    @property
    def mtime(self) -> tuple[int, int]: ...
    @property
    def dev(self) -> int: ...
    @property
    def ino(self) -> int: ...
    @property
    def uid(self) -> int: ...
    @property
    def gid(self) -> int: ...
    @property
    def size(self) -> int: ...
    @property
    def flags(self) -> int: ...

@final
class Index:
    def add(self, path: bytes, oid: ObjectId, mode: int) -> None: ...
    def add_entry(self, entry: IndexEntry) -> None: ...
    def stage(self, path: bytes | os.PathLike[str]) -> None: ...
    def remove(self, path: bytes) -> bool: ...
    def write(self, path: bytes | os.PathLike[str] | None = None) -> None: ...
    def write_tree(self) -> ObjectId: ...
    def __len__(self) -> int: ...
    def __iter__(self) -> Iterator[IndexEntry]: ...

@final
class MergeResult:
    @property
    def index(self) -> Index: ...
    @property
    def has_conflicts(self) -> bool: ...
    @property
    def conflicts(self) -> list[bytes]: ...
    def conflict_blob(self, path: bytes) -> ObjectId | None: ...
    def write_tree(self) -> ObjectId: ...

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

@final
class RemoteRef:
    @property
    def name(self) -> bytes: ...
    @property
    def oid(self) -> ObjectId: ...
    @property
    def symref_target(self) -> bytes | None: ...

@final
class RefUpdate:
    @property
    def remote_ref(self) -> bytes: ...
    @property
    def local_ref(self) -> bytes | None: ...
    @property
    def old_oid(self) -> ObjectId | None: ...
    @property
    def new_oid(self) -> ObjectId | None: ...
    @property
    def mode(self) -> str: ...
    @property
    def note(self) -> str | None: ...

@final
class FetchReport:
    @property
    def updates(self) -> list[RefUpdate]: ...
    @property
    def default_branch(self) -> bytes | None: ...

# --- Object database ------------------------------------------------------

@final
class Odb:
    def write(self, kind: ObjectKind, data: bytes) -> ObjectId: ...
    def hash(self, kind: ObjectKind, data: bytes) -> ObjectId: ...
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
    def init(
        path: str | bytes | os.PathLike[str],
        *,
        bare: bool = False,
        initial_branch: bytes | None = None,
    ) -> Repository: ...
    @staticmethod
    def discover(path: str | bytes | os.PathLike[str]) -> Repository: ...
    @staticmethod
    def open(
        git_dir: str | bytes | os.PathLike[str],
        work_tree: str | bytes | os.PathLike[str] | None = ...,
    ) -> Repository: ...
    @staticmethod
    def clone(
        url: str,
        path: str | bytes | os.PathLike[str],
        *,
        branch: str | None = None,
        username: str | None = None,
        password: str | None = None,
        use_credential_helpers: bool = True,
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
    def index(self) -> Index: ...
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
    def merge_base(self, a: ObjectId, b: ObjectId) -> ObjectId | None: ...
    def merge_trees(
        self,
        base: ObjectId,
        ours: ObjectId,
        theirs: ObjectId,
        *,
        favor: str | None = None,
    ) -> MergeResult: ...
    def merge_commits(
        self, ours: ObjectId, theirs: ObjectId, *, favor: str | None = None
    ) -> MergeResult: ...
    def commit_index(
        self,
        *,
        message: bytes,
        parents: list[ObjectId] | None = None,
        author: Signature | None = None,
        committer: Signature | None = None,
        author_raw: bytes | None = None,
        committer_raw: bytes | None = None,
        encoding: str | None = None,
    ) -> ObjectId: ...
    def create_commit(
        self,
        tree: ObjectId,
        parents: list[ObjectId],
        *,
        message: bytes,
        author: Signature | None = None,
        committer: Signature | None = None,
        author_raw: bytes | None = None,
        committer_raw: bytes | None = None,
        encoding: str | None = None,
    ) -> ObjectId: ...
    def create_tag(
        self,
        target: ObjectId,
        target_kind: ObjectKind,
        name: bytes,
        *,
        message: bytes,
        tagger: Signature | None = None,
        tagger_raw: bytes | None = None,
    ) -> ObjectId: ...
    def create_lightweight_tag(
        self, name: bytes, target: ObjectId, *, force: bool = False
    ) -> None: ...
    def create_annotated_tag(
        self,
        name: bytes,
        target: ObjectId,
        target_kind: ObjectKind,
        *,
        message: bytes,
        tagger: Signature | None = None,
        tagger_raw: bytes | None = None,
        force: bool = False,
    ) -> ObjectId: ...
    def update_ref(
        self,
        name: bytes,
        target: ObjectId,
        *,
        expected_old: ObjectId | None = None,
        create: bool = False,
        message: bytes | None = None,
        signer: Signature | None = None,
    ) -> None: ...
    def delete_ref(
        self,
        name: bytes,
        *,
        expected_old: ObjectId | None = None,
        message: bytes | None = None,
        signer: Signature | None = None,
    ) -> None: ...
    def append_reflog(
        self,
        name: bytes,
        old: ObjectId,
        new: ObjectId,
        *,
        signer: Signature,
        message: bytes,
        force_create: bool = False,
    ) -> None: ...
    def set_head(self, target: bytes) -> None: ...
    def set_symbolic_ref(self, name: bytes, target: bytes) -> None: ...
    def write_to_worktree(self, rel_path: bytes, data: bytes, mode: int) -> None: ...
    def checkout_tree(
        self, tree: ObjectId, *, force: bool = False, update_index: bool = True
    ) -> None: ...
    def fetch(
        self,
        url: str,
        refspecs: list[str] | None = None,
        *,
        tags: str = "following",
        prune: bool = False,
        username: str | None = None,
        password: str | None = None,
        use_credential_helpers: bool = True,
    ) -> FetchReport: ...

# --- Networking (read path) -----------------------------------------------

def ls_remote(
    url: str,
    *,
    username: str | None = None,
    password: str | None = None,
    use_credential_helpers: bool = True,
    heads: bool = False,
    tags: bool = False,
) -> list[RemoteRef]: ...
