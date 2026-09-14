# HANDOFF — afanctl (A1708 pre-T2 Intel Mac fan supervisor)

**State: build complete and review-gate CLOSED. The hardware gate is partially passed; four items remain
and they require you at the machine.** This document is the operator's handoff: what exists, how to
install and use it, what was verified and how, what is honestly still open, and what to do next.

- Repo: `/home/prakhar/Work/tries/2026-09-14-a1708-fanctl` (branch `master`)
- Authoritative documents: `PRD.md` (requirements) · `DESIGN.md` (binding contracts + conventions) ·
  `PLAN.md` (the orchestration plan, task cards, and the gate checklist) · `orchestration/LEDGER.md`
  (the flight recorder: every ruling, dispatch, merge and gate result)
- Review reports: `orchestration/REVIEW-T9.md`, `-T9b.md`, `-T9c.md`, `-T9d.md`
- Task cards as dispatched: `orchestration/instructions/` (T0–T9 + F14–F23)

## 1. What it is

A single-fan supervisor for Macs whose fan is driven by the Apple SMC through the `applesmc` driver and
which expose no `pwm*` attributes (so `fancontrol`/`pwmconfig` cannot be used). One Rust daemon + CLI,
3.5k lines of product code, seven dependencies, no async, one thread, one fan.

Its whole reason to exist is the failure policy: **every failure path ends with the SMC firmware back in
charge, loudly logged.** That is implemented as four layers:

- **L1 — per-poll verify/re-assert.** Every poll re-reads the mode and the tach. A tracking deviation is
  re-asserted but never counted; only mode drift, write errors and register-echo failures feed the
  3-strike ladder to AUTO + monitor-only. A fan whose tach never moves between polls (a dead actuator) is
  caught by a separate stall detector, and a fan that stays off target with a moving tach is warned about
  repeatedly instead of degraded.
- **L2 — death path.** At startup an `O_WRONLY` fd on `fan1_manual` is pre-opened; a panic hook and raw
  `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers perform exactly one async-signal-safe `write(fd, b"0")`,
  restoring AUTO in a single syscall. Proven on hardware (`selftest-panic`: exit 101, `fan1_manual` = 0).
- **L3 — systemd.** `Type=notify`, `WatchdogSec=15` with a ping at both ends of every poll, `Restart=always`,
  `StartLimitIntervalSec=0`, sandboxed (`ProtectSystem=strict`, `ReadWritePaths` pinned to the applesmc
  platform dir). Uncatchable deaths (SIGKILL/OOM-kill) cannot run L2, so the **startup reconcile** restores
  AUTO on the restart — proven on hardware (gate e).
- **Fallback honesty.** If a fallback's own AUTO restore fails, the daemon keeps ownership of the fan,
  retries every poll, and reports `auto_restore_pending` — it never claims a release it could not verify.

## 2. Install and use

```sh
# build + install (the package is also already built at packaging/*.pkg.tar.zst)
cd packaging && makepkg -si

