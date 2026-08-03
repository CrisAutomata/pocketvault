//! The full-screen, nano-style command-line UI. Instead of a scrolling
//! transcript (type a command, print a new log line, repeat), the terminal
//! is taken over as an alternate screen and redrawn in place every tick:
//! a title bar, the current folder's listing (arrow-key navigable, with the
//! highlighted row's actions shown inline — `[Preview] [Export] [Delete]`),
//! a one-line status message, the command input, and a shortcut hint bar.
//!
//! Reachable from the menu UI's "Use command line", and returns to it via
//! the `menu` command, the always-present "[Menu]" row, or Ctrl-L/Ctrl-X.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};

use pocketvault_core::VaultError;

use crate::actions;
use crate::input::InputBox;
use crate::jobs::{self, CancelSlot, EncryptInput, ExportTarget, JobOutcome, RunningJob};
use crate::listing::{self, ItemKind, ListedItem};
use crate::log::{self, Severity};
use crate::session::CliSession;

pub enum TuiOutcome {
    UseMenu,
    Lock,
    Exit,
}

enum Body {
    Listing,
    Help,
}

enum Pending {
    ConfirmDelete(ListedItem),
    PasswordOld,
    PasswordNew {
        old: String,
    },
    PasswordConfirm {
        old: String,
        new: String,
    },
    /// The `[x]`/`[ ]` action list opened by pressing Enter on a highlighted
    /// row — Up/Down move `selected`, Enter activates it, Esc cancels.
    RowActions {
        item: ListedItem,
        options: &'static [&'static str],
        selected: usize,
    },
    ExportDest {
        item: ListedItem,
    },
    RenameInput {
        folder_id: String,
    },
}

/// One entry in the arrow-navigable listing: the two control rows are always
/// present (barring "Up" at the vault root), items are whatever's in the
/// current folder.
enum Row {
    Up,
    Item(usize),
    BackToMenu,
}

struct App<'a> {
    session: &'a mut CliSession,
    cancel_slot: &'a CancelSlot,
    input: InputBox,
    body: Body,
    status: (Severity, String),
    job: Option<RunningJob>,
    pending: Option<Pending>,
    /// Index into `build_rows(app)` — which row is highlighted.
    selected: usize,
    frame: u64,
    outcome: Option<TuiOutcome>,
}

const COMMANDS: &[&str] = &[
    "ls", "cd", "pwd", "mkdir", "rename", "rm", "encrypt", "export", "cat", "passwd", "menu",
    "lock", "exit", "help",
];

const HELP_TEXT: &str = "\
ls                        Refresh the listing
cd <name>|..|/            Change folder ('..' = up, '/' = root)
mkdir <name>              Create a folder here
rename <folder> <name>    Rename a folder
rm <name> [-f]            Delete a file or folder (asks to confirm)
encrypt <path> [...]      Encrypt file(s), or one folder, into the vault here
export <name> <dest>      Decrypt a file or folder out to disk
cat <name>                Preview a file in your OS's default app for it
passwd                    Change the master password
menu                      Switch to the menu-driven UI
lock / exit               Lock the vault / leave PocketVault

Up/Down highlights a row (when the input is empty); Enter opens that row's
actions — [Preview] [Export] [Delete] for files, [Open] [Export] [Rename]
[Delete] for folders. [..] and [Menu] are always available as rows too.
Tab completes the command name, a vault item's name, or a filesystem path.
Esc backs out of a preview/help screen or an action list.
Ctrl-C cancels an in-progress encrypt/export. Ctrl-L locks. Ctrl-X exits.";

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        log::suppress(true);
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        log::suppress(false);
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

