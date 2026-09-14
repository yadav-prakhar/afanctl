# afanctl

A small, safety-first fan supervisor for the pre-T2 Intel Mac (MacBook Pro
A1708 / `applesmc`). It reads `coretemp` sensors, drives the single fan through
`applesmc`'s `fan1_manual` / `fan1_output`, and — when anything goes wrong —
gets out of the way and lets the SMC firmware run the fan again.

> **Scope:** one fan, one machine class. All sysfs path knowledge lives in one
> module; the control policy is pure and table-tested. See `DESIGN.md` for the
> binding contracts and `PRD.md` for the full requirements.

## Safety model

afanctl is built around the principle *fail toward the firmware*:

- **L1 — per-poll verify/re-assert.** Every poll re-reads `fan1_manual` and
  `fan1_input`. A tracking deviation (the fan more than 150 rpm from the last
  verified write) is re-asserted and **never counted** — a fan still moving
  toward its command is not a broken fan. Only mode-drift re-assert failures,
  write-syscall errors and echo (write-verification) failures feed the
  3-strike ladder to AUTO + monitor-only. A fan whose tach never moves
  **between polls** is caught by a separate stall detector, keyed on **tach
  movement between polls** — a changing command (curve slew, mid-band
  oscillation) never resets the window — and degrades directly to AUTO +
  monitor-only after 10 motionless, off-target polls. A fan hovering just
  outside the tolerance band is made visible by a warning after 30 off-target
  polls that **repeats every 30 polls for as long as the excursion lasts**
  (no degradation). Deliberate: the stall detector keys on *any* tach
  movement, so a fan that jitters above the movement epsilon while parked
  off target is reported — repeatedly — not degraded; a per-poll criterion
  cannot separate "jittering but never converging" from "healthily chasing
  without converging yet".
- **L2 — death path.** At startup the daemon pre-opens an `O_WRONLY` fd on
  `fan1_manual`. A panic hook plus raw `SIGSEGV`/`SIGABRT`/`SIGTERM`/`SIGINT`
  handlers perform exactly one async-signal-safe `write(fd, b"0")` — restoring
  AUTO in a single syscall. The hidden `selftest-panic` verb proves it.
- **L3 — systemd-first.** `Type=notify` + `WatchdogSec=15` (a hang gets
  SIGABRT → L2), `Restart=always`, `RestartSec=1`, `StartLimitIntervalSec=0`
  (crash-loops restart forever rather than dying in a manual/fans-off state),
  plus sandboxing (`ProtectSystem=strict`, `ReadWritePaths` pinned to the
  applesmc platform dir, `ProtectHome`, `PrivateTmp`, `NoNewPrivileges`).
- **Sensor loss → AUTO.** Three consecutive polls with no valid temperature and
  the supervisor returns the fan to the firmware.
- **Startup reconcile is unconditional.** At (re)start the daemon restores AUTO
  out of *any* Manual owner it finds, including a live foreign program such as
  mbpfan — deliberate (RULING F14): one fan supervisor owns the fan; the
  firmware is always the fallback.
- **Failed AUTO restore is never silent.** If a fallback's own AUTO restore
  fails — or an `observe` command's own release of a Manual fan fails — the
  daemon stays in charge and re-attempts `set_mode(Auto)` every poll (log
  rate-limited to once every 10) until a verified read-back confirms AUTO;
  the pending state is exposed as `auto_restore_pending` in `state.json` and
  `status --json`'s `daemon` object, and truthfully never reports success
  before that.
- **Invalid config → refuse to start.** A present-but-broken config never
  silently falls back to defaults; every error names the key and the fix.

