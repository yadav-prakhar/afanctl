<!-- Please read CONTRIBUTING.md before opening a PR. -->

## What

<!-- One behavior per PR. What does this change, and why? -->

## Safety checklist

<!-- Check all that apply; explain any unchecked box. -->

- [ ] `./check.sh` is fully green (`fmt`, `clippy -D warnings`, `test`, `hw` skips cleanly)
- [ ] No test writes to the real `/sys` (fixtures via `--sysfs-root` only)
- [ ] Every new failure path ends in AUTO / refuse-to-start / loud log — never silence
- [ ] Every state-changing write goes through write-verify
- [ ] No new dependencies (or `DEVIATIONS.md` entry + justification added)
- [ ] No contract changes (or recorded in `DEVIATIONS.md` and called out below)
- [ ] Docs updated (wiki page and/or `PRD.md` / `DESIGN.md` as appropriate)

## Contract changes

<!-- None, or: old → new → why → affected areas (must also be in DEVIATIONS.md). -->

```text

```

## Test evidence

<!-- Paste the gate output summary and describe the tests added. -->

```text

```
