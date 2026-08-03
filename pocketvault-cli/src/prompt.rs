//! Line/password/confirm input, tolerant of both a real TTY (full line
//! editing + history via rustyline, masked passwords via rpassword) and a
//! piped stdin (plain line reads) — the latter matters for scripting the CLI
//! and for automated testing, neither of which has a pty attached.

use std::io::{self, BufRead, IsTerminal, Write};

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

pub enum Prompter {
    Interactive(DefaultEditor),
    Piped,
}

impl Prompter {
    pub fn new() -> Self {
        if io::stdin().is_terminal() {
            if let Ok(editor) = DefaultEditor::new() {
                return Prompter::Interactive(editor);
            }
        }
        Prompter::Piped
    }

    /// Reads one line. `None` means EOF (Ctrl-D, or a piped stream running
    /// out) — the caller should exit the session. A Ctrl-C at an empty
    /// prompt just redraws it (returned as `Some(String::new())`), matching
    /// the "cancel this input, don't quit the session" feel of a real shell.
    pub fn read_line(&mut self, prompt: &str) -> Option<String> {
        match self {
            Prompter::Interactive(editor) => match editor.readline(prompt) {
                Ok(line) => {
                    let _ = editor.add_history_entry(line.as_str());
                    Some(line)
                }
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    Some(String::new())
                }
                Err(_) => None,
            },
            Prompter::Piped => read_stdin_line(prompt),
        }
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

/// Reads a password, masked on a real TTY, plain on a pipe. `None` means EOF.
pub fn read_password(prompt: &str) -> Option<String> {
    if io::stdin().is_terminal() {
        rpassword::prompt_password(prompt).ok()
    } else {
        read_stdin_line(prompt)
    }
}

/// `[y/N]` confirmation — defaults to no on anything but an explicit yes.
pub fn confirm(message: &str) -> bool {
    match read_stdin_line(&format!("{message} [y/N] ")) {
        Some(answer) => matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"),
        None => false,
    }
}
