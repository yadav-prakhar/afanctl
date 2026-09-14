//! Typed TOML configuration (PRD R6): exactly five user keys under
//! `[thresholds]`, `[curve]`, `[poll]`; strict validation; unknown-key
//! warnings (not errors); provenance (default vs file).
//!
//! Invariants: validation rejects `high >= max`, `max > 95`,
//! `min_rpm >= max_rpm`, `interval_s < 1`, non-integer temps — with key name
//! and fix in the error (kills H3 + missing≡0 trap). Invalid config = refuse
//! to start, never run on defaults after a bad edit.

/// Typed config: the five user-tunable values (PRD R6). Safety tunables are
/// named constants in `policy.rs`/`supervisor.rs`, never config.
#[derive(Debug, Clone)]
pub struct Config {
    pub high_c: i32,
    pub max_c: i32,
    pub min_rpm: u32,
    pub max_rpm: u32,
    pub interval_s: u64,
}

/// Configuration errors. `Invalid` messages carry the key, the reason, and
/// the fix (§8 style: errors name the path/key and the fix).
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("parse: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("io {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("invalid {key}: {reason} (fix: {fix})")]
    Invalid {
        key: &'static str,
        reason: String,
        fix: String,
    },
}

impl Config {
    /// Parse from TOML text. Returns the config plus unknown-key warnings.
    pub fn from_toml(
        s: &str,
    ) -> Result<(Self, Vec<String> /* unknown-key warnings */), ConfigError> {
        let _ = s;
        unimplemented!("owned by T1")
    }

    /// Load from a file path. Missing file => defaults (PRD R6: H5 trap —
    /// a *bad* file refuses to start, a *missing* file is fine).
    pub fn load(path: &std::path::Path) -> Result<(Self, Vec<String>), ConfigError> {
        let _ = path;
        unimplemented!("owned by T1")
    }

    /// Validate against hardware range `hw = (fan1_min, fan1_max)`:
    /// rejects `high >= max`, `max > 95`, `min_rpm >= max_rpm`,
    /// `interval_s < 1`, out-of-hw-range rpms (before clamping at load).
    pub fn validate(&self, hw: (u32, u32)) -> Result<(), ConfigError> {
        let _ = hw;
        unimplemented!("owned by T1")
    }

    /// The documented defaults (PRD R6 TOML block).
    pub fn defaults() -> Self {
        unimplemented!("owned by T1")
    }
}

/// Config + derived values, fully validated against hardware. What
/// `Controller` consumes.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub config: Config,
    pub low_c: i32, // = high-3
}

impl From<&Config> for ResolvedConfig {
    fn from(_config: &Config) -> Self {
        // Derived-value wrapper; real validation/clamp path is owned by T1.
        unimplemented!("owned by T1")
    }
}
