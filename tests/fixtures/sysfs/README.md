Fixture sysfs trees — the canonical tree at the root mirrors PRD §2.1 exactly
(verified live 2026-09-14) and is the **legacy** (kernel <= 7.2) applesmc ABI.

RULING F26: applesmc has two attribute generations and afanctl supports both,
so `modern/` (below) is a full kernel >= 7.3 tree. Every test that asserts on
an attribute name must say which generation it is exercising.

Layout (legacy / canonical):
  devices/platform/applesmc.768/
    fan1_label   = "Exhaust"       (single-fan scope)
    fan1_min     = 1200            (hardware range floor; all writes clamped here)
    fan1_max     = 7200            (hardware range ceiling)
    fan1_manual  = 0               (0 = auto / SMC firmware curve, 1 = manual)
    fan1_input   = 1200            (actual rpm — verification readback)
    fan1_output  = 1200            (write target rpm — actuator)
    fan1_safe    = (empty)         (present, semantics unknown — ignored, Q3)
    hwmon/hwmon3/                  (attribute-less husk — upstream applesmc is
                                    mid-conversion to standard hwmon attrs;
                                    doctor must detect if it gains fan attrs)

  devices/platform/coretemp.0/hwmon/hwmon4/
    temp1_input = 45000, temp1_label = "Package id 0", temp1_crit = 100000 (Tjmax)
    temp2_input = 44000, temp2_label = "Core 0",      temp2_crit = 100000
    temp3_input = 43000, temp3_label = "Core 1",      temp3_crit = 100000

Notes:
- Temps are milli-°C exactly as sysfs provides; rpm are u32.
- hwmon indices are DYNAMIC on real hardware (coretemp = hwmon4 today):
  discovery must walk coretemp.0/hwmon/hwmon*, never hardcode hwmonN (Q5).
- Fan files sit directly in the platform dir, NOT under hwmon (§2.1).
- All tests run against THIS tree via --sysfs-root. Never write real /sys.
- T3 completes this skeleton (more fixtures for fault injection as needed);
  T0 only creates the data files above.

Fault-injecting fixture variants (added by T3; each is a full independent
tree = copy of `devices/` with one fault applied — read faults too are
committed trees; mutation-needing tests copy a tree to a tempdir first):

  outlier_hi/       temp1_input = 125000          (> 120 °C outlier rejection)
  outlier_neg/      temp2_input = -1000           (< 0 °C outlier)
  empty_temp/       temp2_input = (empty)         (short/empty read)
  garbage_temp/     temp3_input = "abc"           (unparseable sensor value)
  garbage_fan/      fan1_input  = "12x4"          (InvalidValue on read_fan)
  missing_fan/      applesmc.768 lacks fan1_input (NotFound at open)
  no_coretemp/      no coretemp.0 platform dir    (NotFound at open)
  layout_changed/   fan1_* moved INTO applesmc.768/hwmon/hwmon3/ (Q4 hwmon
                    conversion happened; discovery walks and finds it,
                    fan1_input = 3400 — exercises the layout-change hook)

The modern (kernel >= 7.3) ABI tree (RULING F26; added with issues #2 + #3):

  modern/           devices/platform/applesmc.768/
                      fan1_label   = "Exhaust"     (unchanged)
                      fan1_min     = 1200          (unchanged, RW)
                      fan1_max     = 7200          (unchanged name, now RO 0444
                                                    upstream; afanctl only reads
                                                    it — tests chmod a copy to
                                                    prove that)
                      fan1_input   = 1200          (unchanged)
                      fan1_target  = 1200          (was fan1_output)
                      pwm1_enable  = 2             (was fan1_manual; 1 = manual,
                                                    2 = AUTO, 0 = -EINVAL)
                      fan1_safe    = (empty)       (retained via extra_groups)
                    and the same coretemp.0 subtree as the canonical tree.

  NOTE: there is deliberately **no `pwm1`** — applesmc declares only
  HWMON_PWM_ENABLE, so the duty attribute does not exist. Nothing may create
  one, and `modern_abi_daemon_takes_control_and_l2_restores_auto_token_2`
  asserts no `fan1_output` appears either.

  File permissions are NOT carried by git (only the exec bit is), so the
  read-only-ness of `fan1_max` is applied by `chmod` on a tempdir copy inside
  the tests that need it, exactly as the read-fault tests already do.

  Ambiguous ("both generations present") and half-converted trees are built by
  mutating a tempdir copy rather than committed, since they are error paths
  that must never be mistaken for a supported layout.
