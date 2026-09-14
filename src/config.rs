//! Typed TOML configuration (PRD R6): exactly five user keys under
//! `[thresholds]`, `[curve]`, `[poll]`; strict validation; unknown-key
//! warnings (not errors); provenance (default vs file).
//!
//! Invariants: `from_toml` rejects `high >= max`, `max > 95`,
//! `min_rpm >= max_rpm`, `interval_s < 1`, and non-integer or missing keys —
//! always naming the key and the fix (kills H3 and mbpfan's missing≡0 trap).
//! `load` maps a *missing* file to `defaults()`; a present-but-invalid file is
//! refused. `validate(hw)` clamps rpms to the hardware band `(fan1_min,
//! fan1_max)` and insists the clamped band is non-empty.

use std::path::Path;

/// Dotted key paths used verbatim in `ConfigError::Invalid` messages.
const K_HIGH: &str = "thresholds.high";
const K_MAX: &str = "thresholds.max";
const K_MIN_RPM: &str = "curve.min_rpm";
const K_MAX_RPM: &str = "curve.max_rpm";
const K_INTERVAL: &str = "poll.interval_s";

/// The recognised sections and their keys; anything else is an unknown-key
/// warning (never an error — a newer config must still load).
const KNOWN: &[(&str, &[&str])] = &[
    ("thresholds", &["high", "max"]),
    ("curve", &["min_rpm", "max_rpm"]),
    ("poll", &["interval_s"]),
];

/// Typed config: the five user-tunable values (PRD R6). Safety tunables are
/// named constants in `policy.rs`/`supervisor.rs`, never config.
#[derive(Debug, Clone)]
pub struct Config {
    /// Ramp start (°C); `low_c = high_c - 3` is derived in `ResolvedConfig`.
    pub high_c: i32,
    /// Full-speed threshold (°C); guarded to <= 95 (Tjmax 100 - 5).
    pub max_c: i32,
    /// Curve floor (rpm); clamped up to hardware `fan1_min` at load.
    pub min_rpm: u32,
    /// Curve ceiling (rpm); clamped down to hardware `fan1_max` at load.
    pub max_rpm: u32,
    /// Poll period (s); `>= 1`.
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
    /// Parse and structurally validate TOML text. Returns the config plus
    /// warnings for unknown keys (a warning, not an error, so forward-
    /// compatible files still load). Missing keys, non-integer values, and
    /// contradictory values are `Invalid` errors naming the key and the fix.
    pub fn from_toml(
        s: &str,
    ) -> Result<(Self, Vec<String> /* unknown-key warnings */), ConfigError> {
        let root = toml::from_str::<toml::Table>(s)?;
        let warnings = unknown_key_warnings(&root);
        let thresholds = section(&root, "thresholds")?;
        let curve = section(&root, "curve")?;
        let poll = section(&root, "poll")?;

        let interval = read_int(poll, K_INTERVAL, "interval_s", "s")?;
        if interval < 1 {
            return Err(interval_below_one(interval));
        }
        let cfg = Self {
            high_c: to_i32(read_int(thresholds, K_HIGH, "high", "°C")?, K_HIGH)?,
            max_c: to_i32(read_int(thresholds, K_MAX, "max", "°C")?, K_MAX)?,
            min_rpm: to_u32(read_int(curve, K_MIN_RPM, "min_rpm", "rpm")?, K_MIN_RPM)?,
            max_rpm: to_u32(read_int(curve, K_MAX_RPM, "max_rpm", "rpm")?, K_MAX_RPM)?,
            interval_s: to_u64(interval, K_INTERVAL)?,
        };
        cfg.check_structural()?;
        Ok((cfg, warnings))
    }

