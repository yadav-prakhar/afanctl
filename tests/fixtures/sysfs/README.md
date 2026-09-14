Fixture sysfs tree — mirrors PRD §2.1 exactly (verified live 2026-09-14).

Layout:
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