pub fn run(session: &mut CliSession, cancel_slot: &CancelSlot) -> TuiOutcome {
    let _guard = match TerminalGuard::enter() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Couldn't start the command-line UI: {e}");
            return TuiOutcome::UseMenu;
        }
    };

    let mut terminal = match Terminal::new(CrosstermBackend::new(io::stdout())) {
        Ok(t) => t,
        Err(_) => return TuiOutcome::UseMenu,
    };

    let mut app = App {
        session,
        cancel_slot,
        input: InputBox::new(),
        body: Body::Listing,
        status: (
            Severity::Ok,
            "↑/↓ to highlight a row, Enter to act on it — or type a command. `help` lists them."
                .to_string(),
        ),
        job: None,
        pending: None,
        selected: 0,
        frame: 0,
        outcome: None,
    };
    refresh_listing(&mut app);

    loop {
        if let Some(outcome) = app.job.as_ref().and_then(RunningJob::poll) {
            let job = app.job.take().expect("polled job must exist");
            jobs::finish_job(job, app.cancel_slot);
            apply_job_outcome(&mut app, outcome);
        }

        let _ = terminal.draw(|f| draw(f, &app));

        // Drain every event already waiting, not just one — a burst of
        // characters (fast typing, or pasted text) can arrive in a single
        // readable chunk; polling again with a zero timeout after each read
        // catches the rest instead of leaving them queued for a later,
        // unrelated wakeup.
        if event::poll(Duration::from_millis(80)).unwrap_or(false) {
            loop {
                match event::read() {
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                        handle_key(&mut app, key)
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
                if app.outcome.is_some() || !event::poll(Duration::from_millis(0)).unwrap_or(false)
                {
                    break;
                }
            }
        }
        app.frame = app.frame.wrapping_add(1);

        if let Some(outcome) = app.outcome.take() {
            return outcome;
        }
    }
}

// ── Rows: the arrow-navigable listing ────────────────────────────────────

/// `[..]` (if not at the vault root) + every item in the current folder +
/// `[Menu]` — always in that order, so index 0 is always either "Up" or the
/// first item, and the last index is always "back to menu".
fn build_rows(app: &App) -> Vec<Row> {
    let mut rows = Vec::new();
    if app.session.current_folder_id.is_some() {
        rows.push(Row::Up);
    }
    for i in 0..app.session.last_listing.len() {
        rows.push(Row::Item(i));
    }
    rows.push(Row::BackToMenu);
    rows
}

fn move_selection(app: &mut App, delta: i32) {
    let len = build_rows(app).len();
    if len == 0 {
        return;
    }
    let current = app.selected as i32;
    app.selected = (current + delta).clamp(0, len as i32 - 1) as usize;
}

fn activate_selected_row(app: &mut App) {
    let rows = build_rows(app);
    if rows.is_empty() {
        return;
    }
    let idx = app.selected.min(rows.len() - 1);
    match &rows[idx] {
        Row::Up => go_up(app),
        Row::BackToMenu => app.outcome = Some(TuiOutcome::UseMenu),
        Row::Item(item_idx) => {
            let item = app.session.last_listing[*item_idx].clone();
            let options: &'static [&'static str] = match item.kind {
                ItemKind::Folder => &["Open", "Export", "Rename", "Delete", "Cancel"],
                ItemKind::File => &["Preview", "Export", "Delete", "Cancel"],
            };
            app.pending = Some(Pending::RowActions {
                item,
                options,
                selected: 0,
            });
        }
    }
}

fn run_row_action(app: &mut App, item: ListedItem, choice: &str) {
    match choice {
        "Open" => navigate_to(app, Some(item.id.clone())),
        "Preview" => {
            actions::preview_item(app.session, &item);
            apply_log(app);
        }
        "Export" => app.pending = Some(Pending::ExportDest { item }),
        "Rename" => {
            app.pending = Some(Pending::RenameInput {
                folder_id: item.id.clone(),
            })
        }
        "Delete" => {
            app.status = (
                Severity::Warn,
                format!("Delete '{}'? Type y and press Enter to confirm.", item.name),
            );
            app.pending = Some(Pending::ConfirmDelete(item));
        }
        _ => {} // "Cancel"
    }
}

fn navigate_to(app: &mut App, folder_id: Option<String>) {
    app.session.current_folder_id = folder_id;
    refresh_listing(app);
    app.selected = 0;
}

fn go_up(app: &mut App) {
    let Some(id) = app.session.current_folder_id.clone() else {
        return;
    };
    let chain = app.session.vault.meta.folder_path(&id);
    let parent = if chain.len() >= 2 {
        Some(chain[chain.len() - 2].id.clone())
    } else {
        None
    };
    navigate_to(app, parent);
}

