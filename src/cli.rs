//! CLI verb dispatch, hand-rolled arg parsing, and output formatting
//! (PRD R5, R11). No clap — small verb set, exact grammar, nonzero exits.
//!
//! Grammar: verbs `daemon|status|doctor|once|observe|curve|hold|selftest-panic`,
//! globals `--config <path>`, `--sysfs-root <path>`; `--json` (status/doctor),
//! `--at-temp <C>` + `--dry-run` (once), `--mode observe|curve` (daemon),
//! `--roundtrip`, `--compare <s>` (doctor). Unknown flag → usage on stderr +
//! exit 2. Exit codes: 0 success, 1 runtime failure, 2 usage/CLI error.
//! This is the only file allowed `println!` (output formatting, §8).

/// Parse args, dispatch the verb, return the process exit code (0/1/2).
pub fn run() -> i32 {
    unimplemented!("owned by T6")
}
