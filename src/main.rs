//! Wiring only: parse args, init `tracing`, dispatch to the verb handler, map
//! errors to exit codes (0 success / 1 runtime failure / 2 usage).
//!
//! Invariants: no logic here beyond wiring; `unwrap`/`expect` permitted only
//! in this file (§8); never touches sysfs paths itself (owned by `smc`).

fn main() {
    // Wiring only; real dispatch is owned by T6.
    let code: i32 = stub_exit();
    std::process::exit(code);
}

/// Placeholder dispatcher until T6 lands the real one.
fn stub_exit() -> i32 {
    unimplemented!("owned by T6")
}
