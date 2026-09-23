<p align="center">
  <h1 align="center">afanctl</h1>
  <p align="center">
    A small, safety-first fan supervisor for the pre-T2 Intel Mac<br>
    (MacBook Pro A1708 · <code>applesmc</code>) — <i>fail toward the firmware.</i>
  </p>
  <p align="center">
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--only-blue.svg" alt="License: GPL-3.0-only"></a>
    <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-1.98%2B-orange.svg?logo=rust" alt="Rust 1.98+"></a>
    <a href="https://github.com/yadav-prakhar/afanctl/wiki/Hardware-Interface"><img src="https://img.shields.io/badge/platform-Arch%20Linux%20%C2%B7%20A1708-lightgrey.svg" alt="Platform: Arch Linux, MacBook Pro A1708"></a>
    <a href="https://github.com/yadav-prakhar/afanctl/wiki"><img src="https://img.shields.io/badge/docs-wiki-green.svg" alt="Documentation: wiki"></a>
    <a href="https://github.com/yadav-prakhar/afanctl/releases"><img src="https://img.shields.io/github/v/release/yadav-prakhar/afanctl" alt="Latest release"></a>
  </p>
  <p align="center">
    <a href="https://github.com/yadav-prakhar/afanctl/wiki">📖 Wiki</a> ·
    <a href="https://github.com/yadav-prakhar/afanctl/wiki/Installation">Install</a> ·
    <a href="https://github.com/yadav-prakhar/afanctl/wiki/CLI-Reference">CLI</a> ·
    <a href="https://github.com/yadav-prakhar/afanctl/wiki/Troubleshooting-and-FAQ">FAQ</a> ·
    <a href="CONTRIBUTING.md">Contribute</a>
  </p>
</p>

---