// ── Rendering ────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_title(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_status(f, chunks[2], app);
    draw_input(f, chunks[3], app);
    draw_shortcuts(f, chunks[4]);
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let text = format!(" PocketVault — vault:{}", listing::path_string(app.session));
    let style = Style::default()
        .bg(Color::Cyan)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);
    f.render_widget(Paragraph::new(text).style(style), area);
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();

    if let Some(job) = &app.job {
        lines.push(Line::from(job_progress_text(app, job)));
        lines.push(Line::from(""));
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    if let Some(Pending::RowActions {
        item,
        options,
        selected,
    }) = &app.pending
    {
        lines.push(Line::from(format!(
            "Actions for '{}' — Enter to run, Esc to cancel:",
            item.name
        )));
        lines.push(Line::from(""));
        for (i, opt) in options.iter().enumerate() {
            let marker = if i == *selected { "[x]" } else { "[ ]" };
            let line = Line::from(format!("  {marker} {opt}"));
            lines.push(if i == *selected {
                line.style(Style::default().add_modifier(Modifier::BOLD))
            } else {
                line
            });
        }
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    match &app.body {
        Body::Listing => {
            let rows = build_rows(app);
            let selected = app.selected.min(rows.len().saturating_sub(1));
            for (i, row) in rows.iter().enumerate() {
                let highlighted = i == selected;
                let text = row_text(app, row, highlighted);
                lines.push(if highlighted {
                    Line::from(text).style(Style::default().add_modifier(Modifier::REVERSED))
                } else {
                    Line::from(text)
                });
            }
        }
        Body::Help => lines.extend(HELP_TEXT.lines().map(|l| Line::from(l.to_string()))),
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn row_text(app: &App, row: &Row, highlighted: bool) -> String {
    match row {
        Row::Up => "[..]  Up one level".to_string(),
        Row::BackToMenu => "[Menu]  Leave the command line".to_string(),
        Row::Item(idx) => {
            let item = &app.session.last_listing[*idx];
            let base = listing::item_label(&app.session.vault, &app.session.key, item);
            if !highlighted {
                return base;
            }
            let buttons = match item.kind {
                ItemKind::Folder => "[Open] [Export] [Rename] [Delete]",
                ItemKind::File => "[Preview] [Export] [Delete]",
            };
            format!("{base}  {buttons}")
        }
    }
}

fn job_progress_text(app: &App, job: &RunningJob) -> String {
    let spin = SPINNER[app.frame as usize % SPINNER.len()];
    if job.big {
        let done = job
            .control
            .bytes_done
            .load(Ordering::Relaxed)
            .min(job.total_bytes);
        let bar_width = 24usize;
        let filled = if job.total_bytes == 0 {
            0
        } else {
            (done as usize * bar_width) / job.total_bytes as usize
        };
        format!(
            "{spin} {} {} — [{}{}] {}/{} ({}) — Ctrl-C to cancel",
            job.verb,
            job.label,
            "=".repeat(filled),
            " ".repeat(bar_width - filled),
            listing::format_size(done),
            listing::format_size(job.total_bytes),
            eta_text(job, done),
        )
    } else {
        format!("{spin} {} {}…", job.verb, job.label)
    }
}

/// "~Ns left" from a simple whole-job-so-far throughput estimate — good
/// enough for a live status line, unlike the desktop's frozen-after-warmup
/// estimate (see `pocketvault-desktop/src/state.rs::freeze_eta_if_ready`),
/// which exists there to stop a redraw-every-frame UI from visibly jittering.
fn eta_text(job: &RunningJob, done: u64) -> String {
    let elapsed = job.started_at.elapsed().as_secs_f64();
    if done == 0 || elapsed < 0.2 {
        return "estimating…".to_string();
    }
    let rate = done as f64 / elapsed;
    let remaining_secs = ((job.total_bytes - done) as f64 / rate.max(1.0)).round() as u64;
    if remaining_secs == 0 {
        "finishing up…".to_string()
    } else if remaining_secs < 60 {
        format!("~{remaining_secs}s left")
    } else {
        format!("~{}m {}s left", remaining_secs / 60, remaining_secs % 60)
    }
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let (severity, message) = &app.status;
    let color = match severity {
        Severity::Ok => Color::Green,
        Severity::Warn => Color::Yellow,
        Severity::Fail => Color::Red,
    };
    f.render_widget(
        Paragraph::new(message.clone()).style(Style::default().fg(color)),
        area,
    );
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let label = match &app.pending {
        Some(Pending::ConfirmDelete(item)) => format!("Delete '{}'? [y/N] ", item.name),
        Some(Pending::PasswordOld) => "Current password: ".to_string(),
        Some(Pending::PasswordNew { .. }) => "New password (min 8 chars): ".to_string(),
        Some(Pending::PasswordConfirm { .. }) => "Confirm new password: ".to_string(),
        Some(Pending::ExportDest { item }) => {
            format!("Export '{}' to which folder on disk? ", item.name)
        }
        Some(Pending::RenameInput { .. }) => "New name: ".to_string(),
        Some(Pending::RowActions { .. }) | None => {
            format!("vault:{}> ", listing::path_string(app.session))
        }
    };
    let text = format!("{label}{}", app.input.display());
    f.render_widget(Paragraph::new(text), area);

    let cursor_x = area.x + (label.chars().count() + app.input.cursor()) as u16;
    f.set_cursor_position((cursor_x.min(area.x + area.width.saturating_sub(1)), area.y));
}

fn draw_shortcuts(f: &mut Frame, area: Rect) {
    let text = " ↑/↓ Navigate   Enter Select/Run   Tab Complete   Ctrl-C Cancel Job   Ctrl-L Lock   Ctrl-X Exit   Esc Back ";
    f.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::Blue).fg(Color::White)),
        area,
    );
}