    /// Load from a file path. Missing file => defaults (PRD R6: H5 trap —
    /// a *bad* file refuses to start, a *missing* file is fine).
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok((Self::defaults(), Vec::new()))
            }
            Err(source) => Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Validate against hardware range `hw = (fan1_min, fan1_max)`: rejects
    /// `high >= max`, `max > 95`, `min_rpm >= max_rpm`, `interval_s < 1`, and
    /// a band that empties once `min_rpm` is clamped up to `fan1_min` and
    /// `max_rpm` down to `fan1_max` (the load-time clamping rule, PRD R6).
    pub fn validate(&self, hw: (u32, u32)) -> Result<(), ConfigError> {
        self.check_structural()?;
        let (min_rpm, max_rpm) = self.clamped_rpm(hw);
        if min_rpm >= max_rpm {
            return Err(ConfigError::Invalid {
                key: K_MIN_RPM,
                reason: format!(
                    "min_rpm ({}) and max_rpm ({}) leave no usable band once clamped to hardware {}–{} rpm",
                    self.min_rpm, self.max_rpm, hw.0, hw.1
                ),
                fix: format!(
                    "set `min_rpm` below `max_rpm`, both within the hardware band {}–{} rpm",
                    hw.0, hw.1
                ),
            });
        }
        Ok(())
    }

    /// The documented defaults (PRD R6 TOML block).
    pub fn defaults() -> Self {
        Self {
            high_c: 66,
            max_c: 86,
            min_rpm: 1200,
            max_rpm: 6200,
            interval_s: 1,
        }
    }

    /// The hw-independent invariants shared by `from_toml` and `validate`.
    fn check_structural(&self) -> Result<(), ConfigError> {
        if self.max_c > 95 {
            return Err(ConfigError::Invalid {
                key: K_MAX,
                reason: format!(
                    "max ({}) exceeds the 95 °C guard (Tjmax 100 - 5)",
                    self.max_c
                ),
                fix: "set `max` to 95 or lower".to_string(),
            });
        }
        if self.high_c >= self.max_c {
            return Err(ConfigError::Invalid {
                key: K_HIGH,
                reason: format!("high ({}) must be < max ({})", self.high_c, self.max_c),
                fix: format!(
                    "set `high` below `max` (e.g. high = {})",
                    self.max_c.saturating_sub(1)
                ),
            });
        }
        if self.min_rpm >= self.max_rpm {
            return Err(ConfigError::Invalid {
                key: K_MIN_RPM,
                reason: format!(
                    "min_rpm ({}) must be < max_rpm ({})",
                    self.min_rpm, self.max_rpm
                ),
                fix: format!(
                    "set `min_rpm` below `max_rpm` (e.g. min_rpm = {})",
                    self.max_rpm.saturating_sub(1)
                ),
            });
        }
        if self.interval_s == 0 {
            return Err(interval_below_one(0));
        }
        Ok(())
    }

    /// The load-time clamp rule (PRD R6): `min_rpm` up to `fan1_min`, `max_rpm`
    /// down to `fan1_max`. `validate` checks the result; the load path applies
    /// the same rule when building `ResolvedConfig`.
    fn clamped_rpm(&self, hw: (u32, u32)) -> (u32, u32) {
        (self.min_rpm.max(hw.0), self.max_rpm.min(hw.1))
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
    /// Derive the hysteresis floor `low_c = high_c - 3` (PRD R6). Saturating,
    /// so an unvalidated `Config` can never panic here.
    fn from(config: &Config) -> Self {
        Self {
            config: config.clone(),
            low_c: config.high_c.saturating_sub(3),
        }
    }
}

/// Fetch a required `[section]` as a table, or explain the fix.
fn section<'a>(root: &'a toml::Table, name: &'static str) -> Result<&'a toml::Table, ConfigError> {
    match root.get(name) {
        Some(value) => value.as_table().ok_or_else(|| ConfigError::Invalid {
            key: name,
            reason: format!("expected a table, got {value}"),
            fix: format!("write `[{name}]` with its keys"),
        }),
        None => Err(ConfigError::Invalid {
            key: name,
            reason: "missing section".to_string(),
            fix: format!("add a `[{name}]` table"),
        }),
    }
}

