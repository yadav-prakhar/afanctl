//! Crate root for `afanctl`: module wiring. Contracts live in `DESIGN.md`
//! (verbatim copy of PRD §7+§8) — signatures there are frozen for all tasks.

pub mod cli;
pub mod config;
pub mod doctor;
pub mod notify;
pub mod policy;
pub mod safety;
pub mod smc;
pub mod supervisor;