// ── Input handling ───────────────────────────────────────────────────────

fn handle_key(app: &mut App, key: KeyEvent) {
    if app.job.is_some() {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if let Some(job) = &app.job {
                job.control.cancel.store(true, Ordering::Relaxed);
            }
        }
        return; // everything else waits for the job to finish
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('x') => {
                app.outcome = Some(TuiOutcome::Exit);
                return;
            }
            KeyCode::Char('l') => {
                app.outcome = Some(TuiOutcome::Lock);
                return;
            }
            _ => {}
        }
    }

    if matches!(app.pending, Some(Pending::RowActions { .. })) {
        handle_row_actions_key(app, key);
        return;
    }

    match key.code {
        KeyCode::Char(c) => app.input.insert(c),
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left => app.input.left(),
        KeyCode::Right => app.input.right(),
        KeyCode::Home => app.input.home(),
        KeyCode::End => app.input.end(),
        KeyCode::Tab => tab_complete(app),
        KeyCode::Up => {
            if app.input.value().is_empty() {
                move_selection(app, -1);
            } else {
                app.input.history_prev();
            }
        }
        KeyCode::Down => {
            if app.input.value().is_empty() {
                move_selection(app, 1);
            } else {
                app.input.history_next();
            }
        }
        KeyCode::Esc => {
            app.body = Body::Listing;
            app.pending = None;
            app.input.set_masked(false);
            app.input.clear();
        }
        KeyCode::Enter => {
            if app.input.value().is_empty() && app.pending.is_none() {
                activate_selected_row(app);
            } else {
                submit(app);
            }
        }
        _ => {}
    }
}

fn handle_row_actions_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up => {
            if let Some(Pending::RowActions { selected, .. }) = &mut app.pending {
                *selected = selected.saturating_sub(1);
            }
        }
        KeyCode::Down => {
            if let Some(Pending::RowActions {
                options, selected, ..
            }) = &mut app.pending
            {
                *selected = (*selected + 1).min(options.len().saturating_sub(1));
            }
        }
        KeyCode::Esc => app.pending = None,
        KeyCode::Enter => {
            if let Some(Pending::RowActions {
                item,
                options,
                selected,
            }) = &app.pending
            {
                let item = item.clone();
                let choice = options[*selected];
                app.pending = None;
                run_row_action(app, item, choice);
            }
        }
        _ => {}
    }
}

fn submit(app: &mut App) {
    let answer = app.input.submit();

    if let Some(pending) = app.pending.take() {
        handle_pending(app, pending, answer);
        return;
    }

    let line = answer.trim().to_string();
    if line.is_empty() {
        return;
    }
    run_command(app, &line);
}

