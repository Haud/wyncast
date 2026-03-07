// KeyBinding recipe and KeybindManager for the subscription system.
//
// `KeyBindingRecipe<M>` is a builder + `Recipe` implementation that maps
// key events to messages via `KeyTrigger` matching. `KeybindManager` is the
// bridge between keybinding registration and help-bar hint display: components
// call `kb.subscribe(recipe)` which extracts hint metadata and returns a
// `Subscription<M>`. The subscription system itself remains unaware of hints,
// priority, or capture mode.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{AppEvent, Listener, Recipe, Subscription, SubscriptionId};

// ---------------------------------------------------------------------------
// Priority constants
// ---------------------------------------------------------------------------

/// Normal keybinding priority (navigation, actions).
pub const PRIORITY_NORMAL: u8 = 10;
/// Capture mode priority (text input, filter mode).
pub const PRIORITY_CAPTURE: u8 = 20;
/// Modal priority (quit confirm, position filter — swallows all keys).
pub const PRIORITY_MODAL: u8 = 30;

// ---------------------------------------------------------------------------
// KeybindHint
// ---------------------------------------------------------------------------

/// A display hint for the help bar describing a keybinding.
#[derive(Clone, Debug, PartialEq)]
pub struct KeybindHint {
    pub key: String,
    pub description: String,
}

impl KeybindHint {
    pub fn new(key: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            description: description.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// KeyTrigger
// ---------------------------------------------------------------------------

/// Describes which key events a binding should fire on.
pub enum KeyTrigger {
    /// An exact key + modifier combination.
    Exact(KeyCode, KeyModifiers),
    /// Any printable character key (Char(_)) with no modifiers or shift only.
    /// Used for text capture modes.
    AnyChar,
    /// Any key at all. Used for modal capture — swallows everything.
    Any,
}

impl KeyTrigger {
    /// Returns true if this trigger matches the given key event.
    pub fn matches(&self, key: &KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        match self {
            KeyTrigger::Exact(code, mods) => key.code == *code && key.modifiers == *mods,
            KeyTrigger::AnyChar => {
                matches!(key.code, KeyCode::Char(_))
                    && (key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT)
            }
            KeyTrigger::Any => true,
        }
    }
}

/// Helper: exact key with no modifiers.
pub fn exact(code: KeyCode) -> KeyTrigger {
    KeyTrigger::Exact(code, KeyModifiers::NONE)
}

/// Helper: Ctrl+key.
pub fn ctrl(code: KeyCode) -> KeyTrigger {
    KeyTrigger::Exact(code, KeyModifiers::CONTROL)
}

// ---------------------------------------------------------------------------
// KeyBindingEntry (internal)
// ---------------------------------------------------------------------------

struct KeyBindingEntry<M> {
    trigger: KeyTrigger,
    into_message: fn(KeyEvent) -> M,
    hint: Option<KeybindHint>,
}

// ---------------------------------------------------------------------------
// KeyBindingRecipe
// ---------------------------------------------------------------------------

/// Builder and `Recipe` implementation for key-event-to-message bindings.
///
/// Create via `KeyBindingRecipe::new(identity)`, configure with the builder
/// methods, then pass to `KeybindManager::subscribe()` (or directly to
/// `Subscription::from_recipe()` if hint metadata is not needed).
pub struct KeyBindingRecipe<M: 'static> {
    entries: Vec<KeyBindingEntry<M>>,
    priority: u8,
    captures: bool,
    identity: SubscriptionId,
}

impl<M: 'static> KeyBindingRecipe<M> {
    /// Create a new recipe with the given stable identity.
    pub fn new(identity: SubscriptionId) -> Self {
        Self {
            entries: vec![],
            priority: PRIORITY_NORMAL,
            captures: false,
            identity,
        }
    }

    /// Set the hint-display priority (used by `KeybindManager` only).
    pub fn priority(mut self, p: u8) -> Self {
        self.priority = p;
        self
    }

    /// Mark this recipe as capturing (its hints take over the help bar when active).
    pub fn capture(mut self) -> Self {
        self.captures = true;
        self
    }

    /// Add a key binding. `hint` may be `None` (pass `None` or use `.into()`).
    pub fn bind(
        mut self,
        trigger: KeyTrigger,
        into_message: fn(KeyEvent) -> M,
        hint: impl Into<Option<KeybindHint>>,
    ) -> Self {
        self.entries.push(KeyBindingEntry {
            trigger,
            into_message,
            hint: hint.into(),
        });
        self
    }

    /// Collect hints for all entries that have one.
    ///
    /// Called by `KeybindManager::subscribe()` before consuming the recipe.
    pub fn hints(&self) -> Vec<KeybindHint> {
        self.entries.iter().filter_map(|e| e.hint.clone()).collect()
    }

