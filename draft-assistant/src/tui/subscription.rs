// Subscription system: event routing infrastructure for the TUI.
//
// The subscription model decouples event producers (keyboard, tick, resize)
// from consumers (screen components). Each component declares a
// `Subscription<M>` describing which events it cares about; the runtime
// collects all subscriptions, diffs them on each frame, and routes events
// through the active listener set. Listeners are checked in declaration
// order (batch order); first match wins.
//
// # Key types
//
// - [`SubscriptionId`] — stable identity used to track lifecycle across frames
// - [`AppEvent`] — raw events the runtime feeds to listeners
// - [`Recipe`] — extensibility point; creates a [`Listener`] on activation
// - [`Listener`] — processes events; first match wins
// - [`Subscription<M>`] — generic container of recipes; supports `none`, `batch`, `map`
// - [`SubscriptionManager<M>`] — diffs subscriptions, activates/drops listeners, routes events

use std::collections::HashSet;

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
/// current subscription set. They receive every [`AppEvent`] in declaration
/// order; first match wins.
pub trait Listener {
    type Output;

    /// Process one event. Returns `Some(msg)` if the event was consumed.
    fn process(&mut self, event: &AppEvent) -> Option<Self::Output>;
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
///
/// Listeners are stored in declaration order (the order recipes appear in the
/// subscription batch). First match wins during event processing.
pub struct SubscriptionManager<M: 'static> {
    active: Vec<(SubscriptionId, ActiveEntry<M>)>,
}

struct ActiveEntry<M> {
    listener: Box<dyn Listener<Output = M>>,
}

impl<M: 'static> SubscriptionManager<M> {
    pub fn new() -> Self {
        Self { active: vec![] }
    }

    /// Diff the new subscription against the current active set.
    ///
    /// - New IDs are activated (recipe converted to listener).
    /// - Removed IDs are dropped.
    /// - Existing IDs are kept as-is (listener state preserved).
    /// - Order follows the new subscription's recipe order.
    pub fn sync(&mut self, subscription: Subscription<M>) {
        let new_recipes = subscription.into_recipes();
        let new_ids: HashSet<SubscriptionId> = new_recipes.iter().map(|r| r.id()).collect();

        // Keep existing entries that are still in the new set.
        self.active.retain(|(id, _)| new_ids.contains(id));

        // Rebuild in new recipe order, reusing existing listeners.
        let mut new_active: Vec<(SubscriptionId, ActiveEntry<M>)> = Vec::new();
        for recipe in new_recipes {
            let id = recipe.id();
            if let Some(pos) = self.active.iter().position(|(eid, _)| *eid == id) {
                // Reuse existing listener (preserves state).
                new_active.push(self.active.swap_remove(pos));
            } else {
                // New subscription — activate.
                let listener = recipe.into_listener();
                new_active.push((id, ActiveEntry { listener }));
            }
        }
        self.active = new_active;
    }

    /// Route an event through all active listeners in declaration order.
    ///
    /// Returns the first message produced (first match wins).
    pub fn process(&mut self, event: &AppEvent) -> Option<M> {
        for (_, entry) in &mut self.active {
            if let Some(msg) = entry.listener.process(event) {
                return Some(msg);
            }
        }
        None
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
    }

    impl TestConfig {
        fn new(id: SubscriptionId, respond_to: KeyCode, msg: TestMsg) -> Self {
            Self {
                id,
                respond_to,
                msg,
            }
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
    }

    fn make_sub(config: TestConfig) -> Subscription<TestMsg> {
        Subscription::from_recipe(TestRecipe(config))
    }

    /// Helper to check if a manager contains an entry with the given ID.
    fn has_id<M>(mgr: &SubscriptionManager<M>, id: SubscriptionId) -> bool {
        mgr.active.iter().any(|(eid, _)| *eid == id)
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

        assert!(has_id(&mgr, id), "new ID should be activated");
    }

    #[test]
    fn sync_drops_removed_ids() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        let sub = make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A));
        mgr.sync(sub);
        assert!(has_id(&mgr, id));

        // Sync with empty subscription — id should be removed.
        mgr.sync(Subscription::none());
        assert!(!has_id(&mgr, id), "removed ID should be dropped");
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

        assert!(has_id(&mgr, id), "existing ID should be preserved");
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
    // process() declaration-order routing
    // ------------------------------------------------------------------

    #[test]
    fn process_first_declared_wins() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_first = SubscriptionId::unique();
        let id_second = SubscriptionId::unique();

        // Both respond to 'a'. The first one in batch order should win.
        let first = make_sub(TestConfig::new(id_first, KeyCode::Char('a'), TestMsg::A));
        let second = make_sub(TestConfig::new(id_second, KeyCode::Char('a'), TestMsg::B));

        mgr.sync(Subscription::batch([first, second]));

        let result = mgr.process(&key_event(KeyCode::Char('a')));
        assert_eq!(
            result,
            Some(TestMsg::A),
            "first declared listener should win"
        );
    }

    #[test]
    fn process_falls_through_to_later_listener() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_first = SubscriptionId::unique();
        let id_second = SubscriptionId::unique();

        // First responds to 'x', second responds to 'z'.
        let first = make_sub(TestConfig::new(id_first, KeyCode::Char('x'), TestMsg::A));
        let second = make_sub(TestConfig::new(id_second, KeyCode::Char('z'), TestMsg::B));

        mgr.sync(Subscription::batch([first, second]));

        // Send 'z' — first doesn't match, falls through to second.
        let result = mgr.process(&key_event(KeyCode::Char('z')));
        assert_eq!(
            result,
            Some(TestMsg::B),
            "event should fall through to later listener"
        );
    }

    #[test]
    fn process_returns_none_when_no_match() {
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();
        let id = SubscriptionId::unique();
        mgr.sync(make_sub(TestConfig::new(id, KeyCode::Char('a'), TestMsg::A)));

        let result = mgr.process(&key_event(KeyCode::Char('z')));
        assert_eq!(result, None);
    }

    #[test]
    fn process_preserves_batch_order_across_syncs() {
        // Verify that re-syncing with the same batch preserves declaration order.
        let mut mgr: SubscriptionManager<TestMsg> = SubscriptionManager::new();

        let id_first = SubscriptionId::unique();
        let id_second = SubscriptionId::unique();

        let make_batch = || {
            let first = make_sub(TestConfig::new(id_first, KeyCode::Char('a'), TestMsg::A));
            let second = make_sub(TestConfig::new(id_second, KeyCode::Char('a'), TestMsg::B));
            Subscription::batch([first, second])
        };

        mgr.sync(make_batch());
        mgr.sync(make_batch()); // Re-sync with same order.

        let result = mgr.process(&key_event(KeyCode::Char('a')));
        assert_eq!(
            result,
            Some(TestMsg::A),
            "batch order should be preserved across syncs"
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
