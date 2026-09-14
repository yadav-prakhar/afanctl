//! The ONLY module that knows a sysfs path (PRD R1): discovery of fan files
//! and coretemp sensors, path construction, reads, writes, retries, and the
//! read-back-verify invariant.
//!
//! Invariants: every state-changing write is verified by read-back within
//! tolerance; logical state commits only on verified read-back; all rpm
//! writes clamped to [`fan1_min`, `fan1_max`] read at discovery; sensor
//! readings < 0 °C or > 120 °C rejected as failed reads.

/// Fan control mode as exposed by applesmc (`fan1_manual`: 0 = auto, 1 = manual).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode {
    Auto,
    Manual,
}

/// Snapshot of fan hardware state: actual rpm + current mode.
#[derive(Debug, Clone)]
pub struct FanState {
    pub rpm: u32,
    pub mode: FanMode,
}

/// One coretemp sensor reading. `None` = failed/invalid read (outlier
/// rejection per R1: < 0 °C or > 120 °C).
#[derive(Debug, Clone)]
pub struct SensorReading {
    pub label: String,
    pub milli_c: Option<MilliC>,
}

use crate::policy::MilliC;

/// Errors surfaced by the sysfs adapter. Messages name the path and the fix.
#[derive(Debug, thiserror::Error)]
pub enum SmcError {
    #[error("not found: {0}")]
    NotFound(std::path::PathBuf),
    #[error("read {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("write {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("verify failed: wrote {wrote}, read back {read_back}")]
    VerifyFailed { wrote: u32, read_back: u32 },
    #[error("invalid value from {path}: {value}")]
    InvalidValue {
        path: std::path::PathBuf,
        value: String,
    },
}

/// Backend trait for fan/sensor hardware access. `Send` bound is for the
/// trait's future, not for spawning (§8: single-threaded design).
pub trait Smc: Send {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError>;
    fn read_fan(&self) -> Result<FanState, SmcError>;
    fn hw_min_rpm(&self) -> u32;
    fn hw_max_rpm(&self) -> u32;
    /// Write + read-back-verify (R1 invariant). Returns the verified rpm.
    fn write_speed(&mut self, rpm: u32) -> Result<u32, SmcError>;
    /// Write + read-back-verify. Returns the verified mode.
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, SmcError>;
    /// Pre-opened O_WRONLY fd on the manual file for the L2 death path
    /// (None if backend is a mock without one).
    fn panic_fd(&self) -> Option<i32>;
}

/// Real sysfs backend. All path strings live here and nowhere else.
pub struct SysfsSmc {
    // discovery results + pre-opened fd (owned by T3)
    _private: (),
}

impl SysfsSmc {
    /// Discover fans (walk applesmc platform dir for `fan1_*`) and coretemp
    /// sensors (walk `/sys/devices/platform/coretemp.0/hwmon/hwmon*` — never
    /// hardcode hwmonN). Reads hw min/max rpm at discovery.
    pub fn open(root: &std::path::Path) -> Result<Self, SmcError> {
        let _ = root;
        unimplemented!("owned by T3")
    }
}

// Trait impl with all Appendix-A methods, each stubbed:
impl Smc for SysfsSmc {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError> {
        unimplemented!("owned by T3")
    }
    fn read_fan(&self) -> Result<FanState, SmcError> {
        unimplemented!("owned by T3")
    }
    fn hw_min_rpm(&self) -> u32 {
        unimplemented!("owned by T3")
    }
    fn hw_max_rpm(&self) -> u32 {
        unimplemented!("owned by T3")
    }
    /// INVARIANT: write + read-back-verify within K=3 retries; commit logical
    /// state only on verified read-back (R1).
    fn write_speed(&mut self, rpm: u32) -> Result<u32, SmcError> {
        let _ = rpm;
        unimplemented!("owned by T3")
    }
    /// INVARIANT: write + read-back-verify within K=3 retries (R1).
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, SmcError> {
        let _ = mode;
        unimplemented!("owned by T3")
    }
    fn panic_fd(&self) -> Option<i32> {
        unimplemented!("owned by T3")
    }
}

/// Scriptable mock backend: injectable faults, drift, latency. Test-only
/// workhorse for R10 (round-trip, drift, failed reads, mode flip).
pub struct MockSmc {
    // fault injection state (owned by T3)
    _private: (),
}

impl MockSmc {
    /// Create a mock with the given hardware (min, max) rpm range.
    pub fn new(hw_min: u32, hw_max: u32) -> Self {
        let _ = (hw_min, hw_max);
        unimplemented!("owned by T3")
    }
    // + fault injection setters, owned by T3
}

impl Smc for MockSmc {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError> {
        unimplemented!("owned by T3")
    }
    fn read_fan(&self) -> Result<FanState, SmcError> {
        unimplemented!("owned by T3")
    }
    fn hw_min_rpm(&self) -> u32 {
        unimplemented!("owned by T3")
    }
    fn hw_max_rpm(&self) -> u32 {
        unimplemented!("owned by T3")
    }
    fn write_speed(&mut self, rpm: u32) -> Result<u32, SmcError> {
        let _ = rpm;
        unimplemented!("owned by T3")
    }
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, SmcError> {
        let _ = mode;
        unimplemented!("owned by T3")
    }
    fn panic_fd(&self) -> Option<i32> {
        unimplemented!("owned by T3")
    }
}
