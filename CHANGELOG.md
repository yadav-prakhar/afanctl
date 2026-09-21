# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **applesmc ABI break on kernel >= 7.3 (#2).** Kernel commit `94f5081d`
  ("hwmon: (applesmc) Convert to `hwmon_device_register_with_info`", released
  in 7.3) renamed `fanX_output` → `fanX_target` and `fanX_manual` →
  `pwmX_enable` with **no back-compat aliasing**, so afanctl failed to start on
  7.3+ (`open()` on `fan1_manual` returns `ENOENT`). afanctl now supports
  **both attribute generations in one binary** and binds one at startup by
  probing for the files — never by parsing a kernel version. The mode
  vocabulary is mapped per generation: legacy `Manual=1 / Auto=0`, modern
  `Manual=1 / Auto=2` (modern rejects `0` with `-EINVAL`). `fan1_max` being
  read-only on the modern ABI is handled — it was only ever read. A machine
  exposing both mode attributes is refused rather than guessed at, because the
  AUTO tokens are mutually incompatible. `doctor` reports the bound generation
  in an additive `applesmc ABI generation` check; existing check names are
  unchanged. (RULING F26)

### Changed

- **The L2 death path is now a per-backend descriptor, proven at arm time
  (#3).** `safety.rs` no longer holds a compile-time restore byte and now
  contains **no attribute name and no restore value**: the backend supplies a
  `SafeRestore { fd, bytes: &'static [u8] }` and `safety.rs` merely consumes
  it. This is what makes the ABI fix safe — the naive port, renaming the path
  while keeping the byte `b"0"`, would have been *more* dangerous than the
  startup failure, since applesmc >= 7.3 rejects `0` and L2 ignores write
  errors by design, leaving the fan pinned in manual with no supervisor alive.
  The async-signal-safe handler contract is unchanged: one lock-free atomic
  load, one `write(2)`, no allocation, formatting, locks or path construction;
  a multi-byte restore is still a single `write(2)` with the length carried in
  the descriptor. Before advertising L2, the daemon now performs a real write
  of the restore bytes through the very fd the handler will use and verifies
  the hardware reports firmware control; an unprovable restore is reported
  **L2 absent, loudly** and afanctl refuses every control mode rather than
  arming a broken path. `Smc::panic_fd` is replaced by `safe_restore`,
  `probe_safe_restore` and `safety_capabilities`. (RULING F27)
- `status --json` and `state.json` gained an additive `safety` object
  reporting which layers are **actually armed** — `backend` (with the bound ABI
  generation), `l1_verify`, `l2_death_path`, `l3_watchdog_notify`,
  `hw_watchdog`, `firmware_auto_on_suspend` — instead of a single boolean the
  plugin had to infer from. Schema ids stay `afanctl.status.v1` /
  `afanctl.state.v1`. `firmware_auto_on_suspend` is `null` for applesmc: no
  evidence either way, and `false` would be a claim. (RULING F27)
- Log and error messages in `supervisor.rs` / `cli.rs` now describe the
  *logical* fan mode instead of naming `fan1_manual`, which is wrong on half
  the installed base; the real attribute path still reaches the user via
  `SmcError`, generation-correctly. `doctor` check names are unchanged — they
  are a public contract. (RULING F26)

### Added

- `tests/fixtures/sysfs/modern/` — a full kernel >= 7.3 fixture tree, so both
  generations are covered end to end: per-generation restore bytes, a
  regression guard proving the legacy AUTO byte is **rejected** by a modern
  tree, a backend whose arm-time probe fails not arming, `selftest-panic` per
  generation, and a modern-ABI daemon round trip through L2.
- `docs/SAFETY.md` — the per-backend safety model and the supported kernel
  range per ABI generation.

## [0.1.0] - 2026-09-16

### Added

- Safety-first fan supervisor for the pre-T2 Intel Mac (MacBook Pro A1708 /
  `applesmc`): `coretemp` sensing, single-fan control via `fan1_manual` /
  `fan1_output`, fail-toward-firmware on every failure path.
- Three safety layers: L1 per-poll verify/re-assert + stall detector, L2
  death-path fd (panic/signal handlers, `selftest-panic` probe), L3
  systemd-first (`Type=notify`, watchdog, `Restart=always`, sandboxing).
- CLI verbs: `daemon`, `status`, `doctor` (+ `--roundtrip`, `--compare`),
  `once`, `observe` / `curve`, `hold`, hidden `selftest-panic`; exit codes
  0/1/2 (+101 deliberate panic).
- Typed 5-key TOML config with refuse-to-start validation; plugin channel
  (`cmd.json` / `state.json`, `monitor_only` latch); optional polkit rule for
  `wheel`; Arch packaging (`PKGBUILD`, systemd unit, default config).
- Full test suite (unit + fixture integration + `hw`-gated hardware tests) and
  the four quality gates in `check.sh`.
- Docs: `README.md`, `PRD.md`, `DESIGN.md` (binding contracts), `PLAN.md`,
  project [wiki](https://github.com/yadav-prakhar/afanctl/wiki).
- Beautified `README.md` (centered hero, shields badges, concise sections with
  wiki links) and standard open-source housekeeping (`CONTRIBUTING.md`,
  `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue/PR templates, this changelog).

### Distribution

- **One-command install from a GitHub release:**
  `curl -fsSL https://github.com/yadav-prakhar/afanctl/releases/latest/download/install.sh | sudo bash`.
  `install.sh` reads the release's `SHA256SUMS`, downloads the `x86_64` package
  it names, verifies the SHA-256 (a mismatch aborts before pacman runs) and
  installs it with `pacman -U`. `--dry-run` verifies without installing,
  `--version` pins a release, and it never enables or starts the unit. Being
  version-agnostic, it survives future releases untouched.
- Tagged releases are built and published by `.github/workflows/release.yml` in
  a clean Arch Linux container: the four gates, the package, the versioned
  source tarball, `install.sh` and `SHA256SUMS`, all attached to the GitHub
  release. `.gitattributes` keeps `packaging/aur` out of the tarball so a
  published checksum is never self-referential.
- `packaging/README.md` documents every channel: the release artifacts (live),
  the complete-but-unpublished AUR recipe in `packaging/aur/` (the AUR has closed
  registration to new maintainers), a proposal to the
  [Omarchy Package Repository](https://github.com/omacom/omarchy-pkgs/pull/476)
  that tracks releases independently of the AUR, and `cargo publish` for
  crates.io when it is wanted.

### Verified

- Full hardware acceptance gate PASSED on MacBookPro14,1 (kernel
  `7.2.3-arch1-3`): doctor, roundtrip, death-path panic, 1 h+ observe soak
  (0.05 % CPU / 2.4 MB peak), `SIGKILL` rescue in curve mode, 600-sample
  `doctor --compare`, polkit `hold`, reboot test. Four adversarial review
  rounds (`orchestration/REVIEW-T9*.md`).

[Unreleased]: https://github.com/yadav-prakhar/afanctl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yadav-prakhar/afanctl/releases/tag/v0.1.0