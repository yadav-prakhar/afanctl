//! The ONLY module that knows a sysfs path (PRD R1): discovery of fan files
//! and coretemp sensors, path construction, reads, writes, retries, and the
//! read-back-verify invariant.
//!
//! Invariants: every state-changing write (`fan1_output`, `fan1_manual`) is
//! followed by a read-back within `VERIFY_TOLERANCE_RPM` (mode: exact match),
//! retried up to K=3 attempts; logical state commits only on verified
//! read-back; all rpm writes clamped to [`fan1_min`, `fan1_max`] read at
//! discovery; sensor readings < 0 °C or > 120 °C rejected as failed reads.

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

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::policy::VERIFY_TOLERANCE_RPM;

// ---- sysfs path pieces (PRD §2.1): the only path strings in the crate ----
const PLATFORM: &str = "devices/platform";
const APPLESMC: &str = "applesmc.768";
const CORETEMP: &str = "coretemp.0";
const HWMON: &str = "hwmon";
const FAN_INPUT: &str = "fan1_input";
const FAN_OUTPUT: &str = "fan1_output";
const FAN_MANUAL: &str = "fan1_manual";
const FAN_MIN: &str = "fan1_min";
const FAN_MAX: &str = "fan1_max";

/// Verify attempts per state-changing write (PRD R1: K=3), then `VerifyFailed`.
const VERIFY_ATTEMPTS: u32 = 3;
/// Outlier window (PRD R1, milli-°C): readings outside [0, 120] °C are failed reads.
const MILLI_C_MIN: i32 = 0;
const MILLI_C_MAX: i32 = 120_000;

/// Read + trim a sysfs file; io errors carry the path.
fn read_string(path: &Path) -> Result<String, SmcError> {
    fs::read_to_string(path).map_err(|source| SmcError::Read {
        path: path.to_owned(),
        source,
    })
}

/// Parse a u32 sysfs value (rpm files); non-numeric/empty content is `InvalidValue`.
fn read_u32(path: &Path) -> Result<u32, SmcError> {
    let raw = read_string(path)?;
    let trimmed = raw.trim();
    trimmed.parse::<u32>().map_err(|_| SmcError::InvalidValue {
        path: path.to_owned(),
        value: trimmed.to_owned(),
    })
}

/// Parse `fan1_manual` content: "0" = Auto, "1" = Manual, anything else invalid.
fn parse_manual(path: &Path, raw: &str) -> Result<FanMode, SmcError> {
    match raw.trim() {
        "0" => Ok(FanMode::Auto),
        "1" => Ok(FanMode::Manual),
        other => Err(SmcError::InvalidValue {
            path: path.to_owned(),
            value: other.to_owned(),
        }),
    }
}

/// One sensor-file read with outlier rejection (R1): io errors, unparseable
/// content, and values outside [0, 120] °C all degrade to `None` (failed
/// read) so the other sensors can still drive `t_eff` (R2's H1 lesson).
fn read_sensor(input: &Path, label: &str) -> SensorReading {
    let milli_c = match fs::read_to_string(input) {
        Ok(raw) => match raw.trim().parse::<i32>() {
            Ok(v) if (MILLI_C_MIN..=MILLI_C_MAX).contains(&v) => Some(MilliC(v)),
            Ok(v) => {
                tracing::warn!(path = %input.display(), value = v, "sensor outlier rejected (outside 0..120 °C); failed read");
                None
            }
            Err(_) => {
                tracing::warn!(path = %input.display(), raw = %raw.trim(), "unparseable sensor value; failed read");
                None
            }
        },
        Err(source) => {
            tracing::warn!(path = %input.display(), error = %source, "sensor read failed; failed read");
            None
        }
    };
    SensorReading {
        label: label.to_owned(),
        milli_c,
    }
}

