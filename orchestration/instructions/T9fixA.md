ROLE: You are fixing DEFECT TICKETS in the afanctl project (Rust fan supervisor, A1708 MacBook)
as ONE assigned agent. Multi-agent build; a PARALLEL agent (Ticket B) owns config.rs/policy.rs/
Cargo.toml/README/packaging — do NOT touch their files even where a fix might feel natural.

READ FIRST: orchestration/REVIEW-T9.md (full report — your defects are F1, F2, F7, F8, F9-half,
F12) → DESIGN.md (BINDING) → the cited PRD sections.

VERIFIED DEFECTS YOU OWN (reproducible; orchestrator re-ran each):
- F1 CRITICAL (src/cli.rs + tests/integration.rs): live `afanctl once` normal-exits leaving
  fan1_manual=1 with no L2 and no AUTO restore (C1 resurrected by a P0 verb in normal running).
  FIX: before `once` exits — whether by success, VerifyFailed, or any error path — restore AUTO
  via smc set_mode(Auto) (verify path) AND arm L2 (install_death_path with the panic fd from
  SysfsSmc::open) for the duration of the verb so a mid-verb crash also restores AUTO. Update
  tests/integration.rs::once_* which currently LOCKS IN fan1_manual=1 (must now assert ==0 on
  a clean exit; error-path still nonzero exit). Also update the README once-row to state that
  `once` restores AUTO on exit.
- F2 MAJOR (src/cli.rs + src/doctor.rs): doctor must honor the --sysfs-root and --config
  GLOBALS. DESIGN.md doctor::run has been AMENDED by ruling D-T9-F2 to
  pub fn run(sysfs_root: &std::path::Path, config_path: Option<&std::path::Path>,
             roundtrip: bool, compare_secs: Option<u64>, json: bool) -> i32
  — implement exactly this; cli passes the parsed globals through (delete the stale T7-era
  catch_unwind stub-pad — that IS your F8). Defaults: sysfs_root=/sys, config=None→module
  default. Consolidate the duplicated "/sys" + config-path constants via smc.rs-exported
  consts IF smc.rs already exports them (do not add new public items without a DEVIATIONS
  entry — otherwise keep one private const in cli.rs and have doctor take params only).
- F7 MINOR (src/cli.rs): status renders uptime_s from watchdog_pings — schema fidelity bug.
  Appendix B field is uptime_s: compute from pings × interval_s (interval from config as
  loaded), OR rename the field in state + Appendix B(sy) — NO silent rename: if you rename
  uptime_s, add DEVIATIONS entry D-T9F7 with propagation. Simpler acceptable fix: render
  pings × interval_s and document the approximation in Appendix B comment (no signature
  change). Choose; document choice in DEVIATIONS.
- F8 MINOR (src/cli.rs): remove/replace the stale catch_unwind doctor panic message
  ("not available yet (T7 pending)") — doctor is implemented now; a genuine panic should
  surface truthfully. With F2's rework this may vanish naturally;证 nothing false remains in
  stderr text.
- F9 MINOR (doc-level append to DEVIATIONS): note the "/sys" duplication resolution chosen.
- F12 MINOR (src/cli.rs): value-taking flags must reject minus-prefixed garbage values
  (`--config --json`) — treat a following token starting with '-' (except negative numbers for
  --at-temp) as a missing value → usage + exit 2. Parser-table test for it.

AND (from the same report, deferred wiring): tests/integration.rs additions must cover the
fixed behaviors (once-exit-restore + doctor honoring sysfs-root).

RULES:
- Touch ONLY: src/cli.rs, src/doctor.rs, tests/integration.rs, + append-only DEVIATIONS.md/
  QUESTIONS.md under `## FX-A`. NOTHING else (config.rs/policy.rs/Cargo.toml/README are
  Ticket B's; smc.rs is T3's — do not modify even for consts).
- DESIGN.md signatures binding. No unsafe. No unwrap/expect/panic in product code paths you
  add. Error messages name path/key + fix. --json only where approved, etc.
- NEVER real /sys; tests use tempdir fixture copies (copy_tree pattern in smc.rs tests).
- No sudo/pkexec/install; no merge; commit on current branch; git status clean.

CONVENTIONS (DESIGN.md §8, binding):
## 8. Engineering conventions (BINDING — paste verbatim into every subagent instruction)

**Language & toolchain**
- Rust edition 2021; rustc 1.98 (Arch `extra/rust`; install via `sudo pacman -S rust` — or `pkexec --disable-internal-agent pacman -S rust` on this box).
- No async. No threads beyond the main loop (single-threaded design; `Send` bound on `Smc` is for the trait's future, not for spawning).
- Dependencies: allowlist in R11 only. Adding one = stop, record in `DEVIATIONS.md`, await planner.

**Correctness discipline**
- No `unwrap`/`expect`/`panic!` outside tests, `main.rs` wiring, and `safety.rs`'s deliberate test panic. Clippy denies them (`clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic` — allowlisted per-file where required).
- `unsafe` only in `safety.rs`, each block carrying a `// SAFETY:` comment naming the async-signal-safety argument.
- Every state-changing `smc` write goes through write-verify (R1). Never update logical state from an unverified write (the `mbpfan.c:410` class).
- Temps are `MilliC`, never bare `i32` across module boundaries. rpm are `u32`.

**Style**
- `cargo fmt` default config, no customization. `//!` module doc on every file stating its contract and invariants in ≤10 lines. Public items get doc comments. Safety-critical lines get `// SAFETY:` or `// INVARIANT:` comments.
- Errors: per-module `thiserror` enum; error messages name the path/key and the fix. Logging via `tracing` macros only — no `println!` outside `cli.rs` output formatting.
- Tests: unit tests co-located (`#[cfg(test)]`); cross-module tests in `tests/`. Every behavioral claim in this PRD that is testable on fixtures/mocks **must** have a test (see R10 list).

**Safety of the build process itself (non-negotiable)**
- **Never write to the real `/sys` during development.** All tests run against `tests/fixtures/sysfs/` via `--sysfs-root`. Real-hardware tests run only behind `--features hw` + `AFANCTL_HWTEST=1` + applesmc present, and only in the final supervised gate (§9). If a test would write real sysfs without those guards, that test is a bug.
- Never run the daemon against real hardware as part of ordinary development; the supervised gate does that, with the user present.

**Cross-agent protocol**
- You may only create/modify the files listed in your task card. Everything else is read-only.
- Signatures come from Appendix A (repo copy: `DESIGN.md`). If a required change is discovered: implement everything else, record the proposed change in `DEVIATIONS.md` (old → new → why → which tasks are affected), and flag it in your final summary. Never silently rename/add public items.
- Ambiguity or a blocking question → write it to `QUESTIONS.md`, continue with the unambiguous remainder, and say so in your summary. Do not invent spec.
- LOC budget per file in §7; >20% over = stop and report (scope smell), do not absorb silently.

**Quality gates (every task, before declaring done)**
```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   # must SKIP cleanly (no applesmc in dev containers) — proves the guard works
```
Plus the task card's own checks. A task is done only when all gates pass in a clean checkout of its branch.



DONE WHEN (show outputs of all, from a clean checkout of your branch):
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   (must skip cleanly)
plus: your own re-run of the F1 repro (must print 0 after once) and the F2 repro printing the
fixture's 45.0 C (not the live machine's temp), quoted in your final message.

FINAL OUTPUT: files changed; per-defect fix summary; D/Q entries; gate outputs.
