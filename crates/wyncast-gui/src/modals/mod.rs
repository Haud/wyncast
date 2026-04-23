pub mod position_filter;
pub mod quit_confirm;

/// Identifies which modal is currently displayed.
#[derive(Debug, Clone, PartialEq)]
pub enum ModalKind {
    QuitConfirm,
    PositionFilter,
}

/// Push/pop stack that tracks the active modal overlay.
///
/// Only the top element is rendered and receives keyboard input.
/// An empty stack means no modal is open.
pub struct ModalStack {
    stack: Vec<ModalKind>,
}

impl ModalStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn push(&mut self, kind: ModalKind) {
        self.stack.push(kind);
    }

    pub fn pop(&mut self) {
        self.stack.pop();
    }

    pub fn top(&self) -> Option<&ModalKind> {
        self.stack.last()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stack_is_empty() {
        let stack = ModalStack::new();
        assert!(stack.is_empty());
        assert!(stack.top().is_none());
    }

    #[test]
    fn push_makes_non_empty() {
        let mut stack = ModalStack::new();
        stack.push(ModalKind::QuitConfirm);
        assert!(!stack.is_empty());
        assert_eq!(stack.top(), Some(&ModalKind::QuitConfirm));
    }

    #[test]
    fn pop_removes_top() {
        let mut stack = ModalStack::new();
        stack.push(ModalKind::QuitConfirm);
        stack.pop();
        assert!(stack.is_empty());
    }

    #[test]
    fn push_two_top_is_last() {
        let mut stack = ModalStack::new();
        stack.push(ModalKind::QuitConfirm);
        stack.push(ModalKind::PositionFilter);
        assert_eq!(stack.top(), Some(&ModalKind::PositionFilter));
    }

    #[test]
    fn pop_on_empty_is_safe() {
        let mut stack = ModalStack::new();
        stack.pop(); // must not panic
        assert!(stack.is_empty());
    }
}