Ordinary development and CI **never write to the real `/sys`** (see
[Testing](#testing)).

## Install

Packaging lives in `packaging/` (`PKGBUILD`, `afanctl.service`, the polkit rule,
and `afanctl.toml.default`).

```sh
# build the package from a checkout (does not install)
cd packaging && makepkg

# build + install (requires root; part of the supervised hardware gate)
cd packaging && makepkg -si
```

The unit is installed **disabled** and starts in `observe` mode, so nothing
changes until you opt in:

```sh
sudo systemctl enable --now afanctl
systemctl status afanctl
```

Files installed:

| Path | Purpose |
|---|---|
| `/usr/bin/afanctl` | the binary |
| `/usr/lib/systemd/system/afanctl.service` | the unit (Appendix D) |
| `/etc/afanctl/afanctl.toml` | config (pacman `backup=`-protected) |
| `/usr/share/afanctl/afanctl.toml.default` | pristine default copy |
| `/usr/share/polkit-1/rules.d/49-afanctl.rules` | optional passwordless pkexec rule |

## Command line

`afanctl <verb> [options]`. Exit codes: **0** success, **1** runtime failure,
**2** usage/CLI error (a bad flag never exits 0). The hidden `selftest-panic`
probe additionally exits **101**: the deliberate panic *is* the probe, and
101 is the deterministic Rust panic exit code (F6: probe vs. failure codes
are documented apart on purpose).

| Verb | Behavior |
|---|---|
| `daemon [--mode observe\|curve]` | run the supervisor loop (systemd `Type=notify`; default `observe`) |
| `status [--json]` | per-sensor temps, `t_eff`, mode (marked `(monitor-only)` when degraded), fan actual/target/min/max, manual?, config provenance, recent errors |
| `doctor [--json] [--roundtrip] [--compare <s>]` | diagnostics; exit 1 if any check FAILs |
| `once [--at-temp <C>] [--dry-run] [--json]` | exactly one control iteration, print the decision (scripts/CI) |
| `observe` / `curve` | write the command file; the daemon applies it |
| `hold <rpm>` | write a hold command; rejected below `fan1_min`, clamped to `fan1_max` |
| `selftest-panic` | hidden: deliberate panic to prove L2 (exits 101 by design) |

Globals:

- `--config <path>` — config TOML (default `/etc/afanctl/afanctl.toml`).
- `--sysfs-root <dir>` — sysfs root (default `/sys`). Intended for tests and
  fixtures; all sysfs access is redirected through it.
- `--version`, `-h` — version / usage.

Environment:

- `AFANCTL_RUNTIME_DIR` — directory holding `cmd.json` / `state.json`
  (default `/run/afanctl`). It exists so tests can redirect the runtime files;
  you normally do not set it.

`once --at-temp <C>` simulates the sensor at `<C>` through an in-process mock,
so it never opens sysfs even when `--sysfs-root` is set. `once --dry-run`
computes the decision without writing anything (no smc write, no state file).

## Configuration

`/etc/afanctl/afanctl.toml` is typed TOML with **exactly five** honoured keys.
Unknown keys produce a warning (not an error) and are ignored.

```toml
[thresholds]          # °C, integers
high = 66             # ramp starts here; low is derived: high - 3
max  = 86             # full speed from here; guard: max <= 95 (Tjmax 100 - 5)

[curve]
min_rpm = 1200        # clamped to >= fan1_min at load
max_rpm = 6200        # clamped to <= fan1_max at load

[poll]
interval_s = 1        # 1..=12; must stay well below the unit's WatchdogSec=15
                      # (two watchdog pings per poll — start and end of each
                      # one; starving it crash-loops)
```

Semantics:

- A **missing file** falls back to the built-in defaults above.
- A **present file** must contain all five keys. A missing key is **refused**
  (not defaulted) with the key name and the fix — a typo can never silently
  select a different curve. The daemon exits nonzero.
- Validation rejects `high`/`max` outside the 0..=95 °C band (a fan
  controller's thresholds that can't reproduce a real temperature are a
  config error), `high >= max`, `min_rpm >= max_rpm`, `interval_s < 1` or
  `interval_s > 12` (the watchdog pings ride the poll — two per poll, at its
  start and end — and the F19 settle windows block up to ≈2.7 s inside a
  failing write; a poll period without ≥3 s of watchdog headroom
  crash-loops the daemon), and non-integer values, naming the offending key
  and the fix. The watchdog bound is the compiled-in constant
  `config::MAX_INTERVAL_S`, mirroring `packaging/afanctl.service`.
- Safety tunables (verify tolerance, sensor-loss polls, overshoot polls, slew
  rate, write-fail fallback, retry count) are compiled-in constants, not config.

## Diagnostics: `doctor`

Read-only by default. `--roundtrip` is the only write doctor ever performs: a
2-second manual-mode write followed by an AUTO restore and verification.
`--json` renders the same fields as the human output.

Checks: applesmc + coretemp present; fan files present and writable by root;
sensor plausibility against Tjmax; `fan1_min`/`fan1_max` readback; config
validation; systemd unit health (notify/watchdog/start-limit); applesmc
layout-change detection (the hwmon conversion in flight → "update the unit");
and that the L2 death-path fd is armed. Each line is `PASS|FAIL|WARN — <check>
— <detail>`; exit 1 if any check FAILs.

**Root required (F11).** The write-mode checks (`fan1_manual` writable, L2 fd
armed) open the manual file `O_WRONLY` and therefore require root. Run as
non-root those checks **FAIL by design** and doctor exits 1 — an unwritable
manual file leaves L2 unarmed, which is exactly a real problem, not a false
alarm. Run the diagnostic pass under `sudo`/`pkexec` (the supervised gate
does), or expect the FAIL lines plus exit 1 on an otherwise healthy machine.

### Interpreting `--compare <seconds>`

`doctor --compare N` samples `t_eff` and the SMC's own rpm for `N` seconds in
observe mode, then replays the same trace through afanctl's curve and prints a
paired table plus divergence stats (mean/max delta, samples where afanctl is
quieter or louder) and a one-line verdict. Use it to decide, with data, whether
`curve` mode earns its keep on your machine. It is a report, not a control
action.

## Plugin surface (omafan presets)

The command file `/run/afanctl/cmd.json` (`schema: afanctl.cmd.v1`) is the
plugin-facing control channel; `status --json` is its render feed. The daemon
re-reads `cmd.json` every poll, validates it, and applies it through the same
write-verify path (an unknown mode or out-of-range rpm is logged and ignored).
A **freshness gate** applies: a re-issued command whose bytes are byte-identical
to the last *applied* one is ignored (deliberate — freshness beats re-asserting
the same write). To re-arm after monitor-only degradation, re-write the command
with a changed payload or restart the daemon (F10; the behavior itself is
deliberate and lock-tested, this note documents it).

`state.json` and `status --json`'s `daemon` object both carry a boolean
`monitor_only` (additive fields; the `v1` schema ids are unchanged). It is the
degraded latch: `true` means the daemon is **observing only** — no fan writes —
after repeated verified-write failures, or a startup where AUTO could not be
restored. The commanded `mode` is kept as-is, so `mode: "curve"` with
`monitor_only: true` means "curve was requested, but nothing is being written",
and `recent_errors` names the cause. Human `status` renders this as
`mode: curve (monitor-only)`. A plugin must render the latch, not the mode
alone.

`state.json` also carries `polls` — the number of completed polls since the
daemon started, one per poll (additive field; the `v1` schema ids are
unchanged). `status --json`'s `daemon.uptime_s` is exactly
`polls × poll.interval_s`. This is deliberately separate from the L3 audit
field `watchdog_pings`, which advances **twice** per poll (a ping at the start
and the end, RULING F21 R2) and is never used for uptime. A `state.json`
written by a daemon older than this build has no `polls`, so `uptime_s`
transiently falls back to the old `watchdog_pings × interval_s` estimate
(correct for that build, where there was one ping per poll); that fallback is
the compatibility path only.

| Preset | Command |
|---|---|
| auto | `afanctl observe` |
| low | `afanctl hold 1200` (`fan1_min`) |
| med | `afanctl hold 4000` |
| high | `afanctl hold 5800` |
| full | `afanctl hold 7200` (`fan1_max`) |
| off | `afanctl hold 1200` (hardware floor; true off is impossible via sysfs) |
| slider | `afanctl hold <rpm>` |

The optional polkit rule grants passwordless `pkexec` for exactly
`status [--json]`, `observe`, `curve`, and `hold <integer-rpm>` to members of
group `wheel` in a local, active session. It never grants `daemon`,
`doctor --roundtrip`, or any other verb/flag.

## Testing

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   # skips cleanly without real applesmc
```

Integration tests spawn the real binary against a **tempdir copy** of
`tests/fixtures/sysfs/` via `--sysfs-root`, with `AFANCTL_RUNTIME_DIR` pointed
at a tempdir; the repo fixture is never mutated. The `hw` feature is the only
path that may touch real `/sys`, and only with `AFANCTL_HWTEST=1` **and**
applesmc present — i.e. the supervised hardware gate.

## Complexity

The shipped product is ~3.5k non-test LOC against R11's ~1.4k aspiration
(F13-doc). The delta is mandated surface, not creep: a `MockSmc` with the full
fault-injection face the PRD's defect-class map demands (drift, write-not-
taking, sensor outliers, mode flips), a `doctor` with per-check FAIL/WARN
semantics plus the `--compare` divergence report, and every error carrying
key + reason + fix as R6 requires. Each file's overage is ledgered in
QUESTIONS.md/DEVIATIONS.md with its driver named. Slimming passes are deferred
to after the supervised hardware gate; none is appropriate while the gate
defects (T9 F1–F13) are still landing.

## License

MIT.