fn handle_pending(app: &mut App, pending: Pending, answer: String) {
    match pending {
        Pending::ConfirmDelete(item) => {
            if matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
                actions::delete_item(app.session, &item);
                apply_log(app);
                refresh_listing(app);
            } else {
                app.status = (Severity::Warn, "Cancelled.".to_string());
            }
        }
        Pending::PasswordOld => {
            app.pending = Some(Pending::PasswordNew { old: answer });
        }
        Pending::PasswordNew { old } => {
            if answer.len() < 8 {
                app.status = (
                    Severity::Fail,
                    "New password must be at least 8 characters — try again.".to_string(),
                );
                app.pending = Some(Pending::PasswordNew { old });
                return;
            }
            app.pending = Some(Pending::PasswordConfirm { old, new: answer });
        }
        Pending::PasswordConfirm { old, new } => {
            if answer != new {
                app.status = (
                    Severity::Fail,
                    "Passwords don't match — confirm again.".to_string(),
                );
                app.pending = Some(Pending::PasswordConfirm { old, new });
                return;
            }
            app.input.set_masked(false);
            actions::change_password(app.session, &old, &new);
            apply_log(app);
        }
        Pending::ExportDest { item } => {
            let dest = answer.trim();
            if dest.is_empty() {
                app.status = (Severity::Warn, "Cancelled.".to_string());
                return;
            }
            start_export_for_item(app, &item, PathBuf::from(dest));
        }
        Pending::RenameInput { folder_id } => {
            let new_name = answer.trim();
            if new_name.is_empty() {
                app.status = (Severity::Warn, "Cancelled.".to_string());
                return;
            }
            actions::rename_folder(app.session, &folder_id, new_name);
            apply_log(app);
            refresh_listing(app);
        }
        Pending::RowActions { .. } => {
            unreachable!("RowActions is handled by handle_row_actions_key, not submit()")
        }
    }
}

fn apply_log(app: &mut App) {
    if let Some((severity, message)) = log::take_last() {
        app.status = (severity, message);
    }
}

fn refresh_listing(app: &mut App) {
    app.session.last_listing = listing::build_listing(
        &app.session.vault,
        &app.session.key,
        app.session.current_folder_id.as_deref(),
    );
    let len = build_rows(app).len();
    if app.selected >= len {
        app.selected = len.saturating_sub(1);
    }
}

fn apply_job_outcome(app: &mut App, outcome: JobOutcome) {
    match outcome {
        JobOutcome::Encrypt(vault, result) => {
            app.session.vault = vault;
            app.status = match result {
                Ok(ids) => (
                    Severity::Ok,
                    format!(
                        "Encrypted {} file{} into the vault.",
                        ids.len(),
                        if ids.len() == 1 { "" } else { "s" }
                    ),
                ),
                Err(VaultError::Cancelled) => {
                    (Severity::Warn, "Cancelled — nothing was added.".to_string())
                }
                Err(e) => (Severity::Fail, format!("Encrypt failed: {e}")),
            };
        }
        JobOutcome::Export(result) => {
            app.status = match result {
                Ok(path) => (Severity::Ok, format!("Exported to {}", path.display())),
                Err(VaultError::Cancelled) => (
                    Severity::Warn,
                    "Cancelled — no partial file left behind.".to_string(),
                ),
                Err(e) => (Severity::Fail, format!("Export failed: {e}")),
            };
        }
    }
    refresh_listing(app);
}

// ── Commands ─────────────────────────────────────────────────────────────

fn run_command(app: &mut App, line: &str) {
    let args = match shell_words::split(line) {
        Ok(a) => a,
        Err(e) => {
            app.status = (Severity::Fail, format!("Couldn't parse that: {e}"));
            return;
        }
    };
    let Some((cmd, rest)) = args.split_first() else {
        return;
    };

    app.body = Body::Listing;

    match cmd.as_str() {
        "ls" | "dir" => {
            refresh_listing(app);
            app.status = (Severity::Ok, "Listing refreshed.".to_string());
        }
        "cd" => cmd_cd(app, rest),
        "pwd" => app.status = (Severity::Ok, listing::path_string(app.session)),
        "mkdir" => cmd_mkdir(app, rest),
        "rename" | "mv" => cmd_rename(app, rest),
        "rm" | "del" => cmd_rm(app, rest),
        "encrypt" | "add" | "import" => cmd_encrypt(app, rest),
        "export" => cmd_export(app, rest),
        "cat" | "preview" | "view" => cmd_cat(app, rest),
        "passwd" | "change-password" => {
            app.pending = Some(Pending::PasswordOld);
            app.input.set_masked(true);
            app.status = (Severity::Ok, "Changing master password.".to_string());
        }
        "help" | "?" => app.body = Body::Help,
        "lock" => app.outcome = Some(TuiOutcome::Lock),
        "menu" | "ui" => app.outcome = Some(TuiOutcome::UseMenu),
        "exit" | "quit" | "q" => app.outcome = Some(TuiOutcome::Exit),
        other => {
            app.status = (
                Severity::Fail,
                format!("Unknown command: '{other}' (try `help`)"),
            )
        }
    }
}