/// Walk `<platform>/hwmon/hwmon*/` (sorted — never hardcode hwmonN, Q5) for a
/// dir that contains `fan1_input`. `None` if the platform has no such dir.
fn hwmon_dir_with_fan_attrs(platform: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(platform.join(HWMON))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.into_iter().find(|d| d.join(FAN_INPUT).exists())
}

/// Locate the dir holding `fan1_*`: the applesmc platform dir itself, or —
/// post hwmon conversion (Q4) — a hwmon subdir of it.
fn discover_fan_dir(root: &Path) -> Result<PathBuf, SmcError> {
    let platform = root.join(PLATFORM).join(APPLESMC);
    if !platform.is_dir() {
        return Err(SmcError::NotFound(platform));
    }
    if platform.join(FAN_INPUT).exists() {
        return Ok(platform);
    }
    hwmon_dir_with_fan_attrs(&platform).ok_or_else(|| SmcError::NotFound(platform.join(FAN_INPUT)))
}

/// Walk `<root>/devices/platform/coretemp.0/hwmon/hwmon*/` (sorted) for
/// `tempN_input` (+ optional `tempN_label`), discovery-ordered by (dir, N).
fn discover_sensors(root: &Path) -> Result<Vec<(PathBuf, String)>, SmcError> {
    let coretemp = root.join(PLATFORM).join(CORETEMP);
    if !coretemp.is_dir() {
        return Err(SmcError::NotFound(coretemp));
    }
    let hwmon_root = coretemp.join(HWMON);
    let mut dirs: Vec<PathBuf> = fs::read_dir(&hwmon_root)
        .map_err(|source| SmcError::Read {
            path: hwmon_root.clone(),
            source,
        })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut sensors = Vec::new();
    for dir in dirs {
        let mut inputs: Vec<(u32, PathBuf)> = fs::read_dir(&dir)
            .map_err(|source| SmcError::Read {
                path: dir.clone(),
                source,
            })?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let n = name
                    .strip_prefix("temp")?
                    .strip_suffix("_input")?
                    .parse::<u32>()
                    .ok()?;
                Some((n, e.path()))
            })
            .collect();
        inputs.sort();
        for (n, input) in inputs {
            // Missing/unreadable label falls back to the attribute stem.
            let label = fs::read_to_string(dir.join(format!("temp{n}_label")))
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| format!("temp{n}"));
            sensors.push((input, label));
        }
    }
    if sensors.is_empty() {
        return Err(SmcError::NotFound(hwmon_root));
    }
    Ok(sensors)
}

/// Real sysfs backend. All path strings live here and nowhere else.
pub struct SysfsSmc {
    /// applesmc platform dir (classic home of `fan1_*`, matches the unit's
    /// pinned `ReadWritePaths` — DESIGN.md Appendix D).
    platform_dir: PathBuf,
    /// Dir actually holding `fan1_*` files (platform dir, or a hwmon subdir
    /// post-conversion — see Q4).
    fan_dir: PathBuf,
    /// (tempN_input path, label) per coretemp sensor, in discovery order.
    sensors: Vec<(PathBuf, String)>,
    hw_min: u32,
    hw_max: u32,
    /// Pre-opened O_WRONLY handle on `fan1_manual` for the L2 death path.
    panic_file: Option<File>,
}

