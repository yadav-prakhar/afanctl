# Security Policy

`afanctl` controls a physical fan. A bug here can strand hardware in a
manual/fan-off state — treat safety-relevant defects as security issues.

## Supported versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

Only the latest release is supported. Arch users on the AUR package
(`afanctl`) are covered; check `afanctl --version` and reinstall before
reporting.

## Reporting a vulnerability

**Do not open a public issue.** Report privately via a GitHub security
advisory:

- `https://github.com/yadav-prakhar/afanctl/security/advisories/new`
  (Security tab → Report a vulnerability), or
- open an issue titled `SECURITY: <short description>` **only if** advisories
  are unavailable — and put all details in the private advisory body, never in
  comments.

Include:

1. What you did (commands, config, kernel version, `sudo afanctl doctor`
   output if safe to collect).
2. What you expected vs. what happened — especially any state where the fan
   was left in Manual when it should have returned to AUTO.
3. Whether the L2 death path (`selftest-panic` → exit 101, `fan1_manual == 0`)
   still works on your machine.

You will get an initial response within 7 days. If confirmed, we will fix,
credit you in the advisory (unless you prefer to stay anonymous), and publish
the advisory after a fix is released. Please do not disclose publicly until
then.

## Scope notes

- The threat model is documented in the
  [Safety Model](https://github.com/yadav-prakhar/afanctl/wiki/Safety-Model)
  wiki page: L1 verify/re-assert, L2 death path, L3 systemd sandboxing.
- `doctor --roundtrip` intentionally writes to sysfs for 2 seconds — that is
  by design, not a vulnerability. Running `doctor` as non-root FAILs the
  write-mode checks by design (see the wiki
  [Diagnostics](https://github.com/yadav-prakhar/afanctl/wiki/Diagnostics-%28doctor%29)
  page).
