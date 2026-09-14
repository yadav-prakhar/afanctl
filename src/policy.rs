//! Pure control policy (PRD R2): the `Controller` state machine + safety
//! constants. No I/O, no clock, no threads — fully table-testable.
//!
//! Invariants: effective temp = max over valid sensors (kills H1); target
//! recomputed every poll with no direction gates (kills H2); slew-limited
//! transitions; overshoot guard applies in every mode including hold.

use crate::smc::SensorReading;

/// Temperatures are milli-°C, exactly as sysfs provides them (PRD §7 units).
/// Never pass bare `i32` temps across module boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilliC(pub i32);

impl MilliC {
    /// Construct from whole °C.
    pub fn from_c(c: i32) -> Self {
        MilliC(c.saturating_mul(1000))
    }

    /// Whole °C, rounded to nearest (half away from zero for negatives).
    pub fn as_c(&self) -> i32 {
        if self.0 >= 0 {
            (self.0 + 500) / 1000
        } else {
            (self.0 - 500) / 1000
        }
    }
}

/// Slew limit: fan moves at most this many rpm per poll toward target (R2).
pub const SLEW_MAX_RPM_PER_POLL: u32 = 750;
/// Consecutive all-sensors-failing polls before `ReturnToAuto` (R2).
pub const SENSOR_LOSS_POLLS: u32 = 3;
/// Consecutive polls with `t_eff >= max - 1` before bypassing slew to max (R2).
pub const OVERSHOOT_POLLS: u32 = 3;
/// L1 drift tolerance: actual rpm deviating more than this from last written
/// triggers re-assert (R4). Consumed by supervisor, declared here per PRD §7.
pub const VERIFY_TOLERANCE_RPM: u32 = 150;
/// Failed L1 re-assertions before fallback: restore AUTO + monitor-only (R4).
pub const WRITE_FAIL_FALLBACK: u32 = 3;

/// One control decision. Pure output of a `Controller` step.
#[derive(Debug, PartialEq)]
pub enum Decision {
    Observe,
    SetSpeed(u32),
    EscalateMax,
    ReturnToAuto,
}

/// Pure controller state machine: curve hysteresis state, slew state, streak
/// counters. Constructed per resolved config; stepped once per poll.
pub struct Controller {
    // curve hysteresis state, slew state, streak counters (owned by T2)
    _private: (),
}

impl Controller {
    /// Build a controller from fully validated config.
    pub fn new(cfg: &crate::config::ResolvedConfig) -> Self {
        let _ = cfg;
        unimplemented!("owned by T2")
    }

    /// Curve mode step. Interim sensor loss (streak < `SENSOR_LOSS_POLLS`):
    /// keep the current output, write nothing new. Full streak →
    /// `ReturnToAuto`. Never holds a stale speed on valid input: target is
    /// recomputed from t_eff every call (no direction gates).
    pub fn step_curve(&mut self, readings: &[SensorReading]) -> Decision {
        let _ = readings;
        unimplemented!("owned by T2")
    }

    /// Hold mode step: `SetSpeed(held)` normally, `EscalateMax` when the
    /// overshoot guard fires.
    pub fn step_hold(&mut self, readings: &[SensorReading], held_rpm: u32) -> Decision {
        let _ = (readings, held_rpm);
        unimplemented!("owned by T2")
    }

    /// Observe mode step: always `Observe`, but tracks t_eff + sensor-loss
    /// streak (for logging/status).
    pub fn step_observe(&mut self, readings: &[SensorReading]) -> Decision {
        let _ = readings;
        unimplemented!("owned by T2")
    }

    /// Last effective temperature (max over valid sensors), for status.
    pub fn t_eff(&self) -> Option<MilliC> {
        unimplemented!("owned by T2")
    }
}