fn cmd_cd(app: &mut App, rest: &[String]) {
    let target = rest.first().map(String::as_str).unwrap_or("/");

    if target == "/" {
        navigate_to(app, None);
    } else if target == "." {
        // no-op
    } else if target == ".." {
        go_up(app);
    } else {
        refresh_listing(app);
        match listing::resolve(&app.session.last_listing, target) {
            Ok(item) if item.kind == ItemKind::Folder => navigate_to(app, Some(item.id.clone())),
            Ok(_) => {
                app.status = (
                    Severity::Fail,
                    format!("'{target}' is a file, not a folder."),
                );
                return;
            }
            Err(e) => {
                app.status = (Severity::Fail, e);
                return;
            }
        }
    }

    app.status = (
        Severity::Ok,
        format!("Now in {}", listing::path_string(app.session)),
    );
}

fn cmd_mkdir(app: &mut App, rest: &[String]) {
    let Some(name) = rest.first() else {
        app.status = (Severity::Fail, "Usage: mkdir <name>".to_string());
        return;
    };
    actions::create_folder(app.session, name);
    apply_log(app);
    refresh_listing(app);
}

fn cmd_rename(app: &mut App, rest: &[String]) {
    if rest.len() < 2 {
        app.status = (
            Severity::Fail,
            "Usage: rename <folder> <new-name>".to_string(),
        );
        return;
    }
    refresh_listing(app);
    let folder_id = match listing::resolve(&app.session.last_listing, &rest[0]) {
        Ok(item) if item.kind == ItemKind::Folder => item.id.clone(),
        Ok(_) => {
            app.status = (Severity::Fail, "Only folders can be renamed.".to_string());
            return;
        }
        Err(e) => {
            app.status = (Severity::Fail, e);
            return;
        }
    };
    actions::rename_folder(app.session, &folder_id, &rest[1]);
    apply_log(app);
    refresh_listing(app);
}

fn cmd_rm(app: &mut App, rest: &[String]) {
    let force = rest.iter().any(|a| a == "-f" || a == "--force");
    let Some(name) = rest.iter().find(|a| !a.starts_with('-')) else {
        app.status = (Severity::Fail, "Usage: rm <name> [-f]".to_string());
        return;
    };

    refresh_listing(app);
    let item = match listing::resolve(&app.session.last_listing, name) {
        Ok(item) => item.clone(),
        Err(e) => {
            app.status = (Severity::Fail, e);
            return;
        }
    };

    if force {
        actions::delete_item(app.session, &item);
        apply_log(app);
        refresh_listing(app);
    } else {
        app.status = (
            Severity::Warn,
            format!("Delete '{}'? Type y and press Enter to confirm.", item.name),
        );
        app.pending = Some(Pending::ConfirmDelete(item));
    }
}

fn cmd_encrypt(app: &mut App, rest: &[String]) {
    if rest.is_empty() {
        app.status = (
            Severity::Fail,
            "Usage: encrypt <path> [<path> ...]   (a single folder path encrypts recursively)"
                .to_string(),
        );
        return;
    }

    if rest.len() == 1 {
        let path = PathBuf::from(&rest[0]);
        if !path.exists() {
            app.status = (
                Severity::Fail,
                format!("No such file or folder: {}", path.display()),
            );
            return;
        }
        if path.is_dir() {
            start_encrypt_folder(app, path);
        } else {
            start_encrypt_files(app, vec![path]);
        }
        return;
    }

    let mut paths = Vec::with_capacity(rest.len());
    for raw in rest {
        let path = PathBuf::from(raw);
        if !path.exists() {
            app.status = (Severity::Fail, format!("No such file: {}", path.display()));
            return;
        }
        if path.is_dir() {
            app.status = (Severity::Fail, "Encrypt one folder at a time.".to_string());
            return;
        }
        paths.push(path);
    }
    start_encrypt_files(app, paths);
}

fn start_encrypt_files(app: &mut App, paths: Vec<PathBuf>) {
    let dest_folder_id = app.session.current_folder_id.clone();
    let total_bytes: u64 = paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    let label = if paths.len() == 1 {
        paths[0]
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into())
    } else {
        format!("{} files", paths.len())
    };

    let vault = app.session.vault.clone();
    let key = app.session.key.clone();
    let job = jobs::start_encrypt(
        vault,
        key,
        EncryptInput::Files {
            paths,
            dest_folder_id,
        },
        total_bytes,
        label.clone(),
        app.cancel_slot,
    );
    app.job = Some(job);
    app.status = (Severity::Ok, format!("Encrypting {label}…"));
}

fn start_encrypt_folder(app: &mut App, path: PathBuf) {
    let dest_folder_id = app.session.current_folder_id.clone();
    let total_bytes = pocketvault_core::dir_total_size(&path).unwrap_or(0);
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "folder".into());

    let vault = app.session.vault.clone();
    let key = app.session.key.clone();
    let job = jobs::start_encrypt(
        vault,
        key,
        EncryptInput::Folder {
            path,
            dest_folder_id,
        },
        total_bytes,
        label.clone(),
        app.cancel_slot,
    );
    app.job = Some(job);
    app.status = (Severity::Ok, format!("Encrypting {label}…"));
}

fn cmd_export(app: &mut App, rest: &[String]) {
    if rest.len() < 2 {
        app.status = (
            Severity::Fail,
            "Usage: export <name> <destination-folder>".to_string(),
        );
        return;
    }

    refresh_listing(app);
    let item = match listing::resolve(&app.session.last_listing, &rest[0]) {
        Ok(item) => item.clone(),
        Err(e) => {
            app.status = (Severity::Fail, e);
            return;
        }
    };

    start_export_for_item(app, &item, PathBuf::from(&rest[1]));
}

/// Shared by the typed `export` command and the row-action `[Export]`
/// button — both just need to resolve an item and a destination, then kick
/// off the same background job.
fn start_export_for_item(app: &mut App, item: &ListedItem, dest_dir: PathBuf) {
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        app.status = (
            Severity::Fail,
            format!("Can't create destination folder: {e}"),
        );
        return;
    }

    let (target, total_bytes) = match item.kind {
        ItemKind::File => {
            let size = app
                .session
                .vault
                .read_metadata(&item.id, &app.session.key)
                .map(|m| m.original_size)
                .unwrap_or(0);
            (
                ExportTarget::File {
                    file_id: item.id.clone(),
                    dest_dir,
                },
                size,
            )
        }
        ItemKind::Folder => {
            let size = listing::folder_total_size(&app.session.vault, &app.session.key, &item.id);
            (
                ExportTarget::Folder {
                    folder_id: item.id.clone(),
                    dest_dir,
                },
                size,
            )
        }
    };

    let vault = app.session.vault.clone();
    let key = app.session.key.clone();
    let job = jobs::start_export(
        vault,
        key,
        target,
        total_bytes,
        item.name.clone(),
        app.cancel_slot,
    );
    app.job = Some(job);
    app.status = (Severity::Ok, format!("Exporting {}…", item.name));
}

fn cmd_cat(app: &mut App, rest: &[String]) {
    let Some(name) = rest.first() else {
        app.status = (Severity::Fail, "Usage: cat <name>".to_string());
        return;
    };

    refresh_listing(app);
    let item = match listing::resolve(&app.session.last_listing, name) {
        Ok(item) if item.kind == ItemKind::File => item.clone(),
        Ok(_) => {
            app.status = (Severity::Fail, "That's a folder.".to_string());
            return;
        }
        Err(e) => {
            app.status = (Severity::Fail, e);
            return;
        }
    };

    actions::preview_item(app.session, &item);
    apply_log(app);
}

// ── Tab completion ───────────────────────────────────────────────────────

fn tab_complete(app: &mut App) {
    let (start, word) = app.input.word_before_cursor();

    let candidates = match &app.pending {
        Some(Pending::ExportDest { .. }) => filesystem_candidates(&word),
        Some(_) => Vec::new(),
        None => {
            let before = app.input.value_before(start);
            let args_before: Vec<&str> = before.split_whitespace().collect();
            if args_before.is_empty() {
                COMMANDS
                    .iter()
                    .filter(|c| c.starts_with(&word))
                    .map(|s| s.to_string())
                    .collect()
            } else {
                match args_before[0] {
                    "cd" => vault_name_candidates(app, &word, Some(ItemKind::Folder)),
                    "rename" if args_before.len() == 1 => {
                        vault_name_candidates(app, &word, Some(ItemKind::Folder))
                    }
                    "rm" | "del" | "cat" | "preview" | "view" => {
                        vault_name_candidates(app, &word, None)
                    }
                    "export" if args_before.len() == 1 => vault_name_candidates(app, &word, None),
                    "export" | "encrypt" | "add" | "import" => filesystem_candidates(&word),
                    _ => Vec::new(),
                }
            }
        }
    };

    apply_completion(app, start, &word, candidates);
}

