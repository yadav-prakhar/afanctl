# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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