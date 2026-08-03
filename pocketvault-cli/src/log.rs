//! Small, consistently-styled result lines — the same ✔/⚠/✖ vocabulary is
//! used by both the menu UI and the command-line mode, so switching between
//! them (via `menu` / `Use command line`) doesn't change how outcomes read.

use owo_colors::OwoColorize;

pub fn ok(msg: impl AsRef<str>) {
    println!("{} {}", "✔".green().bold(), msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    println!("{} {}", "⚠".yellow().bold(), msg.as_ref());
}

pub fn fail(msg: impl AsRef<str>) {
    eprintln!("{} {}", "✖".red().bold(), msg.as_ref());
}
