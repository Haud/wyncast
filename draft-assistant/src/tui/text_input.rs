// Text input widget with cursor movement, backspace/delete, and insert/overwrite mode.
//
// `TextInput` wraps a `String` buffer and a byte-level cursor position, providing
// the primitives needed for full single-line editing without depending on external
// crates. All cursor positions are byte offsets that always fall on valid UTF-8
// character boundaries.
//
// # Supported operations
// - Append / insert at cursor
// - Overwrite mode (replaces the character under the cursor)
// - Backspace (delete the character before the cursor)
// - Delete (delete the character at the cursor)
// - Move cursor left / right by one character
// - Move cursor to start / end of input (Home / End)
// - Clear the whole buffer
//
// # Rendering
// Use `value()` to get the full string and `cursor_pos()` to get the display
// column of the cursor (i.e. the number of *characters* before it).  Both are
// cheap: `cursor_pos()` is O(cursor_byte) because it counts UTF-8 scalar values
// rather than terminal display columns, which is sufficient for the ASCII-heavy
// content in this application.

/// A single-line text input buffer with a moveable cursor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextInput {
    /// The full text content.
    value: String,
    /// Byte offset of the cursor inside `value`.  Always falls on a UTF-8
    /// character boundary, in the range `[0, value.len()]`.
    cursor: usize,
    /// When `true`, typing overwrites the character under the cursor rather
    /// than inserting before it.
    overwrite: bool,
}

impl TextInput {
    /// Create an empty `TextInput` in insert mode.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a `TextInput` pre-filled with `text`, cursor at end.
    pub fn with_value(text: &str) -> Self {
        let len = text.len();
        TextInput {
            value: text.to_string(),
            cursor: len,
            overwrite: false,
        }
    }

    /// Return the current string value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Return whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Return the cursor position as a *character* index (suitable for display).
    pub fn cursor_pos(&self) -> usize {
        self.value[..self.cursor].chars().count()
    }

    /// Return the cursor byte offset.
    pub fn cursor_byte(&self) -> usize {
        self.cursor
    }

    /// Return whether overwrite mode is active.
    pub fn is_overwrite(&self) -> bool {
        self.overwrite
    }

    /// Toggle between insert and overwrite mode.
    pub fn toggle_overwrite(&mut self) {
        self.overwrite = !self.overwrite;
    }

