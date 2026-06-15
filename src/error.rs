//! Exception hierarchy and error mapping for pylibgrit.
//!
//! Maps `grit_lib::error::Error` onto a mutually-exclusive Python exception
//! hierarchy (design §7): a base `GritError` with three direct, mutually
//! exclusive subclasses (`RepositoryError`, `ObjectNotFoundError`,
//! `InvalidObjectError`). `GritError` is the always-reachable fallback.

use pyo3::prelude::*;
use pyo3::{
    create_exception,
    exceptions::{PyException, PyOSError, PyValueError},
};

create_exception!(
    _pylibgrit,
    GritError,
    PyException,
    "Base class for all pylibgrit errors."
);
create_exception!(
    _pylibgrit,
    RepositoryError,
    GritError,
    "Repository discover/open/config failure."
);
create_exception!(
    _pylibgrit,
    ObjectNotFoundError,
    GritError,
    "A requested object or ref id does not exist."
);
create_exception!(
    _pylibgrit,
    InvalidObjectError,
    GritError,
    "Malformed id or corrupt/undecodable object."
);

// AIDEV-NOTE: We CANNOT write `impl From<grit_lib::error::Error> for PyErr` — the
// orphan rule forbids it because BOTH `grit_lib::error::Error` and `PyErr` are
// foreign types to this crate. So error mapping is a free function instead.
//
// AIDEV-NOTE: `grit_lib::error::Error` is `#[non_exhaustive]`, so the match MUST
// end with a catch-all `_ =>` arm that maps to the base `GritError`; without it
// the crate would fail to compile against any future grit-lib that adds a variant.
// Each arm formats the source error (`format!("{e}")`) into the message so the
// offending path/OID/message is preserved for the caller.
pub fn map_err(e: grit_lib::error::Error) -> PyErr {
    use grit_lib::error::Error;
    let msg = format!("{e}");
    match e {
        // Repository discover/open/config/ref failures.
        Error::NotARepository(_)
        | Error::ForbiddenBareRepository(_)
        | Error::DubiousOwnership(_)
        | Error::UnsupportedRepositoryFormatVersion(_)
        | Error::UnsupportedRepositoryExtension(_)
        | Error::InvalidRef(_)
        | Error::ConfigError(_) => RepositoryError::new_err(msg),

        // A requested object/ref id does not exist.
        Error::ObjectNotFound(_) => ObjectNotFoundError::new_err(msg),

        // Malformed id / corrupt or undecodable object.
        Error::InvalidObjectId(_)
        | Error::CorruptObject(_)
        | Error::UnknownObjectType(_)
        | Error::ObjectHeaderTooLong { .. }
        | Error::Zlib(_)
        | Error::LooseHashMismatch { .. } => InvalidObjectError::new_err(msg),

        // Underlying I/O — surface as OSError, preserving errno where available.
        Error::Io(io_err) => match io_err.raw_os_error() {
            Some(errno) => PyOSError::new_err((errno, format!("{io_err}"))),
            None => PyOSError::new_err(format!("{io_err}")),
        },

        // Bad path argument shape.
        Error::PathError(_) => PyValueError::new_err(msg),

        // Everything else (index, cache-tree, signing, auth, push-options,
        // generic message) plus any future `#[non_exhaustive]` variant.
        _ => GritError::new_err(msg),
    }
}

// AIDEV-NOTE: Small helper for binding-layer ref errors that do not originate from a
// `grit_lib::error::Error` (e.g. a non-UTF-8 ref name we refuse to pass to grit-lib's
// `&str`-typed APIs). Maps to `RepositoryError`, the subclass for ref/repository faults.
pub fn invalid_ref(msg: &str) -> PyErr {
    RepositoryError::new_err(msg.to_owned())
}

/// Registers the exception types on the native module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("GritError", m.py().get_type::<GritError>())?;
    m.add("RepositoryError", m.py().get_type::<RepositoryError>())?;
    m.add(
        "ObjectNotFoundError",
        m.py().get_type::<ObjectNotFoundError>(),
    )?;
    m.add(
        "InvalidObjectError",
        m.py().get_type::<InvalidObjectError>(),
    )?;
    Ok(())
}
