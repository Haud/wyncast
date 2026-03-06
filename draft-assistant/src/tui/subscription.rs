// Subscription system: event routing infrastructure for the TUI.
//
// The subscription model decouples event producers (keyboard, tick, resize)
// from consumers (screen components). Each component declares a
// `Subscription<M>` describing which events it cares about; the runtime
// collects all subscriptions, diffs them on each frame, and routes events
// through the active listener set in priority order.
//
// # Key types
//
// - [`SubscriptionId`] — stable identity used to track lifecycle across frames
// - [`AppEvent`] — raw events the runtime feeds to listeners
// - [`Recipe`] — extensibility point; creates a [`Listener`] on activation
// - [`Listener`] — processes events, optionally capturing (blocking lower-priority listeners)
// - [`Subscription<M>`] — generic container of recipes; supports `none`, `batch`, `map`
// - [`SubscriptionManager<M>`] — diffs subscriptions, activates/drops listeners, routes events
// - [`KeybindHint`] — display hint for the help bar

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// SubscriptionId
// ---------------------------------------------------------------------------

/// Stable, unique identity for a subscription across frames.
///
/// The ID determines whether a subscription is "the same" from one frame to
/// the next. If a subscription with the same ID is present in two consecutive
/// calls to [`SubscriptionManager::sync`], its listener is kept alive (not
/// restarted). A new ID causes the listener to be torn down and rebuilt.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SubscriptionId(u64);

impl SubscriptionId {
    /// Allocate a fresh, globally unique ID.
    ///
    /// IDs are assigned from a monotonically increasing atomic counter so
    /// they are unique across the process lifetime.
    pub fn unique() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------
// AppEvent
// ---------------------------------------------------------------------------

/// Raw events the runtime feeds to listeners.
///
/// Additional variants (Tick, Resize, Custom) will be added as needed.
pub enum AppEvent {
    /// A keyboard event from crossterm.
    Key(crossterm::event::KeyEvent),
}

// ---------------------------------------------------------------------------
// KeybindHint
// ---------------------------------------------------------------------------

/// A keybind hint displayed in the help bar.
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
// Recipe
// ---------------------------------------------------------------------------

/// Describes how to create a [`Listener`] and identifies it across frames.
///
/// A `Recipe` is the declaration side of a subscription. It is cheap to
/// create and compare (via its [`id`][Recipe::id]). The runtime calls
/// [`into_listener`][Recipe::into_listener] exactly once when the ID is new,
/// and the resulting [`Listener`] lives until the ID disappears.
pub trait Recipe: 'static {
    type Output;

    /// Stable identity for this subscription.
    fn id(&self) -> SubscriptionId;

    /// Consume the recipe and produce a live listener.
    fn into_listener(self: Box<Self>) -> Box<dyn Listener<Output = Self::Output>>;
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// A live event handler created from a [`Recipe`].
///
/// Listeners are kept alive across frames as long as their ID appears in the
/// current subscription set. They receive every [`AppEvent`] in priority
/// order; a capturing listener blocks all lower-priority listeners.
pub trait Listener {
    type Output;

    /// Process one event. Returns `Some(msg)` if the event was consumed.
    fn process(&mut self, event: &AppEvent) -> Option<Self::Output>;

    /// Keybind hints this listener wants shown in the help bar.
    fn hints(&self) -> Vec<KeybindHint> {
        vec![]
    }

    /// Higher priority listeners receive events first.
    fn priority(&self) -> u8 {
        0
    }

    /// If true, lower-priority listeners do not receive the event even when
    /// this listener returns `None`.
    fn captures(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Subscription<M>
// ---------------------------------------------------------------------------

/// A set of recipes that will be activated by the runtime.
///
/// `Subscription<M>` is the public API surface for components. Construct one
/// with [`Subscription::none`], [`Subscription::from_recipe`], or
/// [`Subscription::batch`], then transform it with [`Subscription::map`].
pub struct Subscription<M: 'static> {
    recipes: Vec<Box<dyn Recipe<Output = M>>>,
}

impl<M: 'static> Subscription<M> {
    /// An empty subscription (no events requested).
    pub fn none() -> Self {
        Self { recipes: vec![] }
    }