impl SysfsSmc {
    /// Discover fans (walk the applesmc platform dir for `fan1_*`) and
    /// coretemp sensors (walk `/sys/devices/platform/coretemp.0/hwmon/hwmon*`
    /// — never hardcode hwmonN). Reads hw min/max rpm at discovery.
    pub fn open(root: &Path) -> Result<Self, SmcError> {
        let platform_dir = root.join(PLATFORM).join(APPLESMC);
        let fan_dir = discover_fan_dir(root)?;
        let hw_min = read_u32(&fan_dir.join(FAN_MIN))?;
        let hw_max = read_u32(&fan_dir.join(FAN_MAX))?;
        if hw_min > hw_max {
            return Err(SmcError::InvalidValue {
                path: fan_dir.join(FAN_MIN),
                value: format!("{hw_min} > {hw_max} (hw range invalid; fix fan1_min/fan1_max)"),
            });
        }
        let sensors = discover_sensors(root)?;
        // L2 death-path fd: O_WRONLY on fan1_manual, pre-opened once. On a
        // read-only tree this degrades to None + loud log (doctor/status need
        // read-only opens); the supervisor decides whether a daemon may run
        // without it (R4 policy is not this module's call).
        let panic_file = match OpenOptions::new()
            .write(true)
            .open(fan_dir.join(FAN_MANUAL))
        {
            Ok(file) => Some(file),
            Err(source) => {
                tracing::warn!(path = %fan_dir.join(FAN_MANUAL).display(), %source, "cannot pre-open fan1_manual O_WRONLY; L2 death path unavailable");
                None
            }
        };
        Ok(Self {
            platform_dir,
            fan_dir,
            sensors,
            hw_min,
            hw_max,
            panic_file,
        })
    }

    /// Q4 layout-change hook for `doctor`: true when fan attributes are no
    /// longer (only) directly in the applesmc platform dir — either discovery
    /// had to fall back to a hwmon subdir, or a hwmon dir under the platform
    /// has grown fan attributes (upstream conversion in flight). Either way
    /// the systemd unit's pinned `ReadWritePaths` needs revisiting.
    pub fn layout_changed(&self) -> bool {
        self.fan_dir != self.platform_dir || hwmon_dir_with_fan_attrs(&self.platform_dir).is_some()
    }
}

// Trait impl with all Appendix-A methods:
impl Smc for SysfsSmc {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError> {
        Ok(self
            .sensors
            .iter()
            .map(|(input, label)| read_sensor(input, label))
            .collect())
    }

    fn read_fan(&self) -> Result<FanState, SmcError> {
        let rpm = read_u32(&self.fan_dir.join(FAN_INPUT))?;
        let manual_path = self.fan_dir.join(FAN_MANUAL);
        let raw = read_string(&manual_path)?;
        let mode = parse_manual(&manual_path, &raw)?;
        Ok(FanState { rpm, mode })
    }

    fn hw_min_rpm(&self) -> u32 {
        self.hw_min
    }
    fn hw_max_rpm(&self) -> u32 {
        self.hw_max
    }

    /// INVARIANT: write + read-back-verify within K=3 retries; commit logical
    /// state only on verified read-back (R1).
    fn write_speed(&mut self, rpm: u32) -> Result<u32, SmcError> {
        let target = rpm.clamp(self.hw_min, self.hw_max);
        let out = self.fan_dir.join(FAN_OUTPUT);
        let input = self.fan_dir.join(FAN_INPUT);
        let mut mismatch: Option<u32> = None;
        for attempt in 1..=VERIFY_ATTEMPTS {
            // A failed write syscall surfaces immediately (R4 fail-toward-AUTO);
            // only a taken-but-unverified write is retried (R1).
            fs::write(&out, target.to_string().as_bytes()).map_err(|source| SmcError::Write {
                path: out.clone(),
                source,
            })?;
            let read_back = read_u32(&input)?;
            if read_back.abs_diff(target) <= VERIFY_TOLERANCE_RPM {
                tracing::debug!(rpm = target, "fan1_output write verified");
                return Ok(target);
            }
            tracing::warn!(
                attempt,
                attempts = VERIFY_ATTEMPTS,
                wrote = target,
                read_back,
                "fan1_output write not taken by hardware; retrying"
            );
            mismatch = Some(read_back);
        }
        Err(SmcError::VerifyFailed {
            wrote: target,
            read_back: mismatch.unwrap_or_default(),
        })
    }

