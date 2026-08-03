//! A single-line, cursor-aware input buffer for the TUI's bottom command
//! bar — the same editing gestures `rustyline` gave the old scrolling REPL
//! (left/right, backspace/delete, history), but rendered as one fixed line
//! inside a full-screen frame instead of appended to a scrolling terminal.

pub struct InputBox {
    value: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_idx: Option<usize>,
    masked: bool,
}

impl InputBox {
    pub fn new() -> Self {
        Self {
            value: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            history_idx: None,
            masked: false,
        }
    }

    pub fn set_masked(&mut self, masked: bool) {
        self.masked = masked;
    }

    pub fn value(&self) -> String {
        self.value.iter().collect()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// What to actually draw — asterisks while masked (password entry).
    pub fn display(&self) -> String {
        if self.masked {
            "*".repeat(self.value.len())
        } else {
            self.value()
        }
    }

    pub fn insert(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.value.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.value.len() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.len();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// The contiguous non-whitespace run ending at the cursor, and its
    /// starting char index — the "word" Tab-completion expands.
    pub fn word_before_cursor(&self) -> (usize, String) {
        let mut start = self.cursor;
        while start > 0 && !self.value[start - 1].is_whitespace() {
            start -= 1;
        }
        (start, self.value[start..self.cursor].iter().collect())
    }

    /// Everything strictly before char index `end` — used to look at the
    /// already-typed words (e.g. the command name) ahead of the word being
    /// completed.
    pub fn value_before(&self, end: usize) -> String {
        self.value[..end].iter().collect()
    }

    /// Replaces `[start, cursor)` with `replacement` and moves the cursor to
    /// just after it — how a Tab-completion result gets applied.
    pub fn replace_word_before_cursor(&mut self, start: usize, replacement: &str) {
        let end = self.cursor;
        self.value.splice(start..end, replacement.chars());
        self.cursor = start + replacement.chars().count();
    }

    /// Takes the current value, resets the box, and — unless this was a
    /// masked (password) entry — records it in history for Up/Down recall.
    pub fn submit(&mut self) -> String {
        let submitted = self.value();
        if !self.masked && !submitted.trim().is_empty() {
            self.history.push(submitted.clone());
        }
        self.history_idx = None;
        self.clear();
        submitted
    }

    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_idx {
            Some(0) => 0,
            Some(i) => i - 1,
            None => self.history.len() - 1,
        };
        self.load_history(idx);
    }

    pub fn history_next(&mut self) {
        match self.history_idx {
            Some(i) if i + 1 < self.history.len() => self.load_history(i + 1),
            _ => {
                self.history_idx = None;
                self.clear();
            }
        }
    }

    fn load_history(&mut self, idx: usize) {
        self.history_idx = Some(idx);
        self.value = self.history[idx].chars().collect();
        self.cursor = self.value.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> InputBox {
        let mut input = InputBox::new();
        for c in s.chars() {
            input.insert(c);
        }
        input
    }

    #[test]
    fn word_before_cursor_is_the_current_token() {
        let input = typed("cd Doc");
        let (start, word) = input.word_before_cursor();
        assert_eq!(start, 3);
        assert_eq!(word, "Doc");
    }

    #[test]
    fn word_before_cursor_with_cursor_mid_word() {
        let mut input = typed("cd Documents");
        input.cursor = 5; // "cd Do|cuments"
        let (start, word) = input.word_before_cursor();
        assert_eq!(start, 3);
        assert_eq!(word, "Do");
    }

    #[test]
    fn word_before_cursor_empty_at_start_of_line() {
        let input = InputBox::new();
        let (start, word) = input.word_before_cursor();
        assert_eq!(start, 0);
        assert_eq!(word, "");
    }

    #[test]
    fn value_before_returns_prefix() {
        let input = typed("cd Doc");
        assert_eq!(input.value_before(3), "cd ");
        assert_eq!(input.value_before(0), "");
    }

    #[test]
    fn replace_word_before_cursor_expands_completion() {
        let mut input = typed("cd Doc");
        let (start, _) = input.word_before_cursor();
        input.replace_word_before_cursor(start, "Documents");
        assert_eq!(input.value(), "cd Documents");
        assert_eq!(input.cursor(), "cd Documents".chars().count());
    }

    #[test]
    fn replace_word_before_cursor_with_empty_word() {
        let mut input = typed("export ");
        let (start, word) = input.word_before_cursor();
        assert_eq!(word, "");
        input.replace_word_before_cursor(start, "notes.txt");
        assert_eq!(input.value(), "export notes.txt");
    }

    #[test]
    fn masked_display_hides_value_but_submit_returns_it() {
        let mut input = typed("secret");
        input.set_masked(true);
        assert_eq!(input.display(), "*".repeat(6));
        assert_eq!(input.submit(), "secret");
        // masked entries aren't kept in history
        input.set_masked(false);
        input.history_prev();
        assert_eq!(input.value(), "");
    }

    #[test]
    fn history_prev_next_roundtrip() {
        let mut input = typed("ls");
        input.submit();
        let mut input2 = typed("cd Docs");
        input2.history = input.history.clone();
        input2.submit();
        input2.history_prev();
        assert_eq!(input2.value(), "cd Docs");
        input2.history_prev();
        assert_eq!(input2.value(), "ls");
        input2.history_next();
        assert_eq!(input2.value(), "cd Docs");
    }
}
