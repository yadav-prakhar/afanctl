# Safety model

`afanctl`'s differentiator is not fan control — it is what happens when the
supervisor stops running. This page states the model precisely, **per
backend**, because as of RULING F27 it is no longer one universal mechanism.

The short version: *fail toward the firmware.* Every failure path ends with the
firmware owning the fan again, or with afanctl refusing to take it.

## Supported kernels, per ABI generation

applesmc exposes two different sysfs attribute sets. afanctl supports both in
one binary and binds one at startup by **probing for the files** — never by
parsing a kernel version, because distributions backport and the attribute set
is the only truth (RULING F26).

| Generation | Kernel | rpm setpoint | Mode attribute | Manual | **AUTO** | Notes |
|---|---|---|---|---|---|---|
| `legacy` | **<= 7.2** | `fan1_output` (RW) | `fan1_manual` (RW) | `1` | **`0`** | `fan1_max` writable (afanctl only reads it) |
| `modern` | **>= 7.3** | `fan1_target` (RW) | `pwm1_enable` (RW) | `1` | **`2`** | `fan1_max` read-only; **no `pwm1`**; `0` is rejected with `-EINVAL` |

Kernel commit
[`94f5081d`](https://github.com/torvalds/linux/commit/94f5081d359265985587b5c96797ca919253f480)
("hwmon: (applesmc) Convert to `hwmon_device_register_with_info`", released in
7.3) performed the rename. There is **no back-compat aliasing**: the hwmon core
generates attribute names from the driver's declared bitmask, and the
conversion deleted the legacy `fan_group[]` table that created the old names.
`open()` on `fan1_manual` returns `ENOENT` on >= 7.3.

`fan1_input`, `fan1_min`, `fan1_max`, `fan1_label` and `fan1_safe` keep their
names on both generations. `afanctl doctor` prints the bound generation:

```
PASS — applesmc ABI generation — modern (kernel >= 7.3, commit 94f5081d): pwm1_enable + fan1_target; AUTO restore token "2"
```

A machine exposing **both** mode attributes is refused rather than guessed at.
The AUTO tokens are mutually incompatible — legacy `2` means *manual*, modern
`0` is `-EINVAL` — so a wrong guess is the hazard itself.

## The layers

| Layer | Mechanism | Survives |
|---|---|---|
| **L1 — per-poll verify / re-assert** | Every state-changing write is read back inside a settle window; mode drift and write failures feed a 3-strike ladder to AUTO + latched monitor-only; a stall detector catches a motionless tach | a lying or contended actuator |
| **L2 — death path** | A pre-opened `O_WRONLY` fd plus the backend's exact restore bytes, written by a panic hook and raw `SIGSEGV/SIGABRT/SIGTERM/SIGINT` handlers in a single `write(2)` | a panic, a segfault, `SIGTERM`/`SIGINT` |
| **L3 — systemd-first** | `Type=notify` + watchdog + `Restart=always` + sandboxing | a hang; `SIGKILL`/OOM-kill, via the *restart* (see below) |
| **Hardware watchdog** | A firmware/EC dead-man's switch. **applesmc has none** | `SIGKILL`, OOM-kill, a kernel panic |

`SIGKILL` and OOM-kill cannot be caught, so no signal handler can help. What
restores the fan there is the **restart**: `Restart=always` brings a new daemon
up within ~1 s, and its startup reconcile finds the fan in Manual, says so
loudly, and hands it back to the firmware (RULING F14).

## L2 is a per-backend descriptor, not a constant

L2 used to write a compile-time `b"0"`. That byte is correct for exactly one
backend on one kernel generation. The same "one byte, one syscall" mechanism,
ported by renaming a path, yields:

| Write | applesmc <= 7.2 | applesmc >= 7.3 | generic hwmon |
|---|---|---|---|
| `0` to the mode/enable attribute | restores auto | **`-EINVAL`**, stays manual | fan pinned at **full speed** indefinitely |
| `2` to the enable attribute | *means manual* | restores auto | restores auto |
| `0` to the `pwm1` duty attribute | n/a | attribute does not exist | **fan off** on a hot laptop |

L2 ignores write errors by design, so every one of those is silent. The backend
therefore supplies the `(fd, bytes)` pair as a `SafeRestore` descriptor and
`safety.rs` names no attribute and no value of its own (RULING F27).

The handler contract is unchanged: **one** lock-free atomic load and **one**
`write(2)`, with no allocation, formatting, locks or path construction. The
descriptor is published into `'static` storage at arm time, so a single
`AtomicPtr` load reaches fd, pointer and length together. A multi-byte restore
(thinkpad_acpi's `level auto`) is still one `write(2)`; its length is simply
part of the descriptor.

## The arm-time probe

A restore path that cannot be demonstrated is **not advertised as armed**.
Before the daemon reports READY it writes the restore bytes *through the very
fd the signal handler will use* and then verifies the hardware really reports
firmware control. Only then is L2 armed.

If the probe fails, afanctl:

1. logs `L2 death path ABSENT: the arm-time restore probe failed: …` and
   records it in `recent_errors`;
2. reports `safety.l2_death_path: false` in `state.json` / `status --json`;
3. refuses **every** control mode, on the startup path and on the `cmd.json`
   channel alike — no manual control without a trustworthy net (PRD R4).

The probe writes only the fail-safe value, so it can move the fan toward
firmware control and never away from it. That is why it runs in `observe` too:
R3's "observe writes nothing" means "observe issues no *control* write".

`afanctl selftest-panic` runs the same probe and then panics deliberately,
proving the whole path end to end — per backend, per generation.

## What `status` and `state` report

`state.json` (and `status --json`, which passes it through) carry an additive
`safety` object instead of a single boolean:

```json
"safety": {
  "backend": "applesmc/modern",
  "l1_verify": true,
  "l2_death_path": true,
  "l3_watchdog_notify": true,
  "hw_watchdog": false,
  "firmware_auto_on_suspend": null
}
```

- `backend` names the backend **and** the bound ABI generation.
- `l2_death_path` is read from the armed descriptor itself, not from an
  intention recorded elsewhere.
- `hw_watchdog` is `false` for applesmc: it exposes none, which is exactly why
  L2 has to exist here.
- `firmware_auto_on_suspend` is **`null` for applesmc** — unproven. There is no
  evidence either way, and `false` would be a claim. Backends whose firmware
  documents the behaviour (cros_ec_hwmon, hp-wmi) will set it.

In `status --json` the whole object is `null` when no daemon state is
published: `status` reads sysfs read-only and cannot observe another process's
armed layers, so absence of evidence is reported as such rather than guessed
at.

## Non-negotiables

- **Never write the real `/sys` during development.** Tests run against
  `tests/fixtures/sysfs/` via `--sysfs-root`, which carries a tree for **both**
  ABI generations. Hardware tests need `--features hw` + `AFANCTL_HWTEST=1` +
  applesmc present, in a supervised session.
- `unsafe` lives only in `safety.rs`, every block carrying a `// SAFETY:`
  comment that names the async-signal-safety argument.
- Never update logical state from an unverified write.

Reporting a safety defect: see [SECURITY.md](../SECURITY.md).
