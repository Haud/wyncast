use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    /// Find the byte position of the start of the previous word.
    ///
    /// Skips whitespace backwards, then skips non-whitespace backwards.
    /// Returns 0 if at start.
    fn word_boundary_left(&self) -> usize {
        let before = &self.value[..self.cursor];
        let mut chars = before.char_indices().rev().peekable();

        // Skip whitespace
        while let Some(&(_, ch)) = chars.peek() {
            if ch.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        // Skip non-whitespace (the word itself)
        let mut boundary = 0;
        while let Some(&(idx, ch)) = chars.peek() {
            if !ch.is_whitespace() {
                boundary = idx;
                chars.next();
            } else {
                // The word starts at `boundary`
                return boundary;
            }
        }

        boundary
    }

    /// Find the byte position after the end of the next word.
    ///
    /// Skips non-whitespace forward, then skips whitespace forward.
    /// Returns `self.value.len()` if at end.
    fn word_boundary_right(&self) -> usize {
        let after = &self.value[self.cursor..];
        let mut chars = after.char_indices().peekable();

        // Skip non-whitespace
        while let Some(&(_, ch)) = chars.peek() {
            if !ch.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        // Skip whitespace
        while let Some(&(_, ch)) = chars.peek() {
            if ch.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        match chars.peek() {
            Some(&(idx, _)) => self.cursor + idx,
            None => self.value.len(),
        }
    }

    /// Move cursor to the start of the previous word.
    pub fn move_word_left(&mut self) {
        self.cursor = self.word_boundary_left();
    }

    /// Move cursor past the end of the next word.
    pub fn move_word_right(&mut self) {
        self.cursor = self.word_boundary_right();
    }

    /// Delete from the start of the previous word to the cursor.
    pub fn delete_word_backward(&mut self) {
        let boundary = self.word_boundary_left();
        self.value.drain(boundary..self.cursor);
        self.cursor = boundary;
    }

    /// Delete from the cursor to the end of the next word.
    pub fn delete_word_forward(&mut self) {
        let boundary = self.word_boundary_right();
        self.value.drain(self.cursor..boundary);
    }

    /// Delete from the start of the line to the cursor (Ctrl+U).
    pub fn delete_to_start(&mut self) {
        self.value.drain(..self.cursor);
        self.cursor = 0;
    }

    /// Delete from the cursor to the end of the line (Ctrl+K).
    pub fn delete_to_end(&mut self) {
        self.value.truncate(self.cursor);
    }
}

/// Messages for the TextInput component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputMessage {
    InsertChar(char),
    Backspace,
    Delete,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    MoveWordLeft,
    MoveWordRight,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToStart,
    DeleteToEnd,
    ToggleOverwrite,
}

impl TextInput {
    /// Process a message, updating internal state.
    pub fn update(&mut self, msg: TextInputMessage) {
        match msg {
            TextInputMessage::InsertChar(c) => self.insert_char(c),
            TextInputMessage::Backspace => self.backspace(),
            TextInputMessage::Delete => self.delete(),
            TextInputMessage::MoveLeft => self.move_left(),
            TextInputMessage::MoveRight => self.move_right(),
            TextInputMessage::MoveHome => self.move_home(),
            TextInputMessage::MoveEnd => self.move_end(),
            TextInputMessage::MoveWordLeft => self.move_word_left(),
            TextInputMessage::MoveWordRight => self.move_word_right(),
            TextInputMessage::DeleteWordBackward => self.delete_word_backward(),
            TextInputMessage::DeleteWordForward => self.delete_word_forward(),
            TextInputMessage::DeleteToStart => self.delete_to_start(),
            TextInputMessage::DeleteToEnd => self.delete_to_end(),
            TextInputMessage::ToggleOverwrite => self.toggle_overwrite(),
        }
    }

    /// Convert a key event to a TextInputMessage, if relevant.
    /// Returns None for keys not handled by text input (Enter, Esc, etc.)
    pub fn key_to_message(key: &KeyEvent) -> Option<TextInputMessage> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            // Ctrl + movement
            KeyCode::Left if ctrl => Some(TextInputMessage::MoveWordLeft),
            KeyCode::Right if ctrl => Some(TextInputMessage::MoveWordRight),

            // Ctrl + deletion
            KeyCode::Backspace if ctrl => Some(TextInputMessage::DeleteWordBackward),
            KeyCode::Delete if ctrl => Some(TextInputMessage::DeleteWordForward),
            KeyCode::Char('w') if ctrl => Some(TextInputMessage::DeleteWordBackward),
            KeyCode::Char('u') if ctrl => Some(TextInputMessage::DeleteToStart),
            KeyCode::Char('k') if ctrl => Some(TextInputMessage::DeleteToEnd),

            // Ctrl + navigation (readline/emacs style)
            KeyCode::Char('a') if ctrl => Some(TextInputMessage::MoveHome),
            KeyCode::Char('e') if ctrl => Some(TextInputMessage::MoveEnd),

            // Alt + movement (some terminals send alt instead of ctrl for word movement)
            KeyCode::Left if alt => Some(TextInputMessage::MoveWordLeft),
            KeyCode::Right if alt => Some(TextInputMessage::MoveWordRight),
            KeyCode::Char('b') if alt => Some(TextInputMessage::MoveWordLeft),
            KeyCode::Char('f') if alt => Some(TextInputMessage::MoveWordRight),
            KeyCode::Backspace if alt => Some(TextInputMessage::DeleteWordBackward),
            KeyCode::Char('d') if alt => Some(TextInputMessage::DeleteWordForward),

            // Plain keys (no Ctrl/Alt modifier, Shift is OK)
            KeyCode::Backspace => Some(TextInputMessage::Backspace),
            KeyCode::Delete => Some(TextInputMessage::Delete),
            KeyCode::Left => Some(TextInputMessage::MoveLeft),
            KeyCode::Right => Some(TextInputMessage::MoveRight),
            KeyCode::Home => Some(TextInputMessage::MoveHome),
            KeyCode::End => Some(TextInputMessage::MoveEnd),
            KeyCode::Insert => Some(TextInputMessage::ToggleOverwrite),

            // Only insert printable chars when no Ctrl/Alt modifier
            KeyCode::Char(c) if !ctrl && !alt => Some(TextInputMessage::InsertChar(c)),

            _ => None,
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

    // -----------------------------------------------------------------------
    // ELM message API tests
    // -----------------------------------------------------------------------

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn key_to_message_char() {
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Char('a'))),
            Some(TextInputMessage::InsertChar('a'))
        );
    }

    #[test]
    fn key_to_message_backspace() {
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Backspace)),
            Some(TextInputMessage::Backspace)
        );
    }

    #[test]
    fn key_to_message_delete() {
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Delete)),
            Some(TextInputMessage::Delete)
        );
    }

    #[test]
    fn key_to_message_movement_keys() {
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Left)),
            Some(TextInputMessage::MoveLeft)
        );
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Right)),
            Some(TextInputMessage::MoveRight)
        );
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Home)),
            Some(TextInputMessage::MoveHome)
        );
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::End)),
            Some(TextInputMessage::MoveEnd)
        );
    }

    #[test]
    fn key_to_message_insert() {
        assert_eq!(
            TextInput::key_to_message(&key(KeyCode::Insert)),
            Some(TextInputMessage::ToggleOverwrite)
        );
    }

    #[test]
    fn key_to_message_returns_none_for_unhandled_keys() {
        assert_eq!(TextInput::key_to_message(&key(KeyCode::Enter)), None);
        assert_eq!(TextInput::key_to_message(&key(KeyCode::Esc)), None);
        assert_eq!(TextInput::key_to_message(&key(KeyCode::Tab)), None);
        assert_eq!(TextInput::key_to_message(&key(KeyCode::F(1))), None);
        assert_eq!(TextInput::key_to_message(&key(KeyCode::Up)), None);
        assert_eq!(TextInput::key_to_message(&key(KeyCode::Down)), None);
    }

    #[test]
    fn update_insert_char() {
        let mut ti = TextInput::new();
        ti.update(TextInputMessage::InsertChar('h'));
        ti.update(TextInputMessage::InsertChar('i'));
        assert_eq!(ti.value(), "hi");
        assert_eq!(ti.cursor_pos(), 2);
    }

    #[test]
    fn update_backspace_and_delete() {
        let mut ti = TextInput::with_value("abc");
        ti.update(TextInputMessage::Backspace);
        assert_eq!(ti.value(), "ab");
        ti.update(TextInputMessage::MoveHome);
        ti.update(TextInputMessage::Delete);
        assert_eq!(ti.value(), "b");
    }

    #[test]
    fn update_movement() {
        let mut ti = TextInput::with_value("hello");
        ti.update(TextInputMessage::MoveHome);
        assert_eq!(ti.cursor_pos(), 0);
        ti.update(TextInputMessage::MoveRight);
        assert_eq!(ti.cursor_pos(), 1);
        ti.update(TextInputMessage::MoveEnd);
        assert_eq!(ti.cursor_pos(), 5);
        ti.update(TextInputMessage::MoveLeft);
        assert_eq!(ti.cursor_pos(), 4);
    }

    #[test]
    fn update_toggle_overwrite() {
        let mut ti = TextInput::with_value("ab");
        assert!(!ti.is_overwrite());
        ti.update(TextInputMessage::ToggleOverwrite);
        assert!(ti.is_overwrite());
        ti.update(TextInputMessage::MoveHome);
        ti.update(TextInputMessage::InsertChar('X'));
        assert_eq!(ti.value(), "Xb");
    }

    // -----------------------------------------------------------------------
    // Word boundary and word-level editing tests
    // -----------------------------------------------------------------------

    #[test]
    fn word_boundary_left_multiple_words() {
        let mut ti = TextInput::with_value("hello world foo");
        // Cursor at end (after "foo")
        assert_eq!(ti.word_boundary_left(), 12); // start of "foo"
        ti.cursor = 12;
        assert_eq!(ti.word_boundary_left(), 6); // start of "world"
        ti.cursor = 6;
        assert_eq!(ti.word_boundary_left(), 0); // start of "hello"
        ti.cursor = 0;
        assert_eq!(ti.word_boundary_left(), 0); // already at start
    }

    #[test]
    fn word_boundary_left_from_middle_of_word() {
        let mut ti = TextInput::with_value("hello world");
        ti.cursor = 8; // middle of "world" (after "wor")
        assert_eq!(ti.word_boundary_left(), 6); // start of "world"
    }

    #[test]
    fn word_boundary_left_leading_spaces() {
        let mut ti = TextInput::with_value("  hello");
        // cursor at end
        assert_eq!(ti.word_boundary_left(), 2); // start of "hello"
        ti.cursor = 2;
        assert_eq!(ti.word_boundary_left(), 0); // at start (only whitespace before)
    }

    #[test]
    fn word_boundary_left_trailing_spaces() {
        let ti = TextInput::with_value("hello  ");
        // cursor at end (after trailing spaces)
        assert_eq!(ti.word_boundary_left(), 0); // skips spaces, then "hello"
    }

    #[test]
    fn word_boundary_left_consecutive_spaces() {
        let mut ti = TextInput::with_value("hello   world");
        ti.cursor = 13; // end
        assert_eq!(ti.word_boundary_left(), 8); // start of "world"
        ti.cursor = 8;
        assert_eq!(ti.word_boundary_left(), 0); // start of "hello"
    }

    #[test]
    fn word_boundary_right_multiple_words() {
        let mut ti = TextInput::with_value("hello world foo");
        ti.cursor = 0;
        assert_eq!(ti.word_boundary_right(), 6); // past "hello "
        ti.cursor = 6;
        assert_eq!(ti.word_boundary_right(), 12); // past "world "
        ti.cursor = 12;
        assert_eq!(ti.word_boundary_right(), 15); // end
    }

    #[test]
    fn word_boundary_right_from_middle_of_word() {
        let mut ti = TextInput::with_value("hello world");
        ti.cursor = 2; // middle of "hello"
        assert_eq!(ti.word_boundary_right(), 6); // past "llo "
    }

    #[test]
    fn word_boundary_right_trailing_spaces() {
        let mut ti = TextInput::with_value("hello  ");
        ti.cursor = 0;
        assert_eq!(ti.word_boundary_right(), 7); // past "hello  " (end)
    }

    #[test]
    fn word_boundary_right_at_end() {
        let mut ti = TextInput::with_value("hello");
        ti.cursor = 5;
        assert_eq!(ti.word_boundary_right(), 5); // already at end
    }

    #[test]
    fn move_word_left() {
        let mut ti = TextInput::with_value("hello world");
        ti.move_word_left();
        assert_eq!(ti.cursor_byte(), 6);
        assert_eq!(ti.cursor_pos(), 6);
        ti.move_word_left();
        assert_eq!(ti.cursor_byte(), 0);
    }

    #[test]
    fn move_word_right() {
        let mut ti = TextInput::with_value("hello world");
        ti.move_home();
        ti.move_word_right();
        assert_eq!(ti.cursor_byte(), 6);
        ti.move_word_right();
        assert_eq!(ti.cursor_byte(), 11);
    }

    #[test]
    fn delete_word_backward_basic() {
        let mut ti = TextInput::with_value("hello world");
        ti.delete_word_backward();
        assert_eq!(ti.value(), "hello ");
        assert_eq!(ti.cursor_byte(), 6);
    }

    #[test]
    fn delete_word_backward_middle() {
        let mut ti = TextInput::with_value("one two three");
        ti.cursor = 7; // after "two"
        ti.delete_word_backward();
        assert_eq!(ti.value(), "one  three");
        assert_eq!(ti.cursor_byte(), 4);
    }

    #[test]
    fn delete_word_backward_at_start() {
        let mut ti = TextInput::with_value("hello");
        ti.move_home();
        ti.delete_word_backward();
        assert_eq!(ti.value(), "hello");
        assert_eq!(ti.cursor_byte(), 0);
    }

    #[test]
    fn delete_word_forward_basic() {
        let mut ti = TextInput::with_value("hello world");
        ti.move_home();
        ti.delete_word_forward();
        assert_eq!(ti.value(), "world");
        assert_eq!(ti.cursor_byte(), 0);
    }

    #[test]
    fn delete_word_forward_middle() {
        let mut ti = TextInput::with_value("hello world foo");
        ti.cursor = 6; // start of "world"
        ti.delete_word_forward();
        assert_eq!(ti.value(), "hello foo");
        assert_eq!(ti.cursor_byte(), 6);
    }

    #[test]
    fn delete_word_forward_at_end() {
        let mut ti = TextInput::with_value("hello");
        ti.delete_word_forward();
        assert_eq!(ti.value(), "hello");
    }

    #[test]
    fn delete_to_start() {
        let mut ti = TextInput::with_value("hello world");
        ti.cursor = 6; // after "hello "
        ti.delete_to_start();
        assert_eq!(ti.value(), "world");
        assert_eq!(ti.cursor_byte(), 0);
    }

    #[test]
    fn delete_to_start_at_beginning() {
        let mut ti = TextInput::with_value("hello");
        ti.move_home();
        ti.delete_to_start();
        assert_eq!(ti.value(), "hello");
        assert_eq!(ti.cursor_byte(), 0);
    }

    #[test]
    fn delete_to_end() {
        let mut ti = TextInput::with_value("hello world");
        ti.cursor = 5; // after "hello"
        ti.delete_to_end();
        assert_eq!(ti.value(), "hello");
        assert_eq!(ti.cursor_byte(), 5);
    }

    #[test]
    fn delete_to_end_at_end() {
        let mut ti = TextInput::with_value("hello");
        ti.delete_to_end();
        assert_eq!(ti.value(), "hello");
    }

    #[test]
    fn word_operations_with_unicode() {
        let mut ti = TextInput::with_value("αβγ δεζ");
        // cursor at end
        ti.move_word_left();
        assert_eq!(ti.cursor_pos(), 4); // start of "δεζ"
        ti.move_word_left();
        assert_eq!(ti.cursor_pos(), 0); // start of "αβγ"

        ti.move_word_right();
        // Should be past "αβγ "
        let mut ti2 = TextInput::with_value("αβγ δεζ");
        ti2.move_home();
        ti2.delete_word_forward();
        assert_eq!(ti2.value(), "δεζ");
    }

    // -----------------------------------------------------------------------
    // key_to_message modifier tests
    // -----------------------------------------------------------------------

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn alt_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::ALT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn key_to_message_ctrl_left_right() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Left)),
            Some(TextInputMessage::MoveWordLeft)
        );
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Right)),
            Some(TextInputMessage::MoveWordRight)
        );
    }

    #[test]
    fn key_to_message_ctrl_backspace_delete() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Backspace)),
            Some(TextInputMessage::DeleteWordBackward)
        );
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Delete)),
            Some(TextInputMessage::DeleteWordForward)
        );
    }

    #[test]
    fn key_to_message_ctrl_w_deletes_word_backward() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('w'))),
            Some(TextInputMessage::DeleteWordBackward)
        );
    }

    #[test]
    fn key_to_message_ctrl_u_deletes_to_start() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('u'))),
            Some(TextInputMessage::DeleteToStart)
        );
    }

    #[test]
    fn key_to_message_ctrl_k_deletes_to_end() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('k'))),
            Some(TextInputMessage::DeleteToEnd)
        );
    }

    #[test]
    fn key_to_message_ctrl_a_moves_home() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('a'))),
            Some(TextInputMessage::MoveHome)
        );
    }

    #[test]
    fn key_to_message_ctrl_e_moves_end() {
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('e'))),
            Some(TextInputMessage::MoveEnd)
        );
    }

    #[test]
    fn key_to_message_ctrl_random_char_returns_none() {
        // Ctrl+z, Ctrl+x, etc. should NOT insert characters
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('z'))),
            None
        );
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('x'))),
            None
        );
        assert_eq!(
            TextInput::key_to_message(&ctrl_key(KeyCode::Char('q'))),
            None
        );
    }

    #[test]
    fn key_to_message_alt_left_right() {
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Left)),
            Some(TextInputMessage::MoveWordLeft)
        );
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Right)),
            Some(TextInputMessage::MoveWordRight)
        );
    }

    #[test]
    fn key_to_message_alt_b_f_word_movement() {
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Char('b'))),
            Some(TextInputMessage::MoveWordLeft)
        );
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Char('f'))),
            Some(TextInputMessage::MoveWordRight)
        );
    }

    #[test]
    fn key_to_message_alt_backspace_deletes_word() {
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Backspace)),
            Some(TextInputMessage::DeleteWordBackward)
        );
    }

    #[test]
    fn key_to_message_alt_d_deletes_word_forward() {
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Char('d'))),
            Some(TextInputMessage::DeleteWordForward)
        );
    }

    #[test]
    fn key_to_message_alt_random_char_returns_none() {
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Char('z'))),
            None
        );
        assert_eq!(
            TextInput::key_to_message(&alt_key(KeyCode::Char('x'))),
            None
        );
    }

    // -----------------------------------------------------------------------
    // Update tests for new message variants
    // -----------------------------------------------------------------------

    #[test]
    fn update_move_word_left_right() {
        let mut ti = TextInput::with_value("hello world");
        ti.update(TextInputMessage::MoveWordLeft);
        assert_eq!(ti.cursor_byte(), 6);
        ti.update(TextInputMessage::MoveWordRight);
        assert_eq!(ti.cursor_byte(), 11);
    }

    #[test]
    fn update_delete_word_backward() {
        let mut ti = TextInput::with_value("hello world");
        ti.update(TextInputMessage::DeleteWordBackward);
        assert_eq!(ti.value(), "hello ");
    }

    #[test]
    fn update_delete_word_forward() {
        let mut ti = TextInput::with_value("hello world");
        ti.update(TextInputMessage::MoveHome);
        ti.update(TextInputMessage::DeleteWordForward);
        assert_eq!(ti.value(), "world");
    }

    #[test]
    fn update_delete_to_start() {
        let mut ti = TextInput::with_value("hello world");
        ti.cursor = 6;
        ti.update(TextInputMessage::DeleteToStart);
        assert_eq!(ti.value(), "world");
        assert_eq!(ti.cursor_byte(), 0);
    }

    #[test]
    fn update_delete_to_end() {
        let mut ti = TextInput::with_value("hello world");
        ti.cursor = 5;
        ti.update(TextInputMessage::DeleteToEnd);
        assert_eq!(ti.value(), "hello");
        assert_eq!(ti.cursor_byte(), 5);
    }
}
