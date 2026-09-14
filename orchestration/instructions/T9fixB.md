ROLE: You are fixing DEFECT TICKETS in the afanctl project (Rust fan supervisor, A1708 Megabyte)
as ONE assigned agent. Multi-agent build; a PARALLEL agent (Ticket A) owns cli.rs/doctor.rs/
tests/integration.rs — do NOT touch their files.

READ FIRST: orchestration/REVIEW-T9.md (full report; you own F3, F4, F5, F6, F10, F11, F13-doc)
→ DESIGN.md (BINDING) → cited PRD sections.

VERIFIED DEFECTS YOU OWN:
- F3 MAJOR (src/config.rs + test): config accepts interval_s with no upper bound; the systemd
  unit pins WatchdogSec=15 and the supervisor pings once per poll → any interval_s >= 15
  crash-loops the daemon (SIGABRT→L2→Restart=always). FIX: validate an upper bound in
  Config::validate with key+fix in the message (bound: poll interval must be < WatchdogSec;
  make the watchdog budget a named const in config.rs documenting the coupling to the unit
  file; pick 15-1s margin → reject interval_s >= 15 with a message naming the unit constant).
  Add validation test.
- F5 MAJOR (src/config.rs, src/policy.rs + tests): negative/absurd thresholds accepted →
  i32 overflow panic in policy.rs:195/204 (debug panic; release wraparound). FIX IN TWO
  LAYERS: (1) config.rs validates a sane °C band for high/max (e.g. 0..=95 both, naming
  key+fix, matching R6's intent — a fan controller's thresholds that can't reproduce a
  real temperature are a config error); (2) policy.rs converts the two arithmetic sites to
  checked/saturating math so a future config bug degrades loudly, never to UB. Tests both.
- F4 MAJOR (Cargo.toml + per-file allowlists EXCEPT cli.rs/doctor.rs — Ticket A attrs its
  own files): DESIGN §8 mandates clippy::unwrap_used/expect_used/panic denied. Add to
  Cargo.toml: [lints.rust] and [lints.clippy] with unwrap_used/expect_used/panic at
  "warn" so `-D warnings` gates (edition-2021 workspace-wide), and add targeted
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` inside #[cfg(test)]
  mod tests blocks ONLY in files you own (config.rs/policy.rs/safety.rs tests, notify.rs,
  smc.rs, supervisor.rs, main.rs). NOTE for files you don't own (cli.rs, doctor.rs):
  put a TODO in DEVIATIONS.md — the gates will be red for their files until Ticket A lands;
  sequence your final gates with that in mind: run gates EXCLUDING those two files' blame by
  allowing them too — NO: instead, coordinate disabled: your branch must keep `cargo clippy
  --all-targets --all-features -- -D warnings` green. To achieve that WITHOUT touching their
  files, add file allowlist attributes ONLY inside your own files, and inside
  DEVIATIONS.md note that cli.rs/doctor.rs coverage lands in Ticket A + add the attrs are
  added by A. Wait — you cannot gate globally without their attrs. RESOLUTION: set the
  lints to "allow" in Cargo.toml is banned. Instead implement F4 as: [lints] entries live in
  Cargo.toml with level = "warn"; the -D warnings gate stays as-is (warnings become errors
  globally); to keep the build green you MAY add #[allow(...)] scoped ONLY inside cfg(test)
  modules in files you own; for cli.rs/doctor.rs test blocks, Ticket A handles. Your final
  gate command: cargo clippy with the additional -W unwrap_used/expect_used/panic flags
  MUST be green for src/ minus cli/doctor (checked by running it and grepping the blame for
  cli.rs/doctor.rs — if ANY error names those two files, you note it as "covered by Ticket
  A" and the flag must still pass for all other files). Also add the expanded clippy gate
  line into check.sh (gate 2b).
- F6 MINOR (README.md — do NOT touch cli.rs): document that `selftest-panic` exits 101
  (deliberate probe) alongside 0/1/2.
- F10 MINOR (README.md): document the cmd-file freshness gate (verbatim re-issue ignored).
- F11 MINOR (README.md): document that doctor's write-mode checks require root; as non-root
  they FAIL by design (an unwritable manual file leaves L2 unarmed).
- F13-doc (README.md): a "Complexity" note: product LOC ~3.5k vs the ~1.4k aspiration and
  why (mandated fault-injection + doctor surfaces); plans slimming deferred to post-gate.

RULES:
- Touch ONLY: src/config.rs, src/policy.rs, Cargo.toml, check.sh, README.md, packaging/ NOTES
  none needed there, + append-only DEVIATIONS.md/QUESTIONS.md under `## T9F-B`. ALL other
  files read-only (cli.rs/doctor.rs/run selftest are Ticket A's).
- DESIGN.md signatures binding — F5's band validation must NOT change ResolvedConfig shapes
  (error variants may gain a field? NO — reuse Invalid { key, reason, fix }). policy.rs
  arithmetic: no public signature changes.
- Tests required: interval upper-bound rejection; threshold-band rejection; overflow-safety
  regression (the exact F5 repro must now exit 1 with key+fix, not panic — write it as a test
  against the public config API).
- NEVER real /sys; no sudo/pkexec/install; no merge; commit on current branch; git clean.

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
cargo test --features hw
plus: a test for each of F3/F5 demonstrating the key+fix message.

FINAL OUTPUT: files changed; per-defect resolution; gate outputs; D/Q entries.