    /// INVARIANT: write + read-back-verify within K=3 retries (R1). Mode
    /// read-back must match exactly (0/1 — no tolerance on a bit).
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, SmcError> {
        let manual = self.fan_dir.join(FAN_MANUAL);
        let want = u32::from(mode == FanMode::Manual);
        let mut mismatch: Option<u32> = None;
        for attempt in 1..=VERIFY_ATTEMPTS {
            fs::write(&manual, want.to_string().as_bytes()).map_err(|source| SmcError::Write {
                path: manual.clone(),
                source,
            })?;
            let raw = read_string(&manual)?;
            let got = parse_manual(&manual, &raw)?;
            if got == mode {
                tracing::debug!(?mode, "fan1_manual write verified");
                return Ok(mode);
            }
            tracing::warn!(attempt, attempts = VERIFY_ATTEMPTS, want, read_back = ?got, "fan1_manual write not taken by hardware; retrying");
            mismatch = Some(u32::from(got == FanMode::Manual));
        }
        Err(SmcError::VerifyFailed {
            wrote: want,
            read_back: mismatch.unwrap_or_default(),
        })
    }

    fn panic_fd(&self) -> Option<i32> {
        self.panic_file.as_ref().map(File::as_raw_fd)
    }
}

/// Scriptable mock backend: injectable faults, drift, latency. Test-only
/// workhorse for R10 (round-trip, drift, failed reads, mode flip).
pub struct MockSmc {
    hw_min: u32,
    hw_max: u32,
    sensors: Vec<SensorReading>,
    rpm: u32,
    mode: FanMode,
    /// When set, every `write_speed` read-back returns this rpm (write "doesn't take").
    write_stuck_rpm: Option<u32>,
    /// When set, every `set_mode` read-back returns this mode (mode flip).
    mode_read_back: Option<FanMode>,
    /// When set, `read_fan` fails with this error kind.
    fan_read_fault: Option<io::ErrorKind>,
    /// Injected per-call latency (supervisor timeout tests).
    latency: Option<Duration>,
    /// Total write attempts (write_speed + set_mode) — proves the K=3 retry count.
    write_attempts: u32,
}

impl MockSmc {
    /// Create a mock with the given hardware (min, max) rpm range.
    pub fn new(hw_min: u32, hw_max: u32) -> Self {
        Self {
            hw_min,
            hw_max,
            sensors: Vec::new(),
            rpm: hw_min,
            mode: FanMode::Auto,
            write_stuck_rpm: None,
            mode_read_back: None,
            fan_read_fault: None,
            latency: None,
            write_attempts: 0,
        }
    }

    /// Replace the injected sensor set (raw hw values; `read_sensors` applies
    /// the same outlier/parse rejection as the sysfs backend).
    pub fn set_sensors(&mut self, sensors: Vec<SensorReading>) {
        self.sensors = sensors;
    }

    /// Set the fan's current reported state (drift injection between polls).
    pub fn set_fan_state(&mut self, rpm: u32, mode: FanMode) {
        self.rpm = rpm;
        self.mode = mode;
    }

    /// Make every `write_speed` read-back return `read_back` (None = honest).
    pub fn set_write_stuck(&mut self, read_back: Option<u32>) {
        self.write_stuck_rpm = read_back;
    }

    /// Make every `set_mode` read-back return `mode` (None = honest).
    pub fn set_mode_read_back(&mut self, mode: Option<FanMode>) {
        self.mode_read_back = mode;
    }

    /// Make `read_fan` fail with the given error kind (None = honest).
    pub fn set_fan_read_fault(&mut self, kind: Option<io::ErrorKind>) {
        self.fan_read_fault = kind;
    }

    /// Inject per-call latency (None = instant).
    pub fn set_latency(&mut self, latency: Option<Duration>) {
        self.latency = latency;
    }

    /// Total write attempts so far (proves the K=3 retry bound).
    pub fn write_attempts(&self) -> u32 {
        self.write_attempts
    }

    fn nap(&self) {
        if let Some(delay) = self.latency {
            std::thread::sleep(delay);
        }
    }
}