> ✅ **Shipped and verified on hardware** — MacBookPro14,1, kernel
> `7.2.3-arch1-3`, systemd. The full acceptance gate (doctor, 2-second
> roundtrip write test, deliberate-panic death-path test, 1 h+ observe soak at
> 0.05 % CPU / 2.4 MB peak, `SIGKILL` rescue in curve mode, 600-sample
> `doctor --compare` table, polkit-driven `hold`, reboot test) — all PASSED.
> Details: [Background and Verification](https://github.com/yadav-prakhar/afanctl/wiki/Background-and-Verification).
>
> 🎯 **Scope:** one fan, one machine class. All sysfs path knowledge lives in
> one module; the control policy is pure and table-tested.

## Supported kernels

applesmc exposes two different sysfs attribute sets. **afanctl supports both in
one binary** and binds one at startup by probing for the files — never by
parsing a kernel version. `afanctl doctor` prints which one it bound.

| Generation | Kernel | rpm setpoint | Mode attribute | Manual | **AUTO** |
| ---------- | ------ | ------------ | -------------- | ------ | -------- |
| `legacy`   | **<= 7.2** | `fan1_output` | `fan1_manual` | `1` | **`0`** |
| `modern`   | **>= 7.3** | `fan1_target` | `pwm1_enable` | `1` | **`2`** |

Kernel commit `94f5081d` (released in 7.3) renamed both attributes with no
back-compat aliasing, and changed the mode vocabulary: on `pwm1_enable`, `0` is
rejected with `-EINVAL`, so the AUTO token that hands the fan back to the
firmware differs between generations. `fan1_max` also became read-only (afanctl
only ever reads it) and there is no `pwm1` duty attribute. Details:
[docs/SAFETY.md](docs/SAFETY.md).

## How it works

`afanctl` reads `coretemp` sensors, drives the single fan through the
`applesmc` attribute set it detected, and — when anything goes wrong — gets out
of the way and lets the SMC firmware run the fan again.

| Layer | Guarantee |
| ----- | --------- |
| **L1 — per-poll verify / re-assert** | Mode drift and write failures feed a 3-strike ladder to AUTO + monitor-only; tracking wobble is re-asserted, never counted; a stall detector catches a motionless tach |
| **L2 — death path** | Pre-opened fd + panic/signal handlers restore firmware control in a single `write(2)`, using the **bound generation's own restore bytes** — proven at daemon arm time by a real write, and by the `selftest-panic` probe. An unprovable restore is reported absent, never armed |
| **L3 — systemd-first** | `Type=notify` + watchdog + `Restart=always` + sandboxing, so a hang or crash restarts rather than stranding the fan |

`status --json` reports which layers are actually armed (`safety.l1_verify`,
`.l2_death_path`, `.l3_watchdog_notify`, `.hw_watchdog`,
`.firmware_auto_on_suspend`) rather than a single boolean.

Full rationale and constants: [docs/SAFETY.md](docs/SAFETY.md) ·
[Safety Model](https://github.com/yadav-prakhar/afanctl/wiki/Safety-Model).

## Install

```sh
# one command: fetches the package attached to the latest release, checks its
# SHA-256 against that release's SHA256SUMS, and installs it with pacman
curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | sudo bash
```

That is the whole install. The script lives in this repo as
[`install.sh`](install.sh) — read it before you pipe it, or ask it to resolve,
download and verify without installing:

```sh
curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | bash -s -- --dry-run
curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | sudo bash -s -- --version 0.1.0
```

```sh
# from a checkout instead
cd packaging && makepkg -si
```

```sh
# either way the unit installs DISABLED and starts in observe mode
sudo systemctl enable --now afanctl
systemctl status afanctl
afanctl status
```

Neither the package nor `systemctl enable` hands the fan over: the daemon starts
in `observe`, writing nothing, until you ask for `curve`. Re-run the install
command to upgrade; `sudo pacman -R afanctl` removes it.

> **Not on the AUR yet.** Registration there is closed to new maintainers, so
> the release package is the distribution channel; `packaging/aur/` keeps the
> AUR recipe ready for when that changes, and
> [`packaging/README.md`](packaging/README.md) tracks every channel.

```sh
afanctl status          # temps, t_eff, mode, fan, config provenance
sudo afanctl doctor     # diagnostics — run as root (see below)
afanctl once --dry-run  # one control decision, writes nothing
```

📦 Install details and installed paths: [Installation](https://github.com/yadav-prakhar/afanctl/wiki/Installation) ·
🔧 Unit, PKGBUILD and polkit rule: [systemd and Packaging](https://github.com/yadav-prakhar/afanctl/wiki/systemd-and-Packaging)

## Command line

| Verb | Behavior |
| ---- | -------- |
| `daemon [--mode observe\|curve]` | Run the supervisor loop (default `observe`) |
| `status [--json]` | Temps, `t_eff`, mode, fan actual/target/min/max, config provenance, recent errors |
| `doctor [--json] [--roundtrip] [--compare <s>]` | Diagnostics; exit 1 if any check FAILs |
| `once [--at-temp <C>] [--dry-run] [--json]` | Exactly one control iteration (scripts/CI) |
| `observe` / `curve` | Write the command file; the daemon applies it |
| `hold <rpm>` | Hold a fixed rpm (rejected below `fan1_min`, clamped to `fan1_max`) |

Exit codes: `0` success · `1` runtime failure · `2` usage error · `101` deliberate `selftest-panic`.

📚 Every verb, flag and exit code: [CLI Reference](https://github.com/yadav-prakhar/afanctl/wiki/CLI-Reference) ·
🩺 All checks, `--roundtrip` and `--compare`: [Diagnostics (doctor)](https://github.com/yadav-prakhar/afanctl/wiki/Diagnostics-%28doctor%29)

> **Run doctor as root.** The write-mode checks open the fan mode attribute
> `O_WRONLY`, so as non-root they FAIL by design (exit 1) — an unwritable mode
> attribute leaves the L2 death path unarmed, which is exactly a real problem,
> not a false alarm.

## Configuration

`/etc/afanctl/afanctl.toml` — typed TOML with **exactly five** honoured keys
(unknown keys warn and are ignored; a present-but-broken file refuses to start):

```toml
[thresholds]          # °C, integers
high = 66             # ramp starts here; low is derived: high - 3
max  = 86             # full speed from here; guard: max <= 95 (Tjmax 100 - 5)

[curve]
min_rpm = 1200        # clamped to >= fan1_min at load
max_rpm = 6200        # clamped to <= fan1_max at load

[poll]
interval_s = 1        # 1..=12; must stay well below WatchdogSec=15
```

⚙️ Defaults, validation table and watchdog coupling: [Configuration](https://github.com/yadav-prakhar/afanctl/wiki/Configuration) ·
📈 Curve formula, hysteresis and slew limiter: [Control Policy](https://github.com/yadav-prakhar/afanctl/wiki/Control-Policy)

## Plugin surface

`/run/afanctl/cmd.json` (`afanctl.cmd.v1`) is the plugin-facing control
channel; `status --json` is its render feed. Presets map to one-liners
(`observe`, `hold 1200` … `hold <rpm>`), guarded by an optional polkit rule for
`wheel`. Plugins must render the `monitor_only` latch, not the mode alone.

🔌 Channel, freshness gate and presets: [Plugin Surface (omafan)](https://github.com/yadav-prakhar/afanctl/wiki/Plugin-Surface-%28omafan%29) ·
🧾 Every field: [JSON Schemas](https://github.com/yadav-prakhar/afanctl/wiki/JSON-Schemas)

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   # skips cleanly without real applesmc
```

Or simply `./check.sh` — it runs all four gates. Tests run against fixture
sysfs trees; ordinary development **never writes to the real `/sys`**.

🧪 Gates, fixtures and the `hw` guard: [Development and Testing](https://github.com/yadav-prakhar/afanctl/wiki/Development-and-Testing) ·
🏗️ Crate layout and module contracts: [Architecture and Internals](https://github.com/yadav-prakhar/afanctl/wiki/Architecture-and-Internals)

## Docs map

| I want… | Go to… |
| ------- | ------ |
| The full story | [Wiki Home](https://github.com/yadav-prakhar/afanctl/wiki) |
| Install / upgrade / enable | [Installation](https://github.com/yadav-prakhar/afanctl/wiki/Installation) |
| Something broke | [Troubleshooting and FAQ](https://github.com/yadav-prakhar/afanctl/wiki/Troubleshooting-and-FAQ) |
| Why not mbpfan / fancontrol | [afanctl vs mbpfan](https://github.com/yadav-prakhar/afanctl/wiki/afanctl-vs-mbpfan) |
| Requirements & build plan | `PRD.md` · `PLAN.md` · `DESIGN.md` in this repo |
| To contribute | [CONTRIBUTING.md](CONTRIBUTING.md) |
| To publish a release (GitHub, AUR, Omarchy) | [`packaging/README.md`](packaging/README.md) |

## Contributing

PRs welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first — it covers
the quality gates, the "never touch real `/sys`" rule, and the engineering
conventions (binding, from `DESIGN.md` §8). By participating you agree to the
[Code of Conduct](CODE_OF_CONDUCT.md); security issues go through
[SECURITY.md](SECURITY.md), never a public issue.

## License

GPL-3.0-only — see [`LICENSE`](LICENSE). Free software with no warranty; the
Cargo/package metadata uses the SPDX form `GPL-3.0-only`.