    /// The hint-display priority level.
    pub fn priority_level(&self) -> u8 {
        self.priority
    }

    /// Whether this recipe is in capture mode.
    pub fn is_capturing(&self) -> bool {
        self.captures
    }
}

impl<M: 'static> Recipe for KeyBindingRecipe<M> {
    type Output = M;

    fn id(&self) -> SubscriptionId {
        self.identity
    }

    fn into_listener(self: Box<Self>) -> Box<dyn Listener<Output = M>> {
        Box::new(KeyBindingListener {
            entries: self.entries,
        })
    }
}

// ---------------------------------------------------------------------------
// KeyBindingListener (internal)
// ---------------------------------------------------------------------------

struct KeyBindingListener<M> {
    entries: Vec<KeyBindingEntry<M>>,
}

impl<M: 'static> Listener for KeyBindingListener<M> {
    type Output = M;

    fn process(&mut self, event: &AppEvent) -> Option<M> {
        // Only key events are relevant for keybinding subscriptions.
        let key = match event {
            AppEvent::Key(k) => k,
            AppEvent::Tick(_) => return None,
        };
        for entry in &self.entries {
            if entry.trigger.matches(key) {
                return Some((entry.into_message)(*key));
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// KeybindManager
// ---------------------------------------------------------------------------

/// Bridge between keybinding registration and help-bar hint display.
///
/// Owned by `App` and passed `&mut` through each frame's `subscription()` call.
/// Components call `kb_manager.subscribe(recipe)` which:
/// - Extracts priority, capture flag, and hints from the recipe into the manager.
/// - Returns a `Subscription<M>` for the messaging pipeline.
///
/// After the subscription tree runs, the help bar reads `kb_manager.hints()`.
/// Call `kb_manager.clear()` at the start of each frame to reset hint state.
///
/// Not generic — stores only `Vec<KeybindHint>` (strings), not message types.
pub struct KeybindManager {
    entries: Vec<KeybindEntry>,
}

struct KeybindEntry {
    priority: u8,
    captures: bool,
    hints: Vec<KeybindHint>,
}

impl KeybindManager {
    /// Create a new, empty manager.
    pub fn new() -> Self {
        Self { entries: vec![] }
    }

    /// Register a keybinding recipe.
    ///
    /// Extracts hint metadata (priority, capture flag, hints) into the manager
    /// and returns a `Subscription<M>` for the messaging pipeline.
    pub fn subscribe<M: 'static>(&mut self, recipe: KeyBindingRecipe<M>) -> Subscription<M> {
        self.entries.push(KeybindEntry {
            priority: recipe.priority_level(),
            captures: recipe.is_capturing(),
            hints: recipe.hints(),
        });
        Subscription::from_recipe(recipe)
    }

    /// Reset hint state. Call at the start of each frame before the
    /// subscription tree runs.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Read hints for the help bar.
    ///
    /// If any registered entry is marked as capturing, only that entry's hints
    /// are shown (highest priority capturing entry wins). Otherwise all hints
    /// are returned sorted by priority (highest first).
    pub fn hints(&self) -> Vec<KeybindHint> {
        let mut sorted: Vec<_> = self.entries.iter().collect();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

        for entry in &sorted {
            if entry.captures {
                return entry.hints.clone();
            }
        }
        sorted.iter().flat_map(|e| e.hints.clone()).collect()
    }

    /// Returns true if any registered entry is in capture mode.
    pub fn has_capture(&self) -> bool {
        self.entries.iter().any(|e| e.captures)
    }
}

impl Default for KeybindManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::tui::subscription::{AppEvent, SubscriptionManager};

    // ------------------------------------------------------------------
    // Helper constructors
    // ------------------------------------------------------------------

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn event(code: KeyCode) -> AppEvent {
        AppEvent::Key(key(code))
    }

    fn hint(k: &str, d: &str) -> KeybindHint {
        KeybindHint::new(k, d)
    }

    #[derive(Debug, PartialEq, Clone)]
    enum Msg {
        Quit,
        Up,
        Down,
        Char(char),
        Any,
    }

    // ------------------------------------------------------------------
    // 1. KeyTrigger::Exact — matches correct key, doesn't match wrong key
    // ------------------------------------------------------------------

    #[test]
    fn exact_trigger_matches_correct_key() {
        let trigger = KeyTrigger::Exact(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(trigger.matches(&key(KeyCode::Char('q'))));
    }

    #[test]
    fn exact_trigger_does_not_match_wrong_key() {
        let trigger = KeyTrigger::Exact(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!trigger.matches(&key(KeyCode::Char('x'))));
        assert!(!trigger.matches(&key(KeyCode::Esc)));
        // Same key but wrong modifier.
        assert!(!trigger.matches(&key_ctrl(KeyCode::Char('q'))));
    }

    // ------------------------------------------------------------------
    // 2. KeyTrigger::AnyChar — matches letter keys, not Esc/Enter/function keys
    // ------------------------------------------------------------------

    #[test]
    fn anychar_trigger_matches_char_key_no_modifier() {
        let trigger = KeyTrigger::AnyChar;
        assert!(trigger.matches(&key(KeyCode::Char('a'))));
        assert!(trigger.matches(&key(KeyCode::Char('z'))));
        assert!(trigger.matches(&key(KeyCode::Char('0'))));
    }

    #[test]
    fn anychar_trigger_matches_char_key_with_shift() {
        let trigger = KeyTrigger::AnyChar;
        assert!(trigger.matches(&key_shift(KeyCode::Char('A'))));
        assert!(trigger.matches(&key_shift(KeyCode::Char('Z'))));
    }

    #[test]
    fn anychar_trigger_does_not_match_special_keys() {
        let trigger = KeyTrigger::AnyChar;
        assert!(!trigger.matches(&key(KeyCode::Esc)));
        assert!(!trigger.matches(&key(KeyCode::Enter)));
        assert!(!trigger.matches(&key(KeyCode::F(1))));
        assert!(!trigger.matches(&key(KeyCode::Up)));
    }

    #[test]
    fn anychar_trigger_does_not_match_ctrl_char() {
        let trigger = KeyTrigger::AnyChar;
        // Ctrl+c should not be captured by AnyChar.
        assert!(!trigger.matches(&key_ctrl(KeyCode::Char('c'))));
    }

    // ------------------------------------------------------------------
    // 3. KeyTrigger::Any — matches everything
    // ------------------------------------------------------------------

    #[test]
    fn any_trigger_matches_all_keys() {
        let trigger = KeyTrigger::Any;
        assert!(trigger.matches(&key(KeyCode::Esc)));
        assert!(trigger.matches(&key(KeyCode::Enter)));
        assert!(trigger.matches(&key_ctrl(KeyCode::Char('c'))));
        assert!(trigger.matches(&key(KeyCode::Char('a'))));
        assert!(trigger.matches(&key(KeyCode::F(5))));
    }

    // ------------------------------------------------------------------
    // 4. KeyBindingRecipe::hints() returns only entries with hints
    // ------------------------------------------------------------------

    #[test]
    fn recipe_hints_returns_only_entries_with_hints() {
        let id = SubscriptionId::unique();
        let recipe = KeyBindingRecipe::<Msg>::new(id)
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, hint("q", "Quit"))
            .bind(exact(KeyCode::Up), |_| Msg::Up, None)
            .bind(exact(KeyCode::Down), |_| Msg::Down, hint("↓", "Down"));

        let hints = recipe.hints();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0], hint("q", "Quit"));
        assert_eq!(hints[1], hint("↓", "Down"));
    }

    #[test]
    fn recipe_hints_empty_when_no_hints() {
        let id = SubscriptionId::unique();
        let recipe = KeyBindingRecipe::<Msg>::new(id)
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, None)
            .bind(exact(KeyCode::Up), |_| Msg::Up, None);

        assert!(recipe.hints().is_empty());
    }

    // ------------------------------------------------------------------
    // 5. KeybindManager::subscribe() extracts hints and returns working Subscription
    // ------------------------------------------------------------------

    #[test]
    fn keybind_manager_subscribe_extracts_hints_and_returns_subscription() {
        let mut kb = KeybindManager::new();
        let id = SubscriptionId::unique();

        let recipe = KeyBindingRecipe::<Msg>::new(id)
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, hint("q", "Quit"));

        let sub = kb.subscribe(recipe);

        // Manager has the hint.
        assert_eq!(kb.hints(), vec![hint("q", "Quit")]);

        // Subscription works.
        let mut mgr = SubscriptionManager::new();
        mgr.sync(sub);
        let result = mgr.process(&event(KeyCode::Char('q')));
        assert_eq!(result, Some(Msg::Quit));
    }

    // ------------------------------------------------------------------
    // 6. KeybindManager::hints() returns all hints when no capture
    // ------------------------------------------------------------------

    #[test]
    fn keybind_manager_hints_returns_all_when_no_capture() {
        let mut kb = KeybindManager::new();

        let recipe_a = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .priority(PRIORITY_NORMAL)
            .bind(exact(KeyCode::Up), |_| Msg::Up, hint("↑", "Up"))
            .bind(exact(KeyCode::Down), |_| Msg::Down, hint("↓", "Down"));

        let recipe_b = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .priority(PRIORITY_NORMAL)
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, hint("q", "Quit"));

        kb.subscribe(recipe_a);
        kb.subscribe(recipe_b);

        let hints = kb.hints();
        assert_eq!(hints.len(), 3);
    }

    // ------------------------------------------------------------------
    // 7. KeybindManager::hints() returns only capturing entry's hints when capture active
    // ------------------------------------------------------------------

    #[test]
    fn keybind_manager_hints_returns_only_capturing_hints_when_capture_active() {
        let mut kb = KeybindManager::new();

        // Normal recipe with a normal-priority hint.
        let normal = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .priority(PRIORITY_NORMAL)
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, hint("q", "Quit"));

        // Capture recipe (e.g., text filter mode).
        let capture = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .priority(PRIORITY_CAPTURE)
            .capture()
            .bind(KeyTrigger::AnyChar, |k| {
                if let KeyCode::Char(c) = k.code { Msg::Char(c) } else { Msg::Any }
            }, hint("a-z", "Type to filter"))
            .bind(exact(KeyCode::Esc), |_| Msg::Quit, hint("Esc", "Cancel filter"));

        kb.subscribe(normal);
        kb.subscribe(capture);

        let hints = kb.hints();
        // Only the capturing recipe's hints should be shown.
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0], hint("a-z", "Type to filter"));
        assert_eq!(hints[1], hint("Esc", "Cancel filter"));
    }

    // ------------------------------------------------------------------
    // 8. KeybindManager::has_capture() returns true/false correctly
    // ------------------------------------------------------------------

    #[test]
    fn keybind_manager_has_capture_false_when_no_capture() {
        let mut kb = KeybindManager::new();
        let recipe = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, None);
        kb.subscribe(recipe);
        assert!(!kb.has_capture());
    }

    #[test]
    fn keybind_manager_has_capture_true_when_capture_active() {
        let mut kb = KeybindManager::new();
        let recipe = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .capture()
            .bind(KeyTrigger::Any, |_| Msg::Any, None);
        kb.subscribe(recipe);
        assert!(kb.has_capture());
    }

    // ------------------------------------------------------------------
    // 9. KeybindManager::clear() resets state
    // ------------------------------------------------------------------

    #[test]
    fn keybind_manager_clear_resets_state() {
        let mut kb = KeybindManager::new();

        let recipe = KeyBindingRecipe::<Msg>::new(SubscriptionId::unique())
            .capture()
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, hint("q", "Quit"));
        kb.subscribe(recipe);

        assert!(kb.has_capture());
        assert_eq!(kb.hints().len(), 1);

        kb.clear();

        assert!(!kb.has_capture());
        assert!(kb.hints().is_empty());
    }

    // ------------------------------------------------------------------
    // 10. Full integration: subscribe two recipes, process key, get correct message
    // ------------------------------------------------------------------

    #[test]
    fn integration_two_recipes_route_correctly() {
        let mut kb = KeybindManager::new();
        let mut mgr = SubscriptionManager::new();

        let id_nav = SubscriptionId::unique();
        let nav_recipe = KeyBindingRecipe::<Msg>::new(id_nav)
            .priority(PRIORITY_NORMAL)
            .bind(exact(KeyCode::Up), |_| Msg::Up, hint("↑", "Up"))
            .bind(exact(KeyCode::Down), |_| Msg::Down, hint("↓", "Down"))
            .bind(exact(KeyCode::Char('q')), |_| Msg::Quit, hint("q", "Quit"));

        let id_capture = SubscriptionId::unique();
        let capture_recipe = KeyBindingRecipe::<Msg>::new(id_capture)
            .priority(PRIORITY_CAPTURE)
            .capture()
            .bind(KeyTrigger::AnyChar, |k| {
                if let KeyCode::Char(c) = k.code { Msg::Char(c) } else { Msg::Any }
            }, hint("a-z", "Type"));

        let sub_nav = kb.subscribe(nav_recipe);
        let sub_capture = kb.subscribe(capture_recipe);

        // Sync both subscriptions (nav first, so it wins for overlapping keys).
        mgr.sync(Subscription::batch([sub_nav, sub_capture]));

        // 'q' is matched by nav first (Exact wins before AnyChar in batch order).
        let result = mgr.process(&event(KeyCode::Char('q')));
        assert_eq!(result, Some(Msg::Quit));

        // Up arrow is only matched by nav.
        let result = mgr.process(&AppEvent::Key(key(KeyCode::Up)));
        assert_eq!(result, Some(Msg::Up));

        // 'x' is not matched by nav (no binding), falls through to capture.
        let result = mgr.process(&event(KeyCode::Char('x')));
        assert_eq!(result, Some(Msg::Char('x')));

        // Esc is not matched by either — returns None.
        let result = mgr.process(&event(KeyCode::Esc));
        assert_eq!(result, None);

        // Manager knows a capture is active.
        assert!(kb.has_capture());

        // Hints: capture mode → only capture hints.
        let hints = kb.hints();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0], hint("a-z", "Type"));
    }
}
