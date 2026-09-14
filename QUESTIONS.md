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