/// Fetch a required integer under `table`, naming `full_key` on failure.
fn read_int(
    table: &toml::Table,
    full_key: &'static str,
    leaf: &str,
    unit: &str,
) -> Result<i64, ConfigError> {
    let value = table.get(leaf).ok_or_else(|| ConfigError::Invalid {
        key: full_key,
        reason: "missing (required when a config file is present)".to_string(),
        fix: format!("add `{leaf} = <integer {unit}>`"),
    })?;
    value.as_integer().ok_or_else(|| ConfigError::Invalid {
        key: full_key,
        reason: format!("expected an integer {unit}, got {value}"),
        fix: format!("set `{leaf}` to a whole number ({unit})"),
    })
}

/// Unknown keys anywhere (top-level or inside a known section) become
/// warnings, sorted for deterministic output.
fn unknown_key_warnings(root: &toml::Table) -> Vec<String> {
    let mut warnings = Vec::new();
    for (name, value) in root {
        match KNOWN.iter().find(|(known, _)| *known == name.as_str()) {
            Some((_, keys)) => {
                if let Some(section) = value.as_table() {
                    for key in section.keys() {
                        if !keys.contains(&key.as_str()) {
                            warnings.push(format!("unknown config key `{name}.{key}` ignored"));
                        }
                    }
                }
            }
            None => warnings.push(format!("unknown config key `{name}` ignored")),
        }
    }
    warnings.sort();
    warnings
}

fn to_i32(v: i64, key: &'static str) -> Result<i32, ConfigError> {
    i32::try_from(v).map_err(|_| ConfigError::Invalid {
        key,
        reason: format!("out of range: {v}"),
        fix: "use an integer between -2147483648 and 2147483647".to_string(),
    })
}

fn to_u32(v: i64, key: &'static str) -> Result<u32, ConfigError> {
    u32::try_from(v).map_err(|_| ConfigError::Invalid {
        key,
        reason: format!("out of range: {v}"),
        fix: "use a non-negative integer (0..=4294967295)".to_string(),
    })
}

fn to_u64(v: i64, key: &'static str) -> Result<u64, ConfigError> {
    u64::try_from(v).map_err(|_| ConfigError::Invalid {
        key,
        reason: format!("out of range: {v}"),
        fix: "use a non-negative integer".to_string(),
    })
}

