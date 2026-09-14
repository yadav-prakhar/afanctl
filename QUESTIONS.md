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
