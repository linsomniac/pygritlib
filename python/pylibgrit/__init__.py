"""pylibgrit — Python bindings for grit-lib."""

import enum

from pylibgrit._pylibgrit import (
    Blob,
    Commit,
    ConfigSet,
    Diff,
    DiffEntry,
    DiffStats,
    GritError,
    InvalidObjectError,
    Object,
    ObjectId,
    ObjectNotFoundError,
    Odb,
    Reference,
    Repository,
    RepositoryError,
    Signature,
    Tag,
    Tree,
    TreeEntry,
)


# AIDEV-NOTE: ObjectKind is the single source of truth for git object kinds. The
# native binding does NOT define its own enum; kind-returning getters (e.g.
# Object.kind) construct a member of THIS IntEnum by integer value (see
# src/objects.rs::kind_to_py). The discriminants below MUST match
# object_kind_discriminant() in src/objects.rs (asserted by tests/test_objectkind.py
# and exercised end-to-end by the Odb read tests' `obj.kind is ObjectKind.*` checks).
class ObjectKind(enum.IntEnum):
    """Git object kind (blob, tree, commit, tag)."""

    COMMIT = 0
    TREE = 1
    BLOB = 2
    TAG = 3


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
