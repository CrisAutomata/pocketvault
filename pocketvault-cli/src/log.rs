//! Small, consistently-styled result lines — the same ✔/⚠/✖ vocabulary is
//! shared by the menu UI, the full-screen command-line UI, and (indirectly)
//! `actions.rs`, so switching between front ends doesn't change how an
//! operation's outcome reads.
//!
//! The full-screen UI (`tui.rs`) owns the whole terminal via an alternate
//! screen — a stray `println!` there would corrupt the frame instead of
//! scrolling harmlessly. `suppress(true)` turns these into silent recorders;
//! `take_last()` is how the TUI pulls the message into its own status line.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use owo_colors::OwoColorize;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

static SUPPRESS_STDOUT: AtomicBool = AtomicBool::new(false);

thread_local! {
    static LAST: RefCell<Option<(Severity, String)>> = const { RefCell::new(None) };
}

pub fn suppress(enabled: bool) {
    SUPPRESS_STDOUT.store(enabled, Ordering::Relaxed);
}

/// Takes (clearing) whatever `ok`/`warn`/`fail` most recently recorded.
pub fn take_last() -> Option<(Severity, String)> {
    LAST.with(|cell| cell.borrow_mut().take())
}

fn record(sev: Severity, msg: &str) {
    LAST.with(|cell| *cell.borrow_mut() = Some((sev, msg.to_string())));
}

pub fn ok(msg: impl AsRef<str>) {
    record(Severity::Ok, msg.as_ref());
    if !SUPPRESS_STDOUT.load(Ordering::Relaxed) {
        println!("{} {}", "✔".green().bold(), msg.as_ref());
    }
}

pub fn warn(msg: impl AsRef<str>) {
    record(Severity::Warn, msg.as_ref());
    if !SUPPRESS_STDOUT.load(Ordering::Relaxed) {
        println!("{} {}", "⚠".yellow().bold(), msg.as_ref());
    }
}

pub fn fail(msg: impl AsRef<str>) {
    record(Severity::Fail, msg.as_ref());
    if !SUPPRESS_STDOUT.load(Ordering::Relaxed) {
        eprintln!("{} {}", "✖".red().bold(), msg.as_ref());
    }
}
