//! Policy trace tests (PRD R10, owned by T2): the seven trace families from
//! the mbpfan debate, written as data tables — input readings → expected
//! decision sequences. Config below: high=66, max=86, low=63, rpm 1200..6200
//! (the documented defaults, PRD R6 / Appendix B status example).
//!
//! Curve math for this config: f(t) = 1200 + (t-66) * 250 rpm (span 20 °C,
//! 5000 rpm). Slew = 750 rpm/poll. Overshoot threshold = max − 1 °C = 85 °C.

use afanctl::config::{Config, ResolvedConfig};
use afanctl::policy::{
    Controller, Decision, MilliC, OVERSHOOT_POLLS, SENSOR_LOSS_POLLS, SLEW_MAX_RPM_PER_POLL,
};
use afanctl::smc::SensorReading;

const MIN: u32 = 1200;
const MAX: u32 = 6200;

fn cfg() -> ResolvedConfig {
    ResolvedConfig {
        config: Config {
            high_c: 66,
            max_c: 86,
            min_rpm: MIN,
            max_rpm: MAX,
            interval_s: 1,
        },
        low_c: 63,
    }
}

fn hot(milli: i32) -> SensorReading {
    SensorReading {
        label: "Core 0".to_string(),
        milli_c: Some(MilliC(milli)),
    }
}

fn cool(milli: i32) -> SensorReading {
    SensorReading {
        label: "Core 1".to_string(),
        milli_c: Some(MilliC(milli)),
    }
}

fn lost() -> SensorReading {
    SensorReading {
        label: "Core 0".to_string(),
        milli_c: None,
    }
}

/// Fan-curve target at `t` °C for this config (documented above).
fn f(t: i32) -> u32 {
    (MIN as i32 + (t - 66) * 250) as u32
}

/// Replay a whole-poll trace: each entry is the combined sensor vector for one
/// poll; collect the decisions.
fn replay(c: &mut Controller, polls: &[Vec<SensorReading>]) -> Vec<Decision> {
    polls.iter().map(|p| c.step_curve(p)).collect()
}

/// Verify a set of SetSpeed decisions is slew-bounded stepwise.
fn slew_bounded(rpms: &[u32]) {
    for w in rpms.windows(2) {
        assert!(
            w[1].abs_diff(w[0]) <= SLEW_MAX_RPM_PER_POLL,
            "slew violated at {w:?}"
        );
    }
}

/// Family 1 (H2 dead): steady 80 °C plateau × 50 polls converges exactly to
/// f(80) = 4700, monotonic and slew-bounded — no direction gates.
#[test]
fn trace_1_plateau_80c_converges_to_f80() {
    let mut c = Controller::new(&cfg());
    let polls: Vec<Vec<SensorReading>> = (0..50).map(|_| vec![hot(80_000)]).collect();
    let decisions = replay(&mut c, &polls);

    let rpms: Vec<u32> = decisions
        .iter()
        .map(|d| match d {
            Decision::SetSpeed(r) => *r,
            other => panic!("unexpected {other:?} during plateau"),
        })
        .collect();

    // 1200 → 4700 takes ceil(3500/750) = 5 polls; then it must hold.
    assert_eq!(rpms[..6], [1950, 2700, 3450, 4200, 4700, 4700]);
    for &r in &rpms[5..] {
        assert_eq!(r, 4700, "must stay exactly converged at f(80)");
    }
    slew_bounded(&rpms);
    assert_eq!(c.t_eff(), Some(MilliC(80_000)));
}

/// Family 2 (H1 dead): repeated {92, 60, 88} °C acts on 92 — the hottest
/// core, not the 80 °C average. Two slew polls, then the guard at ≥ 85 °C.
#[test]
fn trace_2_hot_core_trio_acts_on_hottest() {
    let mut c = Controller::new(&cfg());
    let polls: Vec<Vec<SensorReading>> = (0..OVERSHOOT_POLLS)
        .map(|_| vec![hot(92_000), cool(60_000), hot(88_000)])
        .collect();
    let ds = replay(&mut c, &polls);
    assert_eq!(
        ds,
        vec![
            Decision::SetSpeed(1950),
            Decision::SetSpeed(2700),
            Decision::EscalateMax,
        ]
    );
    assert_eq!(c.t_eff(), Some(MilliC(92_000)));
}

