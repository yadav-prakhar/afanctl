//! Wiring only: initialise logging, hand off to `cli::run`, and map its return
//! to the process exit code (0 success / 1 runtime failure / 2 usage; R11).
//!
//! Invariants: no logic here beyond wiring (§8); `unwrap`/`expect` are
//! permitted only in this file; this module never touches sysfs paths itself
//! (`smc` owns all path strings).

fn main() {
    init_tracing();
    std::process::exit(afanctl::cli::run());
}

/// Logging via `tracing` to stderr; journald captures it under systemd (R7).
/// A missing global subscriber must never abort a run, so the result is ignored.
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stderr)
        .try_init();
}
