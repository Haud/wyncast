use std::hash::Hash;
use std::sync::{Arc, Mutex};

use futures::stream::BoxStream;
use iced::Subscription;
use iced::advanced::subscription::{EventStream, Hasher, Recipe, from_recipe};
use tokio::sync::mpsc;
use wyncast_app::protocol::UiUpdate;

use crate::message::Message;

// ---------------------------------------------------------------------------
// Public API: takes receiver by value (used by unit tests and first boot).
// ---------------------------------------------------------------------------

/// Wraps `ui_rx` in a keyed [`Subscription`] that emits one [`Message::UiUpdate`]
/// per item received from the backend.
#[allow(dead_code)]
pub fn ui_subscription(rx: mpsc::Receiver<UiUpdate>) -> Subscription<Message> {
    from_recipe(UiUpdateRecipe { rx: Some(rx) })
}

// ---------------------------------------------------------------------------
// Internal helper: builds the subscription from an Arc-wrapped Option.
// The Arc is cloned each render cycle; the receiver is taken on first use.
// ---------------------------------------------------------------------------

pub fn ui_subscription_from_arc(
    rx_arc: Arc<Mutex<Option<mpsc::Receiver<UiUpdate>>>>,
) -> Subscription<Message> {
    let rx = rx_arc.lock().unwrap().take();
    from_recipe(UiUpdateRecipe { rx })
}

// ---------------------------------------------------------------------------
// Recipe implementation
// ---------------------------------------------------------------------------

struct UiUpdateRecipe {
    rx: Option<mpsc::Receiver<UiUpdate>>,
}

impl Recipe for UiUpdateRecipe {
    type Output = Message;

    fn hash(&self, state: &mut Hasher) {
        // All UiUpdateRecipes share the same identity regardless of receiver
        // state. This ensures subsequent renders don't restart the stream.
        std::any::TypeId::of::<UiUpdateRecipe>().hash(state);
    }

    fn stream(self: Box<Self>, _input: EventStream) -> BoxStream<'static, Self::Output> {
        match self.rx {
            Some(mut rx) => Box::pin(iced::stream::channel::<Message>(256, |mut out: futures::channel::mpsc::Sender<Message>| async move {
                while let Some(update) = rx.recv().await {
                    out.try_send(Message::UiUpdate(update)).ok();
                }
            })),
            // No receiver (duplicate subscription call): produce nothing.
            // Iced keeps the already-running stream by hash identity.
            None => Box::pin(futures::stream::empty::<Message>()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use tokio::sync::mpsc;
    use wyncast_app::protocol::UiUpdate;

    /// Bridge round-trips a single [`UiUpdate`] end-to-end via a fake channel.
    #[tokio::test]
    async fn round_trips_single_update() {
        let (tx, rx) = mpsc::channel::<UiUpdate>(1);

        // Build the recipe's stream directly (bypass the Subscription wrapper).
        let recipe = UiUpdateRecipe { rx: Some(rx) };
        let (_, event_rx) = futures::channel::mpsc::channel(0);
        let mut stream = Box::new(recipe).stream(event_rx.boxed());

        // Send one update and close the sender.
        tx.send(UiUpdate::NominationCleared).await.unwrap();
        drop(tx);

        let msg = stream.next().await;
        assert!(
            matches!(msg, Some(Message::UiUpdate(UiUpdate::NominationCleared))),
            "expected UiUpdate(NominationCleared), got {msg:?}",
        );
    }

    #[tokio::test]
    async fn empty_recipe_produces_no_items() {
        let recipe = UiUpdateRecipe { rx: None };
        let (_, event_rx) = futures::channel::mpsc::channel(0);
        let mut stream = Box::new(recipe).stream(event_rx.boxed());

        let msg = stream.next().await;
        assert!(msg.is_none(), "empty recipe should produce no items");
    }
}
