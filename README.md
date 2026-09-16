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

## How it works

`afanctl` reads `coretemp` sensors, drives the single fan through `applesmc`'s
`fan1_manual` / `fan1_output`, and — when anything goes wrong — gets out of
the way and lets the SMC firmware run the fan again.

| Layer | Guarantee |
| ----- | --------- |
| **L1 — per-poll verify / re-assert** | Mode drift and write failures feed a 3-strike ladder to AUTO + monitor-only; tracking wobble is re-asserted, never counted; a stall detector catches a motionless tach |
| **L2 — death path** | Pre-opened fd + panic/signal handlers restore AUTO in a single `write(2)` — proven by the `selftest-panic` probe |
| **L3 — systemd-first** | `Type=notify` + watchdog + `Restart=always` + sandboxing, so a hang or crash restarts rather than stranding the fan |

Full rationale and constants: [Safety Model](https://github.com/yadav-prakhar/afanctl/wiki/Safety-Model).

## Quick start

```sh
# build the package from a checkout (does not install)
cd packaging && makepkg

# build + install (requires root)
cd packaging && makepkg -si

# the unit installs DISABLED and starts in observe mode — nothing changes yet
sudo systemctl enable --now afanctl
systemctl status afanctl
```

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

> **Run doctor as root.** The write-mode checks open the manual file `O_WRONLY`,
> so as non-root they FAIL by design (exit 1) — an unwritable manual file leaves
> the L2 death path unarmed, which is exactly a real problem, not a false alarm.

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

## Contributing

PRs welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) first — it covers
the quality gates, the "never touch real `/sys`" rule, and the engineering
conventions (binding, from `DESIGN.md` §8). By participating you agree to the
[Code of Conduct](CODE_OF_CONDUCT.md); security issues go through
[SECURITY.md](SECURITY.md), never a public issue.

## License

GPL-3.0-only — see [`LICENSE`](LICENSE). Free software with no warranty; the
Cargo/package metadata uses the SPDX form `GPL-3.0-only`.
