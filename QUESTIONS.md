# QUESTIONS

Append-only log of ambiguities or blocking questions. Do not invent spec;
record the question, do the unambiguous remainder, flag it in the summary.

---

## T0 — scaffold + contracts

### Q-T0-1: `Supervisor::run(&mut self) -> !` return type vs. gate `cargo clippy -D warnings`

`run` is specified to "run forever" (`-> !`) and `step_once`/`run` bodies are
`unimplemented!("owned by T5")` in T0. `unimplemented!` panics, which makes
the `-> !` signature need a diverging tail after the macro. Resolved locally
without contract change (the macro's `!` type satisfies the return type), so
**no deviation** — logged only because the T5 implementer should keep the
body shape `unimplemented!("owned by T5")` as the final expression.

### Q-T0-2: `Cargo.toml` `[profile.release]` keys are not in the dep allowlist spirit?

Cargo profiles are toolchain config, not dependencies. Added `strip = true`
and `lto = true` in support of R11's binary < 2 MB requirement. Flagging for
planner awareness; no dependency added. If unwanted, delete the section —
nothing references it.

*(no blocking questions; T0 proceeded on the unambiguous remainder)*

---

## T1 — config

### Q-T1-1: semantics of a *present* config file with a missing key

Appendix A does not say what happens when `/etc/afanctl/afanctl.toml` exists
but omits one of the five keys. Two readings: (a) fill the missing key from its
documented default, or (b) refuse the file. **Chosen: (b) refuse** — every key
is required once a file is present; only a wholly *missing* file falls back to
`defaults()` (PRD R6 qualifies defaults with "missing file"). This is the
fail-safe reading for a thermal controller (a typo'd key can never silently
select a different curve) and matches the module contract "never run on defaults
after a bad edit". The unambiguous remainder is unaffected; no contract change.
`from_toml`/`load` return `ConfigError::Invalid { key: "thresholds.max", … }`
naming the missing key and the fix; a test locks it.

### N-T1-1 (note, not a question): `src/config.rs` LOC vs the §7 budget

§7 budgets `config.rs ~150`; this implementation is 231 non-comment product
lines (298 including doc comments and blanks) — ~54% over. The overage is the
mandated error surface itself: five keys × (missing / wrong-type / out-of-range)
plus the four `check_structural` rejections, each carrying `key` + `reason` +
`fix`, and the unknown-key warning walk (toml/serde derive cannot emit key+fix
messages, so the table is walked by hand). No functionality was trimmed; no
file other than `src/config.rs` grew. Flagged per §8 ("do not absorb silently")
for the orchestrator's LOC audit.
## T3 — smc

### Q-T3-1: smc.rs LOC vs §7 budget (~230)

Measured: 553 lines implementation + 334 lines co-located unit tests
(`#[cfg(test)]`, mandated by the card's "+ unit tests" and §8 conventions)
= 887 total. The ~230 budget (+20% → 276) is exceeded by the mandated
feature set: two backends (SysfsSmc + MockSmc with full fault-injection
surface per DESIGN's "scriptable ... faults, drift, latency"), discovery
walks (Q4/Q5-proof), K=3 write-verify on both writers, outlier window, and
the card-required layout hook. Reporting rather than absorbing silently;
Nothing was cut to fit, and no public item drifted to save lines.
## T6 — cli + main (PHASE a)

### Q-T6-1: runtime-dir override for `cmd.json` / `state.json`

R8 pins `cmd.json`/`state.json` at `/run/afanctl/`, but R5 defines only
`--config` and `--sysfs-root` as globals. Integration tests (T8) therefore have
no sanctioned way to redirect the runtime dir off the real `/run` (root-owned,
shared between tests). PHASE (a) reads `AFANCTL_RUNTIME_DIR` when set and
defaults to `/run/afanctl`. Question: is this env seam blessed for T5/T8, or
should a `--runtime-dir` global / explicit `RuntimePaths` injection be added?
No behavior beyond the default was invented; flagging the seam.

### Q-T6-2: deliberate PHASE-a deferrals

- `once --at-temp <C>` (simulated sensor input) and `--dry-run` are parsed and
  debug-logged but not yet applied — they need the real `StepReport` (PHASE b).
- `hold <rpm>` writes the raw rpm to `cmd.json` without R5's "clamp to hw range
  / reject below `fan1_min`" check, because the hardware min/max come from `smc`
  (T3, still a stub). The daemon re-validates/clamps (R8), so no unsafe write
  occurs; the CLI check lands in PHASE (b).
- `status` output (human + `afanctl.status.v1`) is formatted in full but cannot
  be verified against real data yet; PHASE (b) verifies the config + state file
  + direct-sysfs merge (R7).

### Q-T6-3: `cli.rs` LOC vs the §7 budget (~200)

`src/cli.rs` is ~650 lines (`wc -l`, tests included) at PHASE (a) scope: the
exact R5 parser + parser-table tests, all-verb dispatch, Appendix B/C
formatting, and exit-code discipline. §7 budgets cli.rs at ~200 (±20% = 240).
Flagged rather than silently absorbed (PLAN §2 rule). Options: (a) accept as
PHASE-a scope and let PHASE (b) refactor to fit; (b) split Appendix B/C
formatting into PHASE (b) or its own module. Awaiting the planner's ruling.