/// Family 3: descending approach converges *down* with no ratchet. Ramp up at
/// 84 °C, then walk down through the band to below low and reach min_rpm.
#[test]
fn trace_3_descending_approach_no_ratchet() {
    let mut c = Controller::new(&cfg());
    // Ramp to f(84)=5700: 1950,2700,3450,4200,4950,5700,5700.
    let up: Vec<Vec<SensorReading>> = (0..7).map(|_| vec![hot(84_000)]).collect();
    let ds_up = replay(&mut c, &up);
    let up_rpms: Vec<u32> = ds_up
        .iter()
        .map(|d| match d {
            Decision::SetSpeed(r) => *r,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(up_rpms, [1950, 2700, 3450, 4200, 4950, 5700, 5700]);

    // Descend: 80, 78, 76, 74, 72, 70, 68 then below low: 62, 62. At each
    // poll the recomputed target must be met-or-slewed-toward; output may
    // rise only if the target is above (never sticks above its target).
    let temps = [80, 78, 76, 74, 72, 70, 68, 62, 62];
    let mut prev: u32 = 5700;
    for &t in &temps {
        match c.step_curve(&[hot(t * 1000)]) {
            Decision::SetSpeed(rpm_now) => {
                let target = if t < 63 { MIN } else { f(t) };
                if target <= prev {
                    assert!(
                        rpm_now <= prev && rpm_now >= target,
                        "at t={t}: {rpm_now} not descending to {target} from {prev}"
                    );
                } else {
                    assert!(rpm_now >= prev);
                }
                prev = rpm_now;
            }
            other => panic!("unexpected {other:?} at t={t}"),
        }
    }
    // Below low the latch releases; run to convergence and assert min_rpm.
    let final_polls: Vec<Vec<SensorReading>> = (0..20).map(|_| vec![hot(62_000)]).collect();
    for d in replay(&mut c, &final_polls) {
        assert_eq!(d, Decision::SetSpeed(MIN), "must converge to min_rpm");
    }
}

/// Family 4: hysteresis no-flap sweep. Ramping at 84 °C, drop into the mid
/// zone (63–66 °C) → stays ramping (holds its position); below low → min_rpm
/// and the latch clears; mid zone while not ramping → min_rpm (no spurious
/// ramp-up).
#[test]
fn trace_4_hysteresis_no_flap() {
    let mut c = Controller::new(&cfg());
    // Ramp up fully at 84 °C → 5700, latch set.
    for _ in 0..8 {
        c.step_curve(&[hot(84_000)]);
    }
    assert_eq!(c.step_curve(&[hot(65_000)]), Decision::SetSpeed(5700));
    assert_eq!(c.step_curve(&[hot(64_000)]), Decision::SetSpeed(5700));
    assert_eq!(c.step_curve(&[hot(63_000)]), Decision::SetSpeed(5700));
    // Below low: latch clears, and the slew limiter walks down toward
    // min_rpm (5700 → 4950 → … → 1200: it descends, no ratchet, no jump).
    assert_eq!(c.step_curve(&[hot(62_000)]), Decision::SetSpeed(4950));
    // Mid zone while NOT ramping → target stays min; the walk-down continues.
    assert_eq!(c.step_curve(&[hot(65_000)]), Decision::SetSpeed(4200));
    for expected in [3450, 2700, 1950, 1200] {
        assert_eq!(c.step_curve(&[hot(65_000)]), Decision::SetSpeed(expected));
    }
    // And it stays at min_rpm (no spurious ramp-up in the mid zone).
    assert_eq!(c.step_curve(&[hot(65_000)]), Decision::SetSpeed(MIN));

    // Fresh controller, mid zone without prior ramping → min_rpm.
    let mut c2 = Controller::new(&cfg());
    assert_eq!(c2.step_curve(&[hot(65_000)]), Decision::SetSpeed(MIN));
    // Then climbing into the band starts ramping from there.
    assert_eq!(c2.step_curve(&[hot(70_000)]), Decision::SetSpeed(1950));
}

/// Family 5: sensor-loss streak → `ReturnToAuto` after exactly
/// SENSOR_LOSS_POLLS consecutive all-failing polls; interim loss writes
/// nothing new (Observe).
#[test]
fn trace_5_sensor_loss_streak_returns_to_auto() {
    let mut c = Controller::new(&cfg());
    // Warm up with a valid poll first.
    let _ = c.step_curve(&[hot(75_000)]);
    let mut ds = vec![c.step_curve(&[lost()])];
    ds.push(c.step_curve(&[lost()]));
    ds.push(c.step_curve(&[lost()]));
    assert_eq!(
        ds,
        vec![Decision::Observe, Decision::Observe, Decision::ReturnToAuto]
    );
    // After auto return a valid poll resumes the curve from min_rpm.
    assert!(matches!(
        c.step_curve(&[hot(80_000)]),
        Decision::SetSpeed(1950)
    ));
}

/// Family 6: overshoot guard — t_eff ≥ 85 °C for OVERSHOOT_POLLS consecutive
/// polls escalates to max bypassing slew (guard fires while the slew walk
/// would still need many more polls); at 84.999 °C the guard never fires.
#[test]
fn trace_6_overshoot_guard() {
    let mut c = Controller::new(&cfg());
    let polls: Vec<Vec<SensorReading>> = (0..OVERSHOOT_POLLS).map(|_| vec![hot(85_000)]).collect();
    let ds = replay(&mut c, &polls);
    assert_eq!(
        &ds[..2],
        &[Decision::SetSpeed(1950), Decision::SetSpeed(2700)]
    );
    assert_eq!(
        ds[2],
        Decision::EscalateMax,
        "guard fires on the 3rd poll, bypassing slew"
    );

    // Just under the threshold never escalates (within a generous window).
    let mut c2 = Controller::new(&cfg());
    let under: Vec<Vec<SensorReading>> = (0..=2 * OVERSHOOT_POLLS)
        .map(|_| vec![hot(85_000 - 1)])
        .collect();
    for d in replay(&mut c2, &under) {
        assert_ne!(d, Decision::EscalateMax, "84.999 °C must not escalate");
    }
}

/// Family 7: hold with hot core escalates; hold with cool core commands the
/// held rpm; hold sensor-loss also funges toward AUTO.
#[test]
fn trace_7_hold_with_hot_core_escalates() {
    let held: u32 = 2000;

    // Repeated hot core: two SetSpeed(held), then EscalateMax on poll 3.
    let mut c = Controller::new(&cfg());
    assert_eq!(c.step_hold(&[hot(90_000)], held), Decision::SetSpeed(held));
    assert_eq!(c.step_hold(&[hot(90_000)], held), Decision::SetSpeed(held));
    assert_eq!(c.step_hold(&[hot(90_000)], held), Decision::EscalateMax);

    // {92,60,88} × 4 acts on 92 → escalates by poll 3 and stays escalated
    // while poll 3+ sees the hot core.
    let mut c2 = Controller::new(&cfg());
    for poll in 0..4 {
        let d = c2.step_hold(&[hot(92_000), cool(60_000), hot(88_000)], held);
        if poll < 2 {
            assert_eq!(d, Decision::SetSpeed(held));
        } else {
            assert_eq!(d, Decision::EscalateMax);
        }
    }

    // Cool core → plain hold, never escalates.
    let mut c3 = Controller::new(&cfg());
    for _ in 0..2 * OVERSHOOT_POLLS {
        assert_eq!(c3.step_hold(&[hot(50_000)], held), Decision::SetSpeed(held));
    }

    // Sensor loss in hold: interim Observe, full streak ReturnToAuto.
    let mut c4 = Controller::new(&cfg());
    assert_eq!(c4.step_hold(&[lost()], held), Decision::Observe);
    assert_eq!(c4.step_hold(&[lost()], held), Decision::Observe);
    assert_eq!(
        c4.step_hold(&[lost()], held),
        Decision::ReturnToAuto,
        "after {SENSOR_LOSS_POLLS} failed polls hold must return to AUTO"
    );
}
