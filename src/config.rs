//! Python wrapper over `grit_lib::config::ConfigSet` (read-only getters).
//!
//! A `ConfigSet` is the merged, layered view of git configuration
//! (system < global < local < worktree < command). `repo.config` loads the
//! effective cascade (like git) and hands back one of these. Only the read
//! getters are exposed here — set/unset live with the round-trip `ConfigFile`
//! API, which is not part of this binding subsystem yet.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

// AIDEV-NOTE: NOT `frozen`, and that's fine: every method takes `&self` and never
// mutates, so the immutability `frozen` would enforce buys us nothing here. We hold
// the inner ConfigSet BY VALUE (it owns its `Vec<ConfigEntry>`), so this handle is
// fully self-contained and outlives the `Repository` it was loaded from.
#[pyclass(module = "pylibgrit._pylibgrit")]
pub struct ConfigSet {
    inner: grit_lib::config::ConfigSet,
}

#[pymethods]
impl ConfigSet {
    // AIDEV-NOTE: `get` is last-wins across the merged cascade. A BARE boolean key
    // (e.g. a lone `[core]\n\tbare` with no `=`) returns "true" here, matching git.
    // Returns None when the key is absent in every layer.
    fn get_str(&self, key: &str) -> Option<String> {
        self.inner.get(key)
    }

    // AIDEV-NOTE: grit returns Option<Result<bool,String>>: None=absent,
    // Some(Err)=present-but-not-a-valid-bool. We surface a parse failure as
    // ValueError (bad config value), absent as None. Truthy/falsey spellings
    // (true/yes/on/1 vs false/no/off/0) are decided inside grit's parse_bool.
    fn get_bool(&self, key: &str) -> PyResult<Option<bool>> {
        match self.inner.get_bool(key) {
            None => Ok(None),
            Some(Ok(b)) => Ok(Some(b)),
            Some(Err(msg)) => Err(PyValueError::new_err(format!("config key {key:?}: {msg}"))),
        }
    }

    // AIDEV-NOTE: Same shape as get_bool. grit's get_i64 supports git's k/m/g
    // suffixes; an unparseable value yields Some(Err) -> ValueError.
    fn get_int(&self, key: &str) -> PyResult<Option<i64>> {
        match self.inner.get_i64(key) {
            None => Ok(None),
            Some(Ok(n)) => Ok(Some(n)),
            Some(Err(msg)) => Err(PyValueError::new_err(format!("config key {key:?}: {msg}"))),
        }
    }
}

impl ConfigSet {
    pub fn new(inner: grit_lib::config::ConfigSet) -> Self {
        Self { inner }
    }
}
