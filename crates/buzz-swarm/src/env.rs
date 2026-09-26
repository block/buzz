//! A snapshot of the process environment.
//!
//! Every environment read goes through this type so that secret resolution
//! and `PATH` lookup are pure functions of an explicit input and can
//! be exercised in tests without mutating global process state.

use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct Env {
    vars: BTreeMap<String, Option<String>>,
}

impl Env {
    /// Preserve variable names even when values are not UTF-8, so identity and
    /// ACP settings can always be removed from inherited child environments.
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars_os()
                .filter_map(|(key, value)| {
                    Some((key.into_string().ok()?, value.into_string().ok()))
                })
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).and_then(Option::as_deref)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.vars.keys().map(String::as_str)
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Env {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            vars: iter
                .into_iter()
                .map(|(k, v)| (k.into(), Some(v.into())))
                .collect(),
        }
    }
}
