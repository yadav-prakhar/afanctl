# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Beautified `README.md` (centered hero, shields badges, concise sections with
  wiki links) and standard open-source housekeeping (`CONTRIBUTING.md`,
  `CODE_OF_CONDUCT.md`, `SECURITY.md`, issue/PR templates, this changelog).

## [0.1.0] - 2026-09-15

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

### Verified

- Full hardware acceptance gate PASSED on MacBookPro14,1 (kernel
  `7.2.3-arch1-3`): doctor, roundtrip, death-path panic, 1 h+ observe soak
  (0.05 % CPU / 2.4 MB peak), `SIGKILL` rescue in curve mode, 600-sample
  `doctor --compare`, polkit `hold`, reboot test. Four adversarial review
  rounds (`orchestration/REVIEW-T9*.md`).

[Unreleased]: https://github.com/yadav-prakhar/afanctl/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yadav-prakhar/afanctl/releases/tag/v0.1.0
