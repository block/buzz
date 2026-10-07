//! Typed feature-flag keys and declared defaults.

/// Typed boolean feature definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BooleanFlag {
    key: &'static str,
    default: bool,
}

impl BooleanFlag {
    /// Construct a typed boolean flag from its stable key and declared default.
    pub const fn new(key: &'static str, default: bool) -> Self {
        Self { key, default }
    }

    /// Stable key for this flag.
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// Declared default used when an evaluator is unavailable or cannot return a value.
    pub const fn default(self) -> bool {
        self.default
    }
}

/// Typed integer feature definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntegerFlag {
    key: &'static str,
    default: i64,
}

impl IntegerFlag {
    /// Construct a typed integer flag from its stable key and declared default.
    pub const fn new(key: &'static str, default: i64) -> Self {
        Self { key, default }
    }

    /// Stable key for this flag.
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// Declared default used when an evaluator is unavailable or cannot return a value.
    pub const fn default(self) -> i64 {
        self.default
    }
}