fn vault_name_candidates(app: &App, prefix: &str, kind: Option<ItemKind>) -> Vec<String> {
    app.session
        .last_listing
        .iter()
        .filter(|item| kind.is_none_or(|k| item.kind == k))
        .filter(|item| item.name.starts_with(prefix))
        .map(|item| item.name.clone())
        .collect()
}

fn filesystem_candidates(partial: &str) -> Vec<String> {
    let (dir_part, file_prefix) = match partial.rsplit_once('/') {
        Some((dir, file)) => (format!("{dir}/"), file.to_string()),
        None => (String::new(), partial.to_string()),
    };
    let read_dir_path = if dir_part.is_empty() {
        "."
    } else {
        dir_part.as_str()
    };
    let Ok(entries) = std::fs::read_dir(read_dir_path) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(&file_prefix) {
            continue;
        }
        let is_dir = entry.path().is_dir();
        out.push(format!("{dir_part}{name}{}", if is_dir { "/" } else { "" }));
    }
    out.sort();
    out
}

fn apply_completion(app: &mut App, start: usize, word: &str, mut candidates: Vec<String>) {
    candidates.sort();
    candidates.dedup();
    match candidates.len() {
        0 => {}
        1 => app.input.replace_word_before_cursor(start, &candidates[0]),
        _ => {
            let common = longest_common_prefix(&candidates);
            if common.len() > word.len() {
                app.input.replace_word_before_cursor(start, &common);
            }
            let preview = candidates
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join("  ");
            app.status = (
                Severity::Ok,
                format!("{} matches: {preview}", candidates.len()),
            );
        }
    }
}

fn longest_common_prefix(strings: &[String]) -> String {
    let Some(first) = strings.first() else {
        return String::new();
    };
    let mut prefix = first.clone();
    for s in &strings[1..] {
        let common_len = prefix
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let byte_len = prefix
            .char_indices()
            .nth(common_len)
            .map(|(i, _)| i)
            .unwrap_or(prefix.len());
        prefix.truncate(byte_len);
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn longest_common_prefix_of_one_string_is_itself() {
        let strings = vec!["Documents".to_string()];
        assert_eq!(longest_common_prefix(&strings), "Documents");
    }

    #[test]
    fn longest_common_prefix_across_several() {
        let strings = vec![
            "Downloads".to_string(),
            "Documents".to_string(),
            "Docs".to_string(),
        ];
        assert_eq!(longest_common_prefix(&strings), "Do");
    }

    #[test]
    fn longest_common_prefix_none_in_common() {
        let strings = vec!["Photos".to_string(), "Videos".to_string()];
        assert_eq!(longest_common_prefix(&strings), "");
    }

    #[test]
    fn longest_common_prefix_of_empty_list() {
        let strings: Vec<String> = Vec::new();
        assert_eq!(longest_common_prefix(&strings), "");
    }

    #[test]
    fn filesystem_candidates_lists_matching_entries_in_cwd() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("astro.svg"), b"").unwrap();
        fs::write(dir.path().join("app.tsx"), b"").unwrap();
        fs::create_dir(dir.path().join("assets")).unwrap();
        fs::write(dir.path().join("other.txt"), b"").unwrap();

        let prefix = dir.path().join("a").to_string_lossy().to_string();
        let mut candidates = filesystem_candidates(&prefix);
        candidates.sort();

        let names: Vec<String> = candidates
            .iter()
            .map(|c| {
                Path::new(c)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["app.tsx", "assets", "astro.svg"]);

        // directories get a trailing slash so completion can keep descending
        let assets_entry = candidates.iter().find(|c| c.contains("assets")).unwrap();
        assert!(
            assets_entry.ends_with('/'),
            "directory candidate should end with '/': {assets_entry}"
        );
    }

    #[test]
    fn filesystem_candidates_on_missing_dir_is_empty() {
        assert!(filesystem_candidates("/no/such/dir/at/all/xyz").is_empty());
    }
}
