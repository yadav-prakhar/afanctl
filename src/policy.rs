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

/// One control decision. Pure output of a `Controller` step. Every decision
/// path produces exactly one variant — never silence (R2).
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
    min_rpm: u32,
    max_rpm: u32,
    high_c: i32,
    max_c: i32,
    low_c: i32,
    /// Current slew-limited output; the value we keep commanding and the base
    /// the limiter moves from. Starts at `min_rpm` (idle).
    // INVARIANT: always within the configured [min_rpm, max_rpm] curve range.
    slew_rpm: u32,
    /// Hysteresis latch: once ramping, stay ramping until `t < low` (no flap).
    ramping: bool,
    loss_streak: u32,
    overshoot_streak: u32,
    last_t_eff: Option<MilliC>,
}

impl Controller {
    /// Build a controller from fully validated config.
    pub fn new(cfg: &crate::config::ResolvedConfig) -> Self {
        let c = &cfg.config;
        Controller {
            min_rpm: c.min_rpm,
            max_rpm: c.max_rpm,
            high_c: c.high_c,
            max_c: c.max_c,
            low_c: cfg.low_c,
            slew_rpm: c.min_rpm,
            ramping: false,
            loss_streak: 0,
            overshoot_streak: 0,
            last_t_eff: None,
        }
    }

    /// Curve mode step. Interim sensor loss (streak < `SENSOR_LOSS_POLLS`):
    /// keep the current output, write nothing new. Full streak →
    /// `ReturnToAuto`. Never holds a stale speed on valid input: target is
    /// recomputed from t_eff every call (no direction gates).
    pub fn step_curve(&mut self, readings: &[SensorReading]) -> Decision {
        let t = self.absorb(readings);
        match t {
            Some(t) => {
                // INVARIANT: overshoot guard bypasses slew in every mode (R2).
                if self.overshoot_streak >= OVERSHOOT_POLLS {
                    self.slew_rpm = self.max_rpm;
                    self.ramping = true;
                    return Decision::EscalateMax;
                }
                let target = self.target_for(t);
                self.slew_rpm = slew_toward(self.slew_rpm, target, SLEW_MAX_RPM_PER_POLL);
                Decision::SetSpeed(self.slew_rpm)
            }
            None => {
                if self.loss_streak >= SENSOR_LOSS_POLLS {
                    // INVARIANT: start any later curve session with a clean
                    // baseline (R2: fail toward the firmware, then restart).
                    self.reset_streaks();
                    self.slew_rpm = self.min_rpm;
                    self.ramping = false;
                    Decision::ReturnToAuto
                } else {
                    // Interim loss: keep the current output, write nothing new.
                    Decision::Observe
                }
            }
        }
    }

    /// Hold mode step: `SetSpeed(held)` normally, `EscalateMax` when the
    /// overshoot guard fires (guard overrides hold, R2). Clamping to the
    /// hardware range is the supervisor's job at write time (it owns the
    /// `Smc`); the guard arcs here.
    pub fn step_hold(&mut self, readings: &[SensorReading], held_rpm: u32) -> Decision {
        let t = self.absorb(readings);
        match t {
            Some(_) => {
                if self.overshoot_streak >= OVERSHOOT_POLLS {
                    return Decision::EscalateMax;
                }
                // absorb() already advanced the overshoot streak for this poll.
                Decision::SetSpeed(held_rpm)
            }
            None => {
                if self.loss_streak >= SENSOR_LOSS_POLLS {
                    self.reset_streaks();
                    Decision::ReturnToAuto
                } else {
                    // Interim loss: keep the held output, write nothing new.
                    Decision::Observe
                }
            }
        }
    }

    /// Observe mode step: always `Observe` (observe never writes, R3), but
    /// tracks t_eff + sensor-loss streak (for logging/status).
    pub fn step_observe(&mut self, readings: &[SensorReading]) -> Decision {
        let t = self.absorb(readings);
        if t.is_none() && self.loss_streak >= SENSOR_LOSS_POLLS {
            self.reset_streaks();
        }
        Decision::Observe
    }

    /// Last effective temperature (max over valid sensors), for status.
    pub fn t_eff(&self) -> Option<MilliC> {
        self.last_t_eff
    }

    /// Update tracking state from this poll's readings and return t_eff
    /// (max over valid sensors), or None when every sensor failed/empty.
    fn absorb(&mut self, readings: &[SensorReading]) -> Option<MilliC> {
        // H1: act on the hottest core, not the average.
        let t = readings
            .iter()
            .filter_map(|r| r.milli_c)
            .fold(None, |best: Option<MilliC>, m| match best {
                Some(b) if b.0 >= m.0 => Some(b),
                _ => Some(m),
            });
        match t {
            Some(t) => {
                self.last_t_eff = Some(t);
                self.loss_streak = 0;
                self.overshoot_check(t);
                Some(t)
            }
            None => {
                self.loss_streak += 1;
                self.overshoot_streak = 0;
                None
            }
        }
    }

    /// Track the overshoot streak: consecutive polls with `t_eff >= max - 1`.
    fn overshoot_check(&mut self, t: MilliC) {
        // max - 1 °C, in milli-°C.
        // INVARIANT: guard threshold = max_c - 1, per PRD R2.
        if t.0 >= (self.max_c - 1) * 1000 {
            self.overshoot_streak += 1;
        } else {
            self.overshoot_streak = 0;
        }
    }

