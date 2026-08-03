//! The plain-line prompts used only by the login flow (create/unlock a
//! vault) — that happens before either front end (menu or full-screen TUI)
//! takes over the terminal, so it stays simple: masked password reads via
//! `rpassword`, a `[y/N]` confirm.

use std::io::{self, BufRead, IsTerminal, Write};

pub fn read_password(prompt: &str) -> Option<String> {
    if io::stdin().is_terminal() {
        rpassword::prompt_password(prompt).ok()
    } else {
        read_stdin_line(prompt)
    }
}

pub fn confirm(message: &str) -> bool {
    match read_stdin_line(&format!("{message} [y/N] ")) {
        Some(answer) => matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"),
        None => false,
    }
}

fn read_stdin_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
        Err(_) => None,
    }
}