impl Smc for MockSmc {
    fn read_sensors(&self) -> Result<Vec<SensorReading>, SmcError> {
        self.nap();
        Ok(self
            .sensors
            .iter()
            .map(|r| SensorReading {
                label: r.label.clone(),
                // Injected raw hw values may lie; re-apply the R1 window.
                milli_c: r
                    .milli_c
                    .filter(|m| (MILLI_C_MIN..=MILLI_C_MAX).contains(&m.0)),
            })
            .collect())
    }

    fn read_fan(&self) -> Result<FanState, SmcError> {
        self.nap();
        if let Some(kind) = self.fan_read_fault {
            return Err(SmcError::Read {
                path: PathBuf::from("<mock:fan1_input>"),
                source: io::Error::from(kind),
            });
        }
        Ok(FanState {
            rpm: self.rpm,
            mode: self.mode,
        })
    }

    fn hw_min_rpm(&self) -> u32 {
        self.hw_min
    }
    fn hw_max_rpm(&self) -> u32 {
        self.hw_max
    }

    /// Same K=3 read-back-verify invariant as the real backend.
    fn write_speed(&mut self, rpm: u32) -> Result<u32, SmcError> {
        let target = rpm.clamp(self.hw_min, self.hw_max);
        let mut mismatch: Option<u32> = None;
        for _attempt in 1..=VERIFY_ATTEMPTS {
            self.write_attempts += 1;
            self.nap();
            let read_back = self.write_stuck_rpm.unwrap_or(target);
            if read_back.abs_diff(target) <= VERIFY_TOLERANCE_RPM {
                // INVARIANT: logical state commits only on verified read-back.
                self.rpm = read_back;
                return Ok(target);
            }
            mismatch = Some(read_back);
        }
        Err(SmcError::VerifyFailed {
            wrote: target,
            read_back: mismatch.unwrap_or_default(),
        })
    }

    /// Same K=3 read-back-verify invariant as the real backend (exact match).
    fn set_mode(&mut self, mode: FanMode) -> Result<FanMode, SmcError> {
        let want = u32::from(mode == FanMode::Manual);
        let mut mismatch: Option<u32> = None;
        for _attempt in 1..=VERIFY_ATTEMPTS {
            self.write_attempts += 1;
            self.nap();
            let got = self.mode_read_back.unwrap_or(mode);
            if got == mode {
                self.mode = got;
                return Ok(mode);
            }
            mismatch = Some(u32::from(got == FanMode::Manual));
        }
        Err(SmcError::VerifyFailed {
            wrote: want,
            read_back: mismatch.unwrap_or_default(),
        })
    }

