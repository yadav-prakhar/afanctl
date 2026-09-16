# Contributing to afanctl

Thanks for your interest! `afanctl` drives a real fan on real hardware, so this
project is deliberately strict. Please read this whole file before opening a PR.

## Ground rules

1. **Never write to the real `/sys` during development.** All tests run against
   `tests/fixtures/sysfs/` via `--sysfs-root`. Real-hardware tests run only
   behind `--features hw` + `AFANCTL_HWTEST=1` + applesmc present, and only in a
   supervised session on the target machine. If your test would write real sysfs
   without those guards, your test is a bug.
2. **Never run the daemon against real hardware** as part of ordinary
   development. The hardware gate in `PLAN.md` Appendix P5 is operator-run, with
   the owner present.
3. **Fail toward the firmware.** Every new failure path must end in AUTO /
   refusal-to-start / a loud log — never silence. Every state-changing write
   goes through write-verify; never update logical state from an unverified
   write.
4. **Contracts are frozen.** Public signatures live in `DESIGN.md` Appendix A.
   If your change needs a contract change, record it in `DEVIATIONS.md`
   (old → new → why → affected areas) and call it out in the PR. Never silently
   rename or add public items.

## Engineering conventions (binding)

From `DESIGN.md` §8 — the short version. The full text there wins on any
disagreement.

- **Language & toolchain:** Rust edition 2021, rustc 1.98 (`sudo pacman -S
  rust`). No async. No threads beyond the main loop. Dependencies are allowlisted
  (`serde`, `toml`, `serde_json`, `thiserror`, `tracing`, `tracing-subscriber`,
  `libc`) — adding one needs a `DEVIATIONS.md` entry and maintainer approval.
- **Correctness:** no `unwrap` / `expect` / `panic!` outside tests, `main.rs`
  wiring, and `safety.rs`'s deliberate test panic (clippy-gated). `unsafe` only
  in `safety.rs`, every block with a `// SAFETY:` comment. Temps are `MilliC`,
  never bare `i32`, across module boundaries; rpm are `u32`.
- **Style:** `cargo fmt` defaults, no customization. Every file gets a `//!`
  module doc (contract + invariants, ≤10 lines). Public items get doc comments.
  Safety-critical lines get `// SAFETY:` or `// INVARIANT:` comments. Errors are
  per-module `thiserror` enums whose messages name the path/key **and the fix**.
  Logging via `tracing` macros only — no `println!` outside `cli.rs`.

## Workflow

```sh
# 1. Fork, branch from master, make your change
# 2. Run the full gate (all four must pass):
./check.sh
# which runs:
#   cargo fmt --check
#   cargo clippy --all-targets --all-features -- -D warnings
#   cargo test
#   cargo test --features hw   # must SKIP cleanly — proves the hw guard works
```

- Keep PRs focused: one behavior per PR. Include tests for every behavioral
  claim (unit tests co-located under `#[cfg(test)]`, cross-module tests in
  `tests/`).
- Update docs in the same PR: user-facing behavior → the
  [wiki](https://github.com/yadav-prakhar/afanctl/wiki) page for that topic;
  requirements/contracts → `PRD.md` / `DESIGN.md` as appropriate.
- Use the PR template. Fill in the safety checklist — PRs that touch `src/`
  without green gates will not be reviewed.

## Reporting issues

- **Bugs:** use the bug-report template. Include `afanctl status` output,
  `sudo afanctl doctor` output, your kernel (`uname -r`), and whether the unit
  was restarted after the last reinstall (stale-binary trap — see the wiki
  [Troubleshooting and FAQ](https://github.com/yadav-prakhar/afanctl/wiki/Troubleshooting-and-FAQ)).
- **Security:** do **not** open a public issue — see [SECURITY.md](SECURITY.md).
- **Questions:** check the [FAQ](https://github.com/yadav-prakhar/afanctl/wiki/Troubleshooting-and-FAQ)
  first, then open an issue with the `question` label.

## Code of Conduct

Be kind, be precise, assume good intent. The full text is in
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