    /// Insert `ch` at the cursor position (or overwrite the character under
    /// the cursor if overwrite mode is active), then advance the cursor.
    pub fn insert_char(&mut self, ch: char) {
        if self.overwrite && self.cursor < self.value.len() {
            // Delete the character currently under the cursor, then insert.
            let char_len = self.value[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.value.drain(self.cursor..self.cursor + char_len);
        }
        self.value.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Delete the character immediately *before* the cursor (backspace).
    ///
    /// Does nothing if the cursor is at the start.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Walk backwards to find the start of the preceding UTF-8 character.
        let new_cursor = self.prev_char_boundary(self.cursor);
        self.value.drain(new_cursor..self.cursor);
        self.cursor = new_cursor;
    }

    /// Delete the character *at* the cursor (forward delete).
    ///
    /// Does nothing if the cursor is at the end.
    pub fn delete(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let char_len = self.value[self.cursor..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.value.drain(self.cursor..self.cursor + char_len);
        // cursor stays at the same byte offset (now pointing at the next char)
    }

    /// Move the cursor one character to the left.
    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.prev_char_boundary(self.cursor);
    }

    /// Move the cursor one character to the right.
    pub fn move_right(&mut self) {
        if self.cursor >= self.value.len() {
            return;
        }
        let char_len = self.value[self.cursor..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor += char_len;
    }

    /// Move the cursor to the beginning of the input.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move the cursor to the end of the input.
    pub fn move_end(&mut self) {
        self.cursor = self.value.len();
    }

    /// Clear the buffer and reset the cursor.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Replace the buffer contents with `text`, cursor at end.
    pub fn set_value(&mut self, text: &str) {
        self.value = text.to_string();
        self.cursor = self.value.len();
    }

    /// Return the character before the cursor, if any.
    fn prev_char_boundary(&self, byte_pos: usize) -> usize {
        let mut pos = byte_pos;
        loop {
            if pos == 0 {
                return 0;
            }
            pos -= 1;
            if self.value.is_char_boundary(pos) {
                return pos;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let ti = TextInput::new();
        assert_eq!(ti.value(), "");
        assert_eq!(ti.cursor_pos(), 0);
        assert!(ti.is_empty());
    }

    #[test]
    fn with_value_cursor_at_end() {
        let ti = TextInput::with_value("hello");
        assert_eq!(ti.value(), "hello");
        assert_eq!(ti.cursor_pos(), 5);
    }

    #[test]
    fn insert_char_appends_at_end() {
        let mut ti = TextInput::new();
        ti.insert_char('a');
        ti.insert_char('b');
        ti.insert_char('c');
        assert_eq!(ti.value(), "abc");
        assert_eq!(ti.cursor_pos(), 3);
    }

    #[test]
    fn insert_char_at_middle() {
        let mut ti = TextInput::with_value("ac");
        ti.move_left(); // cursor at 'c'
        ti.insert_char('b');
        assert_eq!(ti.value(), "abc");
        assert_eq!(ti.cursor_pos(), 2);
    }

    #[test]
    fn backspace_removes_preceding_char() {
        let mut ti = TextInput::with_value("hello");
        ti.backspace();
        assert_eq!(ti.value(), "hell");
        assert_eq!(ti.cursor_pos(), 4);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut ti = TextInput::new();
        ti.backspace(); // should not panic
        assert_eq!(ti.value(), "");
        assert_eq!(ti.cursor_pos(), 0);
    }

    #[test]
    fn backspace_middle() {
        let mut ti = TextInput::with_value("abcd");
        ti.move_left(); // before 'd'
        ti.move_left(); // before 'c'
        ti.backspace(); // removes 'b'
        assert_eq!(ti.value(), "acd");
        assert_eq!(ti.cursor_pos(), 1);
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut ti = TextInput::with_value("abcd");
        ti.move_home();
        ti.delete(); // removes 'a'
        assert_eq!(ti.value(), "bcd");
        assert_eq!(ti.cursor_pos(), 0);
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut ti = TextInput::with_value("hi");
        ti.delete(); // cursor is already at end
        assert_eq!(ti.value(), "hi");
    }

    #[test]
    fn move_left_right() {
        let mut ti = TextInput::with_value("ab");
        ti.move_left();
        assert_eq!(ti.cursor_pos(), 1);
        ti.move_left();
        assert_eq!(ti.cursor_pos(), 0);
        ti.move_left(); // noop at start
        assert_eq!(ti.cursor_pos(), 0);
        ti.move_right();
        assert_eq!(ti.cursor_pos(), 1);
        ti.move_right();
        assert_eq!(ti.cursor_pos(), 2);
        ti.move_right(); // noop at end
        assert_eq!(ti.cursor_pos(), 2);
    }

    #[test]
    fn home_end() {
        let mut ti = TextInput::with_value("hello");
        ti.move_home();
        assert_eq!(ti.cursor_pos(), 0);
        ti.move_end();
        assert_eq!(ti.cursor_pos(), 5);
    }

    #[test]
    fn clear() {
        let mut ti = TextInput::with_value("hello");
        ti.clear();
        assert!(ti.is_empty());
        assert_eq!(ti.cursor_pos(), 0);
    }

    #[test]
    fn set_value() {
        let mut ti = TextInput::new();
        ti.set_value("world");
        assert_eq!(ti.value(), "world");
        assert_eq!(ti.cursor_pos(), 5);
    }

    #[test]
    fn overwrite_mode() {
        let mut ti = TextInput::with_value("hello");
        ti.move_home();
        ti.toggle_overwrite();
        assert!(ti.is_overwrite());
        ti.insert_char('H'); // replaces 'h'
        assert_eq!(ti.value(), "Hello");
        assert_eq!(ti.cursor_pos(), 1);
    }

    #[test]
    fn overwrite_at_end_inserts() {
        let mut ti = TextInput::with_value("hi");
        ti.toggle_overwrite();
        ti.insert_char('!'); // at end, should insert
        assert_eq!(ti.value(), "hi!");
    }

    #[test]
    fn unicode_chars() {
        let mut ti = TextInput::new();
        ti.insert_char('α');
        ti.insert_char('β');
        ti.insert_char('γ');
        assert_eq!(ti.value(), "αβγ");
        assert_eq!(ti.cursor_pos(), 3);
        ti.backspace();
        assert_eq!(ti.value(), "αβ");
        ti.move_left();
        assert_eq!(ti.cursor_pos(), 1);
        ti.delete();
        assert_eq!(ti.value(), "α");
    }
}
