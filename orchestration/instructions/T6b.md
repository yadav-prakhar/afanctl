ROLE: You are completing PHASE (b) of task T6 of the afanctl project (Rust fan supervisor,
A1708 MacBook) — wiring the real Supervisor end-to-end. Multi-agent build; you touch cli.rs +
main.rs only.

READ FIRST: DESIGN.md (BINDING) → this instruction → PRD §6.R5, R7, §7 Appendices B/C,
§11.2-T6 PHASE (b).

CONTEXT (all now IMPLEMENTED on main, read the real files):
- Supervisor (src/supervisor.rs): new(Box<dyn Smc>, &ResolvedConfig, RunMode, &RuntimePaths)
  -> Result<Self, SupError>, step_once() -> StepReport { fields per DESIGN: decision,
  applied_rpm, verified, ... }, run() -> ! (foreground loop; installs L2, READY, watchdog).
- policy::Controller::step_curve/step_hold/step_observe + Decision variants.
- smc: SysfsSmc::open(root), MockSmc::new(hw_min, hw_max); runtime dir: AFANCTL_RUNTIME_DIR
  env (default /run/afanctl) — BLESSED ruling for cmd.json/state.json.
- config: Config::load(path) -> Result<(Config, Vec<String>), ConfigError> then
  .validate(hw: (u32,u32)) and ResolvedConfig::from_c(c, hw) semantics — read config.rs to
  get exact names.
- doctor::run(roundtrip, compare_secs, json) — T7 stub still (unimplemented!): doctor verb
  MUST map the stub's panic to exit 1 + a clear error on stderr, NOT a raw 101 crash (wrap
  with catch_unwind or check a feature knob? NO — keep DESIGN semantics: doctor verb calls
  doctor::run; a panic from a stub in a release binary must not escape as CODE=101. Catch it
  in run_doctor, print "doctor: not available yet (T7 pending)" on stderr, exit 1. T7 will
  replace the body; your catch stays harmless.

PHASE (b) SCOPE (was deferred in PHASE (a), see QUESTIONS.md Q-T6-2):
1. daemon/curve/observe verbs: build the real Supervisor (config load → validate against
   (min,max) read via SysfsSmc → install L2 via safety::install_death_path(panic_fd) →
   notify::sd_ready() → run()/loop step_once). 'once' runs exactly ONE step_once and prints
   the decision line (Appendix B human format; --json when flag present).
2. Apply --at-temp <C>: inject a simulated sensor reading for the one-shot step (route via a
   MockSmc in-process — 'once --at-temp' must NOT touch real sysfs even when --sysfs-root is
   set; document in -h).
3. --dry-run: run the controller step but do NOT actuate via smc (decision computed, nothing
   written). state file NOT written in dry-run.
4. hold <rpm>: clamp/reject against hw range BEFORE writing cmd.json (fan1_min/max from
   SysfsSmc::open at discovery). Rejects below fan1_min with exit 1 and key+fix message.
5. status: merge config + state file + direct sysfs reads (R7). MISSING state file →
   'daemon: not running' line (already correct from phase a) — verify it still merges real
   sensor/fan readings.
6. Ensure every deferred item in QUESTIONS.md Q-T6-2 is now closed or explicitly re-listed.

RULES:
- Touch ONLY src/cli.rs and src/main.rs (+ co-located tests). Append-only DEVIATIONS.md /
  QUESTIONS.md under `## T6` if needed. Everything else read-only.
- DESIGN.md signatures are binding; supervisor/smc/config/policy constructors are called, not
  modified. No new dependencies. No unsafe. No unwrap/expect in product paths outside tests
  (this applies to the NEW code you add: check_unwind usage included).
- NEVER real /sys in tests; tests use MockSmc + fixtures + AFANCTL_RUNTIME_DIR=tempdir.
- No sudo/pkexec/install; do not merge; commit on current branch; git status clean.

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

DONE WHEN (show outputs):
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo test --features hw   # must SKIP cleanly
plus: an integration-style test proving daemon wiring builds a real Supervisor against
MockSmc inside the test binary (no runtime binary spawn needed — call cli entry fns directly).

FINAL OUTPUT: files changed, gate outputs, tests added, deferred items status.