    /// Absolute target curve (R2), recomputed every call, no direction gates.
    fn target_for(&mut self, t: MilliC) -> u32 {
        if t.0 < self.low_c * 1000 {
            self.ramping = false;
            self.min_rpm
        } else if t.0 >= self.max_c * 1000 {
            self.ramping = true;
            self.max_rpm
        } else if t.0 >= self.high_c * 1000 {
            self.ramping = true;
            linear_target(t, self.high_c, self.max_c, self.min_rpm, self.max_rpm)
        } else {
            // Mid zone (low ≤ t < high): hysteresis. Not ramping → stay at
            // min_rpm. Ramping → hold the current position (no flapping
            // between min and a linear target just below `high`).
            if self.ramping {
                self.slew_rpm
            } else {
                self.min_rpm
            }
        }
    }

    fn reset_streaks(&mut self) {
        self.loss_streak = 0;
        self.overshoot_streak = 0;
    }
}

/// Slew limiter: move `from` at most `max_step` rpm toward `target` (R2).
fn slew_toward(from: u32, target: u32, max_step: u32) -> u32 {
    if target >= from {
        (from + max_step).min(target)
    } else {
        from.saturating_sub(max_step).max(target)
    }
}

/// Linear interpolation of the fan curve over [high, max] (R2). Exact at the
/// endpoints: `t == MilliC::from_c(high)` → `min_rpm`; `t == MilliC::from_c(max)`
/// → `max_rpm`.
fn linear_target(t: MilliC, high_c: i32, max_c: i32, min_rpm: u32, max_rpm: u32) -> u32 {
    // INVARIANT: exact endpoints — no rounding drift at high / max.
    let denom = (max_c - high_c) as i64 * 1000;
    let num = t.0 as i64 - high_c as i64 * 1000;
    (num * (max_rpm as i64 - min_rpm as i64) / denom) as u32 + min_rpm
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u32 = 1200;
    const MAX: u32 = 6200;
    const HIGH: i32 = 66;
    const MAXC: i32 = 86;

    fn cfg() -> crate::config::ResolvedConfig {
        crate::config::ResolvedConfig {
            config: crate::config::Config {
                high_c: HIGH,
                max_c: MAXC,
                min_rpm: MIN,
                max_rpm: MAX,
                interval_s: 1,
            },
            low_c: HIGH - 3,
        }
    }

    fn read(milli: i32) -> SensorReading {
        SensorReading {
            label: "Core 0".to_string(),
            milli_c: Some(MilliC(milli)),
        }
    }

    /// Linear endpoints exact at the boundaries (card test).
    #[test]
    fn linear_endpoints_exact() {
        assert_eq!(
            linear_target(MilliC(HIGH * 1000), HIGH, MAXC, MIN, MAX),
            MIN
        );
        assert_eq!(
            linear_target(MilliC(MAXC * 1000), HIGH, MAXC, MIN, MAX),
            MAX
        );
        // Midpoint: (86-66)/2 °C above high → (MIN+MAX)/2.
        assert_eq!(
            linear_target(MilliC((HIGH + 10) * 1000), HIGH, MAXC, MIN, MAX),
            MIN + (MAX - MIN) / 2
        );
        // Just below max: never exceeds max_rpm.
        assert!(linear_target(MilliC(MAXC * 1000 - 1), HIGH, MAXC, MIN, MAX) <= MAX);
    }

    /// MilliC conversion table (rounding both directions).
    #[test]
    fn milli_c_rounding() {
        assert_eq!(MilliC::from_c(80), MilliC(80_000));
        assert_eq!(MilliC(80_400).as_c(), 80);
        assert_eq!(MilliC(80_500).as_c(), 81);
        assert_eq!(MilliC(-1_500).as_c(), -2);
    }

    /// Interim sensor loss writes nothing new (Observe); the streak then
    /// returns curve control on valid input.
    #[test]
    fn curve_on_valid_input_after_full_loss_streak_and_valid_return() {
        let mut c = Controller::new(&cfg());
        let r = read(MilliC::from_c(80).0);
        // Converge the 80 °C plateau: f(80) = 4700 after 5 slew polls.
        assert_eq!(
            c.step_curve(std::slice::from_ref(&r)),
            Decision::SetSpeed(1950)
        );
        for expected in [2700, 3450, 4200, 4700] {
            assert_eq!(
                c.step_curve(std::slice::from_ref(&r)),
                Decision::SetSpeed(expected)
            );
        }
        let lost: [SensorReading; 1] = [SensorReading {
            label: "Core 0".to_string(),
            milli_c: None,
        }];
        assert_eq!(c.step_curve(&lost), Decision::Observe);
        assert_eq!(c.step_curve(&lost), Decision::Observe);
        assert_eq!(c.step_curve(&lost), Decision::ReturnToAuto);
        // A valid reading after the streak restarts the curve (H2: absolute
        // target, no stale hold).
        assert!(matches!(c.step_curve(&[r]), Decision::SetSpeed(1950)));
    }
}