fn interval_below_one(v: i64) -> ConfigError {
    ConfigError::Invalid {
        key: K_INTERVAL,
        reason: format!("interval_s ({v}) must be >= 1"),
        fix: "set `interval_s` to 1 or more seconds".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped install-time default, embedded so it is tested by the gate.
    const DEFAULT_TOML: &str = include_str!("../packaging/afanctl.toml.default");

    /// Real A1708 hardware band from the fixture tree.
    const HW: (u32, u32) = (1200, 7200);

    fn assert_defaults(c: &Config) {
        assert_eq!(c.high_c, 66);
        assert_eq!(c.max_c, 86);
        assert_eq!(c.min_rpm, 1200);
        assert_eq!(c.max_rpm, 6200);
        assert_eq!(c.interval_s, 1);
    }

    /// A complete config with single values swapped in for the five keys.
    fn toml_with(high: &str, max: &str, min_rpm: &str, max_rpm: &str, interval: &str) -> String {
        format!(
            "[thresholds]\nhigh = {high}\nmax = {max}\n\
             [curve]\nmin_rpm = {min_rpm}\nmax_rpm = {max_rpm}\n\
             [poll]\ninterval_s = {interval}\n"
        )
    }

    fn err(toml: &str) -> ConfigError {
        Config::from_toml(toml).expect_err("expected the config to be rejected")
    }

    /// The rejection must name the offending key and carry a fix (§8).
    fn assert_invalid(e: &ConfigError, key: &str) {
        let msg = e.to_string();
        assert!(msg.contains(key), "message `{msg}` should name `{key}`");
        assert!(msg.contains("fix:"), "message `{msg}` should carry a fix");
    }

    fn cfg(high_c: i32, max_c: i32, min_rpm: u32, max_rpm: u32, interval_s: u64) -> Config {
        Config {
            high_c,
            max_c,
            min_rpm,
            max_rpm,
            interval_s,
        }
    }

    #[test]
    fn defaults_are_the_documented_values() {
        assert_defaults(&Config::defaults());
    }

    #[test]
    fn shipped_default_file_parses_to_defaults_without_warnings() {
        let (parsed, warnings) = Config::from_toml(DEFAULT_TOML).expect("default file parses");
        assert_defaults(&parsed);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert!(parsed.validate(HW).is_ok());
    }

    #[test]
    fn parses_all_five_keys() {
        let (parsed, warnings) =
            Config::from_toml(&toml_with("60", "80", "1500", "5000", "2")).expect("parses");
        assert_eq!(parsed.high_c, 60);
        assert_eq!(parsed.max_c, 80);
        assert_eq!(parsed.min_rpm, 1500);
        assert_eq!(parsed.max_rpm, 5000);
        assert_eq!(parsed.interval_s, 2);
        assert!(warnings.is_empty());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path = Path::new("/nonexistent/afanctl/afanctl.toml");
        let (parsed, warnings) = Config::load(path).expect("a missing file is not an error");
        assert_defaults(&parsed);
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_reads_a_present_file() {
        let path = std::env::temp_dir().join(format!("afanctl-t1-ok-{}.toml", std::process::id()));
        std::fs::write(&path, DEFAULT_TOML).expect("write temp config");
        let result = Config::load(&path);
        std::fs::remove_file(&path).ok();
        let (parsed, warnings) = result.expect("present file parses");
        assert_defaults(&parsed);
        assert!(warnings.is_empty());
    }

    #[test]
    fn present_but_invalid_file_is_refused() {
        let path = std::env::temp_dir().join(format!("afanctl-t1-bad-{}.toml", std::process::id()));
        std::fs::write(&path, toml_with("86", "86", "1200", "6200", "1"))
            .expect("write temp config");
        let result = Config::load(&path);
        std::fs::remove_file(&path).ok();
        assert_invalid(&result.expect_err("bad file refused"), K_HIGH);
    }

    #[test]
    fn rejects_malformed_toml_as_parse_error() {
        assert!(matches!(err("[thresholds"), ConfigError::Parse(_)));
    }

    #[test]
    fn rejects_high_above_max() {
        assert_invalid(&err(&toml_with("90", "80", "1200", "6200", "1")), K_HIGH);
    }

    #[test]
    fn rejects_high_equal_to_max_h3_landmine() {
        assert_invalid(&err(&toml_with("86", "86", "1200", "6200", "1")), K_HIGH);
    }

    #[test]
    fn rejects_max_above_95_guard() {
        assert_invalid(&err(&toml_with("90", "99", "1200", "6200", "1")), K_MAX);
    }

    #[test]
    fn rejects_min_rpm_not_below_max_rpm() {
        assert_invalid(&err(&toml_with("66", "86", "5000", "5000", "1")), K_MIN_RPM);
    }

    #[test]
    fn rejects_interval_below_one() {
        for v in ["0", "-1"] {
            let e = err(&toml_with("66", "86", "1200", "6200", v));
            assert_invalid(&e, K_INTERVAL);
            assert!(e.to_string().contains(">= 1"), "got `{e}`");
        }
    }

    #[test]
    fn rejects_non_integer_temperature() {
        assert_invalid(&err(&toml_with("66.5", "86", "1200", "6200", "1")), K_HIGH);
    }

    #[test]
    fn rejects_non_integer_rpm_and_interval() {
        assert_invalid(
            &err(&toml_with("66", "86", "1200", "6200.5", "1")),
            K_MAX_RPM,
        );
        assert_invalid(
            &err(&toml_with("66", "86", "1200", "6200", "1.5")),
            K_INTERVAL,
        );
    }

    #[test]
    fn rejects_negative_rpm() {
        assert_invalid(&err(&toml_with("66", "86", "-5", "6200", "1")), K_MIN_RPM);
    }

    #[test]
    fn rejects_missing_key_naming_it() {
        let src = "[thresholds]\nhigh = 66\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n\
                   [poll]\ninterval_s = 1\n";
        assert_invalid(&err(src), K_MAX);
    }

    #[test]
    fn rejects_missing_section_naming_it() {
        let src = "[thresholds]\nhigh = 66\nmax = 86\n[poll]\ninterval_s = 1\n";
        assert_invalid(&err(src), "curve");
    }

    #[test]
    fn rejects_non_table_section() {
        let src = "thresholds = 5\n[curve]\nmin_rpm = 1200\nmax_rpm = 6200\n\
                   [poll]\ninterval_s = 1\n";
        assert_invalid(&err(src), "thresholds");
    }

    #[test]
    fn collects_unknown_key_warnings_but_still_parses() {
        let src = "[thresholds]\nhigh = 66\nmax = 86\nfahrenheit = true\n\
                   [curve]\nmin_rpm = 1200\nmax_rpm = 6200\n\
                   [poll]\ninterval_s = 1\n[extra]\nfoo = 1\n";
        let (parsed, warnings) =
            Config::from_toml(src).expect("unknown keys are warnings, not errors");
        assert_defaults(&parsed);
        assert_eq!(warnings.len(), 2, "warnings: {warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("thresholds.fahrenheit")));
        assert!(warnings.iter().any(|w| w.contains("extra")));
    }

    #[test]
    fn unknown_keys_are_ignored_not_applied() {
        let src = "[thresholds]\nhigh = 66\nmax = 86\nhi = 1\nlo = 2\n\
                   [curve]\nmin_rpm = 1200\nmax_rpm = 6200\nturbo = 3\n\
                   [poll]\ninterval_s = 1\nperiod = 4\n";
        let (parsed, warnings) = Config::from_toml(src).expect("parses");
        assert_defaults(&parsed);
        assert_eq!(warnings.len(), 4, "warnings: {warnings:?}");
    }

    #[test]
    fn clamps_rpm_band_to_hardware_tuple() {
        assert_eq!(cfg(66, 86, 500, 9000, 1).clamped_rpm(HW), (1200, 7200));
        assert_eq!(cfg(66, 86, 1200, 6200, 1).clamped_rpm(HW), (1200, 6200));
        assert_eq!(
            cfg(66, 86, 2000, 3000, 1).clamped_rpm((3000, 5000)),
            (3000, 3000)
        );
    }

    #[test]
    fn validate_accepts_a_band_that_survives_clamping() {
        assert!(cfg(66, 86, 500, 9000, 1).validate(HW).is_ok());
    }

    #[test]
    fn validate_rejects_a_band_that_empties_after_clamping() {
        let e = cfg(66, 86, 500, 600, 1)
            .validate(HW)
            .expect_err("empty band rejected");
        assert_invalid(&e, K_MIN_RPM);
    }

    #[test]
    fn validate_rechecks_structural_invariants() {
        assert!(cfg(86, 86, 1200, 6200, 1).validate(HW).is_err());
        assert!(cfg(66, 86, 1200, 6200, 0).validate(HW).is_err());
    }

    #[test]
    fn resolved_config_derives_low_c() {
        let resolved = ResolvedConfig::from(&Config::defaults());
        assert_eq!(resolved.low_c, 63);
        assert_eq!(resolved.config.high_c, 66);
    }
}