    /// Merge multiple subscriptions into one.
    pub fn batch(subs: impl IntoIterator<Item = Self>) -> Self {
        Self {
            recipes: subs.into_iter().flat_map(|s| s.recipes).collect(),
        }
    }

    /// Create a subscription from a single concrete recipe.
    pub fn from_recipe(recipe: impl Recipe<Output = M> + 'static) -> Self {
        Self {
            recipes: vec![Box::new(recipe)],
        }
    }

    /// Transform the message type produced by every recipe in this subscription.
    ///
    /// `f` must be a function pointer (not a closure) so it is `Copy + 'static`.
    pub fn map<N: 'static>(self, f: fn(M) -> N) -> Subscription<N> {
        Subscription {
            recipes: self
                .recipes
                .into_iter()
                .map(|r| -> Box<dyn Recipe<Output = N>> { Box::new(MapRecipe { inner: r, f }) })
                .collect(),
        }
    }

    /// Unwrap the internal recipe list (used by [`SubscriptionManager`]).
    pub fn into_recipes(self) -> Vec<Box<dyn Recipe<Output = M>>> {
        self.recipes
    }
}

// ---------------------------------------------------------------------------
// MapRecipe / MapListener
// ---------------------------------------------------------------------------

/// Internal wrapper that applies a mapping function at the recipe level.
struct MapRecipe<A: 'static, B: 'static> {
    inner: Box<dyn Recipe<Output = A>>,
    f: fn(A) -> B,
}

impl<A: 'static, B: 'static> Recipe for MapRecipe<A, B> {
    type Output = B;

    fn id(&self) -> SubscriptionId {
        self.inner.id()
    }

    fn into_listener(self: Box<Self>) -> Box<dyn Listener<Output = B>> {
        Box::new(MapListener {
            inner: self.inner.into_listener(),
            f: self.f,
        })
    }
}

/// Internal wrapper that applies a mapping function at the listener level.
struct MapListener<A: 'static, B: 'static> {
    inner: Box<dyn Listener<Output = A>>,
    f: fn(A) -> B,
}

impl<A: 'static, B: 'static> Listener for MapListener<A, B> {
    type Output = B;

    fn process(&mut self, event: &AppEvent) -> Option<B> {
        self.inner.process(event).map(self.f)
    }

    fn hints(&self) -> Vec<KeybindHint> {
        self.inner.hints()
    }

    fn priority(&self) -> u8 {
        self.inner.priority()
    }

    fn captures(&self) -> bool {
        self.inner.captures()
    }
}

// ---------------------------------------------------------------------------
// SubscriptionManager<M>
// ---------------------------------------------------------------------------

/// Manages the set of active listeners across frames.
///
/// Call [`sync`][SubscriptionManager::sync] once per frame with the current
/// subscription to activate new listeners and drop removed ones. Then call
/// [`process`][SubscriptionManager::process] for each incoming event to
/// obtain a message.
pub struct SubscriptionManager<M: 'static> {
    active: HashMap<SubscriptionId, ActiveEntry<M>>,
}

struct ActiveEntry<M> {
    listener: Box<dyn Listener<Output = M>>,
    priority: u8,
}