# after ANY reinstall, restart and confirm the loaded build:
sudo systemctl restart afanctl
sudo afanctl doctor            # 9 checks; exits 1 on any FAIL
```

```sh
afanctl status [--json]        # temps, t_eff, mode, fan, target, config provenance, recent errors
afanctl doctor [--roundtrip] [--compare <s>]   # diagnostics; --roundtrip is its only write
afanctl once [--at-temp <C>] [--dry-run]       # one control iteration, for scripts/CI
sudo afanctl observe|curve|hold <rpm>          # write the command file; the daemon applies it
sudo afanctl selftest-panic                    # hidden; proves L2 (exits 101 by design)
```

The daemon always starts in `observe` (writes nothing; the firmware owns the fan). `curve` is opt-in via
`afanctl curve` or the unit's `ExecStart`, and `/run` is tmpfs, so a reboot always returns to observe.

Config: `/etc/afanctl/afanctl.toml`, five keys, pacman `backup=`-protected. Defaults: `high = 66`,
`max = 86` (full speed), `min_rpm = 1200`, `max_rpm = 6200`, `interval_s = 1` (cap 12 — the watchdog
coupling). Invalid config refuses to start rather than running on defaults.

## 3. The SMC contract as measured on this machine

From `drivers/hwmon/applesmc.c` and verified live here:

| sysfs | SMC key | meaning |
|---|---|---|
| `fan1_input` | `F0Ac` | actual rpm, read-only |
| `fan1_output` | `F0Tg` | **target**, read-write (the command register) |
| `fan1_manual` | bit 0 of `FS!` | 0 = firmware curve, 1 = our command |
| `fan1_min`/`fan1_max` | `F0Mn`/`F0Mx` | 1200 / 7200 rpm |

Measured dynamics (2026-09-14, used to calibrate the code and the mocks): the SMC adopts a written target
within ≤ 1 s on a ~1 s internal tick; a full swing takes ~5 s (6688 → 2664 → 1632 → … → 2001 rpm);
steady-state jitter ±20 rpm (worst sampled second-to-second delta 26 rpm). `fan1_output` mirrors the
SMC's own target while in AUTO, so a write must be preceded by a verified manual-mode write.

## 4. What was verified, and how

- **Fixture/mock suite: 162 tests**, all four §8 gates (fmt, clippy `-D warnings`, test, `--features hw`
  skipping cleanly) green on every merge. `--features hw` tests only compile and run with
  `AFANCTL_HWTEST=1` *and* real applesmc present.
- **Four adversarial review rounds** (fresh context each, read-only, mandated to falsify the previous
  fixes): T9 → 5 defects; T9b → 2 MAJOR + 7 MINOR; T9c → 1 MAJOR + 1 arithmetic defect + 5 MINOR;
  T9d → **PASS / CLOSE** (3 informational, all closed or explicitly declined). Every finding was
  re-verified in source by the orchestrator before being ticketed; none was taken on the reviewer's word.
- **The hardware gate found what no fixture could** — three defects that invalidated the tool in
  different ways: (1) no startup reconcile, so a SIGKILL left the fan in manual forever (F14);
  (2) write verification compared the *command* to the *tachometer*, and then counted the fan's normal
  deceleration as a write failure, so curve mode disabled itself during every ramp (F16); (3) the SMC
  updates the target register on a ~1 s tick, so the fixed echo check still failed on hardware until the
  verification was given a settle window (F19). Mocks and fixtures now model all three behaviours
  (`set_tach_lag`, `set_tach_frozen`, `set_write_stuck`, `set_echo_latency`).
- **Gate results so far**: (a) `doctor` PASS, (b) `doctor --roundtrip` PASS (manual 2 s → AUTO restored),
  (c) `selftest-panic` PASS (`fan1_manual` = 0 after), (d) observe soak PASS — 1 h 10 min, 1.966 s CPU
  over 4211 s wall (0.05 %, budget < 0.1 %), 2.4 MB peak RSS (budget < 5 MB), zero errors,
  (e) `SIGKILL` in curve mode PASS — restarted, journal shows the reconcile restoring AUTO, `fan1_manual`
  read 0. **`curve` genuinely controls the fan on hardware** (`manual = 1` within 3 s, target and rpm
  tracking).

## 5. Still open (honest list)

**Hardware steps that need you** (the only real-sysfs session; the orchestrator never ran them):

- (f) `sudo afanctl curve`, soak ~1 h watching `afanctl status`; then `sudo afanctl observe` and
  `sudo afanctl doctor --compare 600` for the empirical SMC-vs-ours table.
- (g) `pkexec afanctl hold 3000` from your user shell (polkit rule), confirm the fan reaches ~3000 and
  `doctor`/`status` report hold; `afanctl once --at-temp 86 --dry-run` for the overshoot-guard simulation;
  then `sudo afanctl observe`.
- `sudo systemctl enable afanctl` + a reboot test (expected: boots in observe, fan on the SMC curve).
- Before publishing anywhere: re-check the name on AUR/crates.io (PRD Q1 — it was free on 2026-09-14, and
  `fanctl`/`macfanctl`/`smctl` are taken).

**Known deviations and residuals (none hidden):**

- **LOC is far over PRD §7's per-file sketch**: ~4,158 product lines (non-test, including the §8-mandated
  module docs, invariant comments and safety arguments) against the 1,420-line sketch (+193 %). Every
  increase was stop-and-reported at the time (ledger `N-T1-1`, `N-T19-1`, `N-F20-1`, `N-F19-1`). The
  budgets were a sketch, not a measurement; the feature set was fixed by the PRD, and the comment density
  is mandated by §8. PRD §9.5's "within ±20 %" criterion is therefore **not met** — amend §7/§9.5 or accept.
- **A plugin must read the additive state fields, not just `mode`**: `monitor_only` (latched degradation)
  and `auto_restore_pending` (a release that has not verified) are the truth-bearers; `mode` alone can read
  `curve`/`observe` while nothing is being written or the fan is still in manual. Documented in README.
- **`doctor`'s stale-binary check** detects "unit older than the installed binary", not "package older than
  repo HEAD" — the reinstall-then-restart discipline covers that gap.
- **Fixture-shaped assumptions that remain** (listed in `REVIEW-T9d.md`, item 8 family): echo adoption
  beyond ~3 s on a busy SMC (would defer the fan to the firmware — fail-safe), transient sysfs read errors
  during verification, torn two-file snapshots of mode+rpm (self-correcting next poll). The hardware gate
  cannot cover all of these either; they are documented rather than papered over.
- **Out of scope by design** (PRD §12): no auto-intervene mode, no multi-fan support, no true fan-off
  (the hardware floor is `fan1_min` = 1200 rpm), no direct SMC key writes, no persisted mode across
  reboots, no GUI — the planned `omafan` plugin consumes this CLI's surface instead.

## 6. Where to look for anything

| Question | Read |
|---|---|
| What is required, and what changed on hardware evidence | `PRD.md` (§6 requirements, §9 acceptance) |
| The frozen contracts, constants and schemas | `DESIGN.md` (Appendix A signatures, B schemas) |
| How the build was run, and the gate checklist | `PLAN.md` (§3 orchestrator runbook, Appendix P5) |
| Every ruling, dispatch, merge, bounce and gate result | `orchestration/LEDGER.md` |
| What each adversarial round found | `orchestration/REVIEW-T9*.md` |
| What each subagent was told | `orchestration/instructions/*.md` |