    fn panic_fd(&self) -> Option<i32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs")
    }

    static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

    /// Private tempdir holding a copy of a fixture's `devices/` tree
    /// (`""` = canonical) — tests mutate the copy, never the repo fixture,
    /// never the real /sys.
    struct TempFixture {
        root: PathBuf,
    }

    impl TempFixture {
        fn copy_of(variant: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "afanctl-t3-{}-{}",
                std::process::id(),
                TMP_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            copy_tree(
                &fixture_root().join(variant).join("devices"),
                &root.join("devices"),
            )
            .expect("copy fixture tree to tempdir");
            Self { root }
        }
    }

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let to = dst.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &to)?;
            } else {
                fs::copy(entry.path(), to)?;
            }
        }
        Ok(())
    }

    fn assert_verify_failed(err: SmcError, wrote: u32, read_back: u32) {
        match err {
            SmcError::VerifyFailed {
                wrote: w,
                read_back: r,
            } => assert_eq!(
                (w, r),
                (wrote, read_back),
                "VerifyFailed {{ {wrote}, {read_back} }} expected"
            ),
            other => panic!("expected VerifyFailed, got {other:?}"),
        }
    }

    #[test]
    fn discovery_walks_hwmon_and_reads_hw_range() {
        let smc = SysfsSmc::open(&fixture_root()).expect("open canonical fixture");
        assert_eq!(smc.hw_min_rpm(), 1200);
        assert_eq!(smc.hw_max_rpm(), 7200);
        // Sensors discovered by walking hwmon/ (hwmon4 here), ordered, labelled.
        let sensors = smc.read_sensors().expect("sensors");
        let got: Vec<(String, Option<i32>)> = sensors
            .into_iter()
            .map(|r| (r.label, r.milli_c.map(|m| m.0)))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Package id 0".to_owned(), Some(45_000)),
                ("Core 0".to_owned(), Some(44_000)),
                ("Core 1".to_owned(), Some(43_000)),
            ]
        );
        let fan = smc.read_fan().expect("fan");
        assert_eq!(fan.rpm, 1200);
        assert_eq!(fan.mode, FanMode::Auto);
    }

    #[test]
    fn mock_write_speed_roundtrip_and_clamp() {
        let mut smc = MockSmc::new(1200, 7200);
        assert_eq!(smc.write_speed(3000).expect("write"), 3000);
        assert_eq!(smc.read_fan().expect("fan").rpm, 3000);
        assert_eq!(smc.write_attempts(), 1);
        // Clamped to the hw range read "from hardware" at construction.
        assert_eq!(smc.write_speed(99_999).expect("clamp high"), 7200);
        assert_eq!(smc.write_speed(0).expect("clamp low"), 1200);
    }

    #[test]
    fn mock_write_not_taking_fails_verify_after_k_retries() {
        let mut smc = MockSmc::new(1200, 7200);
        smc.set_write_stuck(Some(1200)); // firmware still owns the fan
        let err = smc.write_speed(3000).unwrap_err();
        assert_verify_failed(err, 3000, 1200);
        assert_eq!(smc.write_attempts(), VERIFY_ATTEMPTS); // K=3 (PRD R1)
                                                           // INVARIANT: no logical state from unverified writes — rpm unchanged.
        assert_eq!(smc.read_fan().expect("fan").rpm, 1200);
    }

    #[test]
    fn mock_verify_accepts_within_tolerance_readback() {
        let mut smc = MockSmc::new(1200, 7200);
        smc.set_write_stuck(Some(1300)); // actual settles 100 rpm above target
        assert_eq!(smc.write_speed(1200).expect("within tolerance"), 1200);
        assert_eq!(smc.read_fan().expect("fan").rpm, 1300);
        assert_eq!(smc.write_attempts(), 1);
    }

    #[test]
    fn fixture_write_not_taking_fails_verify_after_k_retries() {
        // Static fixture: fan1_input never follows fan1_output, so a write
        // "doesn't take" exactly like stuck hardware would.
        let tmp = TempFixture::copy_of("");
        let mut smc = SysfsSmc::open(&tmp.root).expect("open");
        let err = smc.write_speed(3000).unwrap_err();
        assert_verify_failed(err, 3000, 1200);
        // The write syscall itself happened (last attempt persisted)…
        let out = fs::read_to_string(tmp.root.join("devices/platform/applesmc.768/fan1_output"))
            .expect("read output");
        assert_eq!(out.trim(), "3000");
        // …but verify never saw it on fan1_input → error after K=3 attempts.
    }

    #[test]
    fn set_mode_roundtrip_flips_fixture_file() {
        let tmp = TempFixture::copy_of("");
        let mut smc = SysfsSmc::open(&tmp.root).expect("open");
        let manual = tmp.root.join("devices/platform/applesmc.768/fan1_manual");
        assert_eq!(
            smc.set_mode(FanMode::Manual).expect("manual"),
            FanMode::Manual
        );
        assert_eq!(fs::read_to_string(&manual).expect("read").trim(), "1");
        assert_eq!(smc.read_fan().expect("fan").mode, FanMode::Manual);
        assert_eq!(smc.set_mode(FanMode::Auto).expect("auto"), FanMode::Auto);
        assert_eq!(fs::read_to_string(&manual).expect("read").trim(), "0");
        // The death-path fd stays valid across mode flips.
        assert!(smc.panic_fd().expect("panic fd") > 0);
    }

    #[test]
    fn mock_mode_roundtrip_and_stuck_mode_fails_after_k_retries() {
        let mut smc = MockSmc::new(1200, 7200);
        assert_eq!(
            smc.set_mode(FanMode::Manual).expect("manual"),
            FanMode::Manual
        );
        smc.set_mode_read_back(Some(FanMode::Auto)); // hardware flips back under us
        let err = smc.set_mode(FanMode::Manual).unwrap_err();
        assert_verify_failed(err, 1, 0);
        assert_eq!(smc.write_attempts(), 1 + VERIFY_ATTEMPTS);
        // INVARIANT: unverified mode not committed.
        assert_eq!(smc.read_fan().expect("fan").mode, FanMode::Manual);
    }

    #[test]
    fn fixture_outlier_and_failed_reads_rejected() {
        // > 120 °C outlier → failed read
        let smc = SysfsSmc::open(&fixture_root().join("outlier_hi")).expect("open");
        let rs = smc.read_sensors().expect("sensors");
        assert_eq!(rs[0].label, "Package id 0");
        assert!(rs[0].milli_c.is_none(), "125 °C must be rejected");
        assert!(rs[1].milli_c.is_some());
        // negative outlier → failed read
        let smc = SysfsSmc::open(&fixture_root().join("outlier_neg")).expect("open");
        let rs = smc.read_sensors().expect("sensors");
        assert!(rs[1].milli_c.is_none(), "-1 °C must be rejected");
        // empty (short) file → failed read
        let smc = SysfsSmc::open(&fixture_root().join("empty_temp")).expect("open");
        let rs = smc.read_sensors().expect("sensors");
        assert!(
            rs[1].milli_c.is_none(),
            "empty temp2_input must be a failed read"
        );
        // unparseable content → failed read; valid sensors still report (H1)
        let smc = SysfsSmc::open(&fixture_root().join("garbage_temp")).expect("open");
        let rs = smc.read_sensors().expect("sensors");
        assert!(
            rs[2].milli_c.is_none(),
            "garbage temp3_input must be a failed read"
        );
        assert!(rs[0].milli_c.is_some());
    }

    #[test]
    fn fixture_unreadable_sensor_is_failed_read() {
        let tmp = TempFixture::copy_of("");
        let t1 = tmp
            .root
            .join("devices/platform/coretemp.0/hwmon/hwmon4/temp1_input");
        fs::set_permissions(&t1, fs::Permissions::from_mode(0o000)).expect("chmod 000");
        let smc = SysfsSmc::open(&tmp.root).expect("open");
        let rs = smc.read_sensors().expect("sensors");
        assert!(rs[0].milli_c.is_none(), "EACCES read must degrade to None");
        assert!(rs[1].milli_c.is_some());
    }

    #[test]
    fn fixture_invalid_fan_values_are_invalid_value_errors() {
        // Garbage actual rpm → InvalidValue, never a silent default.
        let smc = SysfsSmc::open(&fixture_root().join("garbage_fan")).expect("open");
        match smc.read_fan() {
            Err(SmcError::InvalidValue { .. }) => {}
            other => panic!("expected InvalidValue, got {other:?}"),
        }
        // Empty actual rpm file (short read) → InvalidValue.
        let tmp = TempFixture::copy_of("");
        fs::write(
            tmp.root.join("devices/platform/applesmc.768/fan1_input"),
            b"",
        )
        .expect("truncate");
        let smc = SysfsSmc::open(&tmp.root).expect("open");
        match smc.read_fan() {
            Err(SmcError::InvalidValue { .. }) => {}
            other => panic!("expected InvalidValue, got {other:?}"),
        }
        // fan1_manual holding neither 0 nor 1 → InvalidValue.
        let tmp = TempFixture::copy_of("");
        fs::write(
            tmp.root.join("devices/platform/applesmc.768/fan1_manual"),
            b"2",
        )
        .expect("write");
        let smc = SysfsSmc::open(&tmp.root).expect("open");
        match smc.read_fan() {
            Err(SmcError::InvalidValue { .. }) => {}
            other => panic!("expected InvalidValue, got {other:?}"),
        }
        // Mock read fault surfaces as a Read error.
        let mut mock = MockSmc::new(1200, 7200);
        mock.set_fan_read_fault(Some(io::ErrorKind::PermissionDenied));
        assert!(matches!(mock.read_fan(), Err(SmcError::Read { .. })));
    }

    #[test]
    fn fixture_invalid_hw_range_rejected_at_open() {
        let tmp = TempFixture::copy_of("");
        fs::write(
            tmp.root.join("devices/platform/applesmc.768/fan1_min"),
            b"8000", // min > max
        )
        .expect("write");
        match SysfsSmc::open(&tmp.root) {
            Err(SmcError::InvalidValue { .. }) => {}
            Ok(_) => panic!("expected InvalidValue for hw range"),
            Err(other) => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn missing_driver_dirs_are_not_found() {
        for root in ["missing_fan", "no_coretemp"] {
            match SysfsSmc::open(&fixture_root().join(root)) {
                Err(SmcError::NotFound(_)) => {}
                Ok(_) => panic!("expected NotFound for {root}"),
                Err(other) => panic!("expected NotFound for {root}, got {other:?}"),
            }
        }
    }

    #[test]
    fn layout_change_hook_detects_hwmon_conversion() {
        let smc = SysfsSmc::open(&fixture_root()).expect("open canonical");
        assert!(
            !smc.layout_changed(),
            "classic layout is not a layout change"
        );
        // Conversion in flight: the attribute-less husk grows fan attrs…
        let tmp = TempFixture::copy_of("");
        let husk = tmp.root.join("devices/platform/applesmc.768/hwmon/hwmon3");
        fs::create_dir_all(&husk).expect("mkdir husk");
        fs::write(husk.join("fan1_input"), b"2400").expect("seed husk");
        let smc = SysfsSmc::open(&tmp.root).expect("open");
        assert!(smc.layout_changed(), "husk growing fan attrs = conversion");
        // …and after the move, discovery finds the fan in the hwmon dir (Q4).
        let smc = SysfsSmc::open(&fixture_root().join("layout_changed")).expect("open");
        assert!(smc.layout_changed());
        assert_eq!(smc.hw_min_rpm(), 1200);
        assert_eq!(smc.hw_max_rpm(), 7200);
        assert_eq!(smc.read_fan().expect("fan").rpm, 3400);
    }

    #[test]
    fn panic_fd_present_on_sysfs_absent_on_mock() {
        let tmp = TempFixture::copy_of("");
        let smc = SysfsSmc::open(&tmp.root).expect("open");
        assert!(smc.panic_fd().expect("panic fd") >= 0);
        let mock = MockSmc::new(1200, 7200);
        assert!(mock.panic_fd().is_none(), "mock has no death-path fd");
    }

    #[test]
    fn mock_rejects_outliers_like_sysfs() {
        let mut smc = MockSmc::new(1200, 7200);
        smc.set_sensors(vec![
            SensorReading {
                label: "ok".to_owned(),
                milli_c: Some(MilliC(60_000)),
            },
            SensorReading {
                label: "hot".to_owned(),
                milli_c: Some(MilliC(125_000)),
            },
            SensorReading {
                label: "dead".to_owned(),
                milli_c: None,
            },
        ]);
        let rs = smc.read_sensors().expect("sensors");
        assert!(rs[0].milli_c.is_some());
        assert!(
            rs[1].milli_c.is_none(),
            "outlier rejection applies to mock too"
        );
        assert!(rs[2].milli_c.is_none());
    }
}
