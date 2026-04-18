// TimerRecipe: a Recipe/Listener implementation for periodic timer events.
//
// `TimerRecipe<M>` fires a message at a fixed interval by inspecting
// `AppEvent::Tick` events. It ignores all other event variants. This proves
// that the Recipe/Listener infrastructure is generic — timer subscriptions
// flow through `SubscriptionManager` without touching `KeybindManager` or
// any keybinding code.
//
// Usage:
//
//   let sub = TimerRecipe::new(
//       SubscriptionId::unique(),
//       Duration::from_millis(500),
//       || AppMessage::Tick,
//   ).build();

use std::time::{Duration, Instant};

use super::{AppEvent, Listener, Recipe, Subscription, SubscriptionId};

// ---------------------------------------------------------------------------
// TimerRecipe
// ---------------------------------------------------------------------------

/// A `Recipe` that produces a message at a fixed interval.
///
/// The recipe fires by inspecting `AppEvent::Tick` events. Key events and any
/// other variants are silently ignored. The interval tracking state lives in
/// the [`TimerListener`] produced by [`into_listener`][Recipe::into_listener].
///
/// Timer subscriptions bypass `KeybindManager` entirely — they have no hints.
/// Pass the subscription directly to `Subscription::batch` without going
/// through `kb.subscribe()`.
pub struct TimerRecipe<M: 'static> {
    id: SubscriptionId,
    interval: Duration,
    into_message: fn() -> M,
}

impl<M: 'static> TimerRecipe<M> {
    /// Create a new timer recipe.
    ///
    /// - `id` — stable identity across frames (keep the same `SubscriptionId`
    ///   across calls to `App::subscription()` so the listener is not restarted
    ///   on every frame).
    /// - `interval` — how often the message fires.
    /// - `into_message` — a zero-argument function pointer that produces the
    ///   message on each tick.
    pub fn new(id: SubscriptionId, interval: Duration, into_message: fn() -> M) -> Self {
        Self {
            id,
            interval,
            into_message,
        }
    }

    /// Consume the recipe and wrap it in a `Subscription<M>`.
    pub fn build(self) -> Subscription<M> {
        Subscription::from_recipe(self)
    }
}

impl<M: 'static> Recipe for TimerRecipe<M> {
    type Output = M;

    fn id(&self) -> SubscriptionId {
        self.id
    }

    fn into_listener(self: Box<Self>) -> Box<dyn Listener<Output = M>> {
        Box::new(TimerListener {
            interval: self.interval,
            // Initialize to now so the first tick fires after one interval,
            // not immediately.
            last_tick: Instant::now(),
            into_message: self.into_message,
        })
    }
}

// ---------------------------------------------------------------------------
// TimerListener (internal)
// ---------------------------------------------------------------------------

struct TimerListener<M: 'static> {
    interval: Duration,
    last_tick: Instant,
    into_message: fn() -> M,
}

impl<M: 'static> Listener for TimerListener<M> {
    type Output = M;

    fn process(&mut self, event: &AppEvent) -> Option<M> {
        // Only Tick events are relevant; ignore Key and any future variants.
        let AppEvent::Tick(now) = event else {
            return None;
        };
        if now.duration_since(self.last_tick) >= self.interval {
            self.last_tick = *now;
            Some((self.into_message)())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::subscription::{AppEvent, SubscriptionManager};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[derive(Debug, PartialEq, Clone)]
    enum Msg {
        Ticked,
    }

    fn make_timer(interval: Duration) -> SubscriptionManager<Msg> {
        let mut mgr = SubscriptionManager::new();
        let sub = TimerRecipe::new(SubscriptionId::unique(), interval, || Msg::Ticked).build();
        mgr.sync(sub);
        mgr
    }

    // 1. Timer fires after the interval has elapsed.
    #[test]
    fn timer_fires_after_interval() {
        let interval = Duration::from_millis(100);
        let mut mgr = make_timer(interval);

        // A tick that is exactly the interval after the listener was created
        // should fire. We fake "last_tick" by using an Instant that is far
        // enough in the past: `Instant::now() + interval` is the "now" we
        // supply so that `now - last_tick >= interval`.
        let past = Instant::now() - interval;
        // Supply a "now" that is `interval` after `past`.
        let now = past + interval;

        // The listener's last_tick is set to Instant::now() at creation, which
        // is approximately equal to `past`. We cannot directly set it, so
        // instead we use `Instant::now() + interval` to get a point in time
        // that is definitely >= interval ahead of when the listener was created.
        let result = mgr.process(&AppEvent::Tick(Instant::now() + interval));
        assert_eq!(result, Some(Msg::Ticked), "timer should fire after interval elapses");
        let _ = now; // used for clarity only
    }

    // 2. Timer does NOT fire before the interval elapses.
    #[test]
    fn timer_does_not_fire_before_interval() {
        let interval = Duration::from_secs(60); // very long interval
        let mut mgr = make_timer(interval);

        // Supply current time — well before the 60-second interval.
        let result = mgr.process(&AppEvent::Tick(Instant::now()));
        assert_eq!(result, None, "timer should not fire before interval elapses");
    }

    // 3. Timer fires again on the second interval.
    #[test]
    fn timer_fires_again_on_second_interval() {
        let interval = Duration::from_millis(100);
        let mut mgr = make_timer(interval);

        // First firing.
        let first = mgr.process(&AppEvent::Tick(Instant::now() + interval));
        assert_eq!(first, Some(Msg::Ticked), "should fire on first interval");

        // A second tick at (interval * 2) after creation should fire again.
        // After the first firing, last_tick was updated to the instant we
        // passed. Supply an instant that is `interval` ahead of that.
        let second = mgr.process(&AppEvent::Tick(Instant::now() + interval * 2));
        assert_eq!(second, Some(Msg::Ticked), "should fire again on second interval");
    }

    // 4. Non-Tick events (Key events) are ignored by TimerListener.
    #[test]
    fn timer_ignores_key_events() {
        let interval = Duration::from_millis(1); // very short — would fire on Tick
        let mut mgr = make_timer(interval);

        let key_event = AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let result = mgr.process(&key_event);
        assert_eq!(result, None, "timer should ignore Key events");
    }
}