impl<M: 'static> SubscriptionManager<M> {
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
        }
    }

    /// Diff the new subscription against the current active set.
    ///
    /// - New IDs are activated (recipe converted to listener).
    /// - Removed IDs are dropped.
    /// - Existing IDs are kept as-is (listener state preserved).
    pub fn sync(&mut self, subscription: Subscription<M>) {
        let new_recipes = subscription.into_recipes();
        let mut new_ids: HashSet<SubscriptionId> = HashSet::new();
        let mut to_start: Vec<(SubscriptionId, Box<dyn Recipe<Output = M>>)> = Vec::new();

        for recipe in new_recipes {
            let id = recipe.id();
            new_ids.insert(id);
            if !self.active.contains_key(&id) {
                to_start.push((id, recipe));
            }
        }

        // Drop listeners whose IDs are no longer present.
        self.active.retain(|id, _| new_ids.contains(id));

        // Activate new listeners.
        for (id, recipe) in to_start {
            let listener = recipe.into_listener();
            let priority = listener.priority();
            self.active.insert(id, ActiveEntry { listener, priority });
        }
    }

    /// Route an event through all active listeners in priority order.
    ///
    /// Returns the first message produced. A capturing listener blocks all
    /// lower-priority listeners from receiving the event.
    pub fn process(&mut self, event: &AppEvent) -> Option<M> {
        // Collect mutable refs sorted by descending priority.
        let mut entries: Vec<&mut ActiveEntry<M>> = self.active.values_mut().collect();
        entries.sort_by(|a, b| b.priority.cmp(&a.priority));

        for entry in entries {
            if let Some(msg) = entry.listener.process(event) {
                return Some(msg);
            }
            if entry.listener.captures() {
                return None;
            }
        }
        None
    }

    /// Collect keybind hints from all active listeners.
    ///
    /// If any listener has [`captures`][Listener::captures] true, only that
    /// listener's hints are returned (it has exclusive focus). Otherwise all
    /// hints are merged.
    pub fn hints(&self) -> Vec<KeybindHint> {
        let mut entries: Vec<&ActiveEntry<M>> = self.active.values().collect();
        entries.sort_by(|a, b| b.priority.cmp(&a.priority));

        for entry in &entries {
            if entry.listener.captures() {
                return entry.listener.hints();
            }
        }
        entries.iter().flat_map(|e| e.listener.hints()).collect()
    }

    /// Returns true if any active listener is currently capturing.
    pub fn has_capture(&self) -> bool {
        self.active.values().any(|e| e.listener.captures())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    // ------------------------------------------------------------------
    // Test helpers
    // ------------------------------------------------------------------

    fn key_event(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// A simple message type for tests.
    #[derive(Debug, PartialEq, Clone)]
    enum TestMsg {
        A,
        B,
        Mapped(Box<TestMsg>),
    }

    /// Configuration for a TestRecipe / TestListener pair.
    struct TestConfig {
        id: SubscriptionId,
        respond_to: KeyCode,
        msg: TestMsg,
        priority: u8,
        captures: bool,
        hints: Vec<KeybindHint>,
    }

    impl TestConfig {
        fn new(id: SubscriptionId, respond_to: KeyCode, msg: TestMsg) -> Self {
            Self {
                id,
                respond_to,
                msg,
                priority: 0,
                captures: false,
                hints: vec![],
            }
        }

        fn with_priority(mut self, p: u8) -> Self {
            self.priority = p;
            self
        }

        fn with_captures(mut self) -> Self {
            self.captures = true;
            self
        }

        fn with_hints(mut self, hints: Vec<KeybindHint>) -> Self {
            self.hints = hints;
            self
        }
    }

    /// A concrete recipe for tests.
    struct TestRecipe(TestConfig);

    impl Recipe for TestRecipe {
        type Output = TestMsg;

        fn id(&self) -> SubscriptionId {
            self.0.id
        }

        fn into_listener(self: Box<Self>) -> Box<dyn Listener<Output = TestMsg>> {
            Box::new(TestListener(self.0))
        }
    }

    /// A concrete listener for tests.
    struct TestListener(TestConfig);

    impl Listener for TestListener {
        type Output = TestMsg;

        fn process(&mut self, event: &AppEvent) -> Option<TestMsg> {
            match event {
                AppEvent::Key(k) if k.code == self.0.respond_to => Some(self.0.msg.clone()),
                _ => None,
            }
        }

        fn hints(&self) -> Vec<KeybindHint> {
            self.0.hints.clone()
        }

        fn priority(&self) -> u8 {
            self.0.priority
        }

        fn captures(&self) -> bool {
            self.0.captures
        }
    }

    fn make_sub(config: TestConfig) -> Subscription<TestMsg> {
        Subscription::from_recipe(TestRecipe(config))
    }

    // ------------------------------------------------------------------
    // sync() lifecycle tests
    // ------------------------------------------------------------------

    #[test]
    fn sync_activates_new_ids() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        let sub = make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A));

        mgr.sync(sub);

        assert!(mgr.active.contains_key(&id), "new ID should be activated");
    }

    #[test]
    fn sync_drops_removed_ids() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        let sub = make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A));
        mgr.sync(sub);
        assert!(mgr.active.contains_key(&id));

        // Sync with empty subscription — id should be removed.
        mgr.sync(Subscription::none());
        assert!(!mgr.active.contains_key(&id), "removed ID should be dropped");
    }

    #[test]
    fn sync_keeps_existing_ids() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();

        // First sync activates it.
        let sub1 = make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A));
        mgr.sync(sub1);

        // Second sync with same id — should still be present.
        let sub2 = make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A));
        mgr.sync(sub2);

        assert!(
            mgr.active.contains_key(&id),
            "existing ID should be preserved"
        );
        assert_eq!(mgr.active.len(), 1);
    }

    #[test]
    fn sync_does_not_restart_existing_listener() {
        // Verify that a listener with the same ID is NOT re-created between
        // syncs (it keeps its state). We test this indirectly: a listener that
        // was activated on the first sync should still respond correctly after
        // a second sync passes the same ID.
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();

        mgr.sync(make_sub(TestConfig::new(id, KeyCode::Char('x'), TestMsg::A)));
        // Second sync — same ID.
        mgr.sync(make_sub(TestConfig::new(id, KeyCode::Char('x'), TestMsg::A)));

        let result = mgr.process(&key_event(KeyCode::Char('x')));
        assert_eq!(result, Some(TestMsg::A));
    }

    // ------------------------------------------------------------------
    // process() priority ordering
    // ------------------------------------------------------------------

    #[test]
    fn process_higher_priority_first() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_low = SubscriptionId::unique();
        let id_high = SubscriptionId::unique();

        // Both respond to 'a'. High priority should win.
        let low = make_sub(
            TestConfig::new(id_low, KeyCode::Char('a'), TestMsg::A).with_priority(1),
        );
        let high = make_sub(
            TestConfig::new(id_high, KeyCode::Char('a'), TestMsg::B).with_priority(10),
        );

        mgr.sync(Subscription::batch([low, high]));

        let result = mgr.process(&key_event(KeyCode::Char('a')));
        assert_eq!(result, Some(TestMsg::B), "high priority listener should win");
    }

    #[test]
    fn process_returns_none_when_no_match() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        mgr.sync(make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A)));

        let result = mgr.process(&key_event(KeyCode::Char('z')));
        assert_eq!(result, None);
    }

    // ------------------------------------------------------------------
    // process() capture behavior
    // ------------------------------------------------------------------

    #[test]
    fn capturing_listener_blocks_lower_priority() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_capturer = SubscriptionId::unique();
        let id_lower = SubscriptionId::unique();

        // High-priority capturer does NOT match 'z' but captures anyway.
        let capturer = make_sub(
            TestConfig::new(id_capturer, KeyCode::Char('x'), TestMsg::A)
                .with_priority(10)
                .with_captures(),
        );
        // Lower priority would match 'z' if given the chance.
        let lower = make_sub(
            TestConfig::new(id_lower, KeyCode::Char('z'), TestMsg::B).with_priority(1),
        );

        mgr.sync(Subscription::batch([capturer, lower]));

        // Send 'z' — capturer doesn't match but captures, blocking lower.
        let result = mgr.process(&key_event(KeyCode::Char('z')));
        assert_eq!(result, None, "capturer should block lower priority listener");
    }

    #[test]
    fn non_capturing_listener_does_not_block() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_high = SubscriptionId::unique();
        let id_low = SubscriptionId::unique();

        // High priority, non-capturing, does NOT match 'z'.
        let high = make_sub(
            TestConfig::new(id_high, KeyCode::Char('x'), TestMsg::A).with_priority(10),
        );
        // Lower priority matches 'z'.
        let low = make_sub(
            TestConfig::new(id_low, KeyCode::Char('z'), TestMsg::B).with_priority(1),
        );

        mgr.sync(Subscription::batch([high, low]));

        let result = mgr.process(&key_event(KeyCode::Char('z')));
        assert_eq!(
            result,
            Some(TestMsg::B),
            "non-capturing listener should not block lower priority"
        );
    }

    // ------------------------------------------------------------------
    // map() composition
    // ------------------------------------------------------------------

    #[test]
    fn map_transforms_message() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();

        let sub: Subscription<TestMsg> =
            make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A))
                .map(|m| TestMsg::Mapped(Box::new(m)));

        mgr.sync(sub);

        let result = mgr.process(&key_event(KeyCode::Char('a')));
        assert_eq!(
            result,
            Some(TestMsg::Mapped(Box::new(TestMsg::A))),
            "map should transform the produced message"
        );
    }

    #[test]
    fn map_preserves_id_for_lifecycle() {
        // A mapped recipe must preserve the original ID so that sync can
        // correctly track it across frames.
        let id = SubscriptionId::unique();
        let sub: Subscription<TestMsg> =
            make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A))
                .map(|m| TestMsg::Mapped(Box::new(m)));

        let recipes = sub.into_recipes();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].id(), id, "map must preserve the original SubscriptionId");
    }

    #[test]
    fn double_map_composes_correctly() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();

        let sub: Subscription<TestMsg> =
            make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A))
                .map(|m| TestMsg::Mapped(Box::new(m)))
                .map(|m| TestMsg::Mapped(Box::new(m)));

        mgr.sync(sub);

        let result = mgr.process(&key_event(KeyCode::Char('a')));
        assert_eq!(
            result,
            Some(TestMsg::Mapped(Box::new(TestMsg::Mapped(Box::new(TestMsg::A))))),
            "double map should compose correctly"
        );
    }

    // ------------------------------------------------------------------
    // hints() collection
    // ------------------------------------------------------------------

    #[test]
    fn hints_returns_all_when_no_capture() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_a = SubscriptionId::unique();
        let id_b = SubscriptionId::unique();

        let hint_a = KeybindHint::new("a", "do A");
        let hint_b = KeybindHint::new("b", "do B");

        let sub_a = make_sub(
            TestConfig::new(id_a, KeyCode::Char('a'), TestMsg::A)
                .with_hints(vec![hint_a.clone()]),
        );
        let sub_b = make_sub(
            TestConfig::new(id_b, KeyCode::Char('b'), TestMsg::B)
                .with_hints(vec![hint_b.clone()]),
        );

        mgr.sync(Subscription::batch([sub_a, sub_b]));

        let hints = mgr.hints();
        assert!(hints.contains(&hint_a), "hint_a should be present");
        assert!(hints.contains(&hint_b), "hint_b should be present");
    }

    #[test]
    fn hints_returns_only_capturing_listener_hints_when_capture_active() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_capturer = SubscriptionId::unique();
        let id_lower = SubscriptionId::unique();

        let hint_capture = KeybindHint::new("Esc", "close dialog");
        let hint_lower = KeybindHint::new("q", "quit");

        let capturer = make_sub(
            TestConfig::new(id_capturer, KeyCode::Esc, TestMsg::A)
                .with_priority(10)
                .with_captures()
                .with_hints(vec![hint_capture.clone()]),
        );
        let lower = make_sub(
            TestConfig::new(id_lower, KeyCode::Char('q'), TestMsg::B)
                .with_priority(1)
                .with_hints(vec![hint_lower.clone()]),
        );

        mgr.sync(Subscription::batch([capturer, lower]));

        let hints = mgr.hints();
        assert!(hints.contains(&hint_capture), "capturing listener hint should be shown");
        assert!(
            !hints.contains(&hint_lower),
            "lower listener hint should NOT be shown when capture is active"
        );
    }

    #[test]
    fn hints_empty_when_no_subscriptions() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        mgr.sync(Subscription::none());
        assert!(mgr.hints().is_empty());
    }

    // ------------------------------------------------------------------
    // has_capture
    // ------------------------------------------------------------------

    #[test]
    fn has_capture_true_when_capturing_listener_present() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        mgr.sync(make_sub(
            TestConfig::new(id, KeyCode::Char('a'), TestMsg::A).with_captures(),
        ));
        assert!(mgr.has_capture());
    }

    #[test]
    fn has_capture_false_when_no_capturing_listener() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        mgr.sync(make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A)));
        assert!(!mgr.has_capture());
    }

    // ------------------------------------------------------------------
    // batch / none
    // ------------------------------------------------------------------

    #[test]
    fn batch_merges_recipes() {
        let id1 = SubscriptionId::unique();
        let id2 = SubscriptionId::unique();

        let sub1 = make_sub(TestConfig::new(id1, KeyCode::Char('a'), TestMsg::A));
        let sub2 = make_sub(TestConfig::new(id2, KeyCode::Char('b'), TestMsg::B));

        let batched = Subscription::batch([sub1, sub2]);
        assert_eq!(batched.into_recipes().len(), 2);
    }

    #[test]
    fn none_has_no_recipes() {
        let sub: Subscription<TestMsg> = Subscription::none();
        assert!(sub.into_recipes().is_empty());
    }
}
