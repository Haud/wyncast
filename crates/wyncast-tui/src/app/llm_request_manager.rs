use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::llm::client::LlmClient;
use crate::protocol::LlmEvent;

/// Manages active LLM streaming requests by ID.
///
/// Each request gets a unique monotonic ID (which doubles as the `generation`
/// field in `LlmEvent`). The manager tracks active request handles so they
/// can be individually cancelled or bulk-cancelled.
pub struct LlmRequestManager {
    next_id: u64,
    active: HashMap<u64, JoinHandle<()>>,
}

impl Default for LlmRequestManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmRequestManager {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            active: HashMap::new(),
        }
    }

    /// Spawn a new LLM streaming request. Returns the request ID.
    ///
    /// The returned ID is used as the `generation` field in `LlmEvent`,
    /// allowing the handler to route events to the correct UI component.
    pub fn start(
        &mut self,
        client: Arc<LlmClient>,
        system: String,
        user_content: String,
        max_tokens: u32,
        tx: mpsc::Sender<LlmEvent>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let handle = tokio::spawn(async move {
            if let Err(e) = client
                .stream_message(&system, &user_content, max_tokens, tx, id)
                .await
            {
                warn!("LLM request {} failed: {}", id, e);
            }
        });

        self.active.insert(id, handle);
        info!("Started LLM request {}", id);
        id
    }

    /// Cancel a specific request by aborting its task.
    pub fn cancel(&mut self, id: u64) {
        if let Some(handle) = self.active.remove(&id) {
            handle.abort();
            info!("Cancelled LLM request {}", id);
        }
    }

    /// Cancel all active requests.
    pub fn cancel_all(&mut self) {
        for (id, handle) in self.active.drain() {
            handle.abort();
            info!("Cancelled LLM request {} (cancel_all)", id);
        }
    }

    /// Check if a request ID is still active.
    pub fn is_active(&self, id: u64) -> bool {
        self.active.contains_key(&id)
    }

    /// Mark a request as complete (remove without aborting).
    ///
    /// Called when a terminal event (Complete/Error) is received.
    pub fn complete(&mut self, id: u64) {
        self.active.remove(&id);
    }

    /// Insert a dummy tracked entry for testing purposes.
    /// This allows tests to register an ID as active without spawning a real task.
    #[doc(hidden)]
    pub fn track_test_id(&mut self, id: u64) {
        self.active.insert(id, tokio::spawn(async {}));
    }

    /// Track an externally-spawned task handle.
    ///
    /// Used when the caller needs to spawn the task themselves (e.g., onboarding)
    /// but still wants it tracked for cancellation.
    pub fn track(&mut self, id: u64, handle: JoinHandle<()>) {
        self.active.insert(id, handle);
    }

    /// Allocate a unique ID without tracking a task handle.
    ///
    /// Used by onboarding where the task is spawned externally but still
    /// needs a unique generation counter.
    pub fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_id_increments() {
        let mut mgr = LlmRequestManager::new();
        let id1 = mgr.allocate_id();
        let id2 = mgr.allocate_id();
        assert_ne!(id1, id2);
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn is_active_returns_false_for_unknown() {
        let mgr = LlmRequestManager::new();
        assert!(!mgr.is_active(999));
    }

    #[test]
    fn complete_removes_without_panic() {
        let mut mgr = LlmRequestManager::new();
        // Completing a non-existent ID should not panic
        mgr.complete(999);
    }

    #[test]
    fn cancel_removes_without_panic() {
        let mut mgr = LlmRequestManager::new();
        // Cancelling a non-existent ID should not panic
        mgr.cancel(999);
    }

    #[tokio::test]
    async fn start_tracks_request() {
        let mut mgr = LlmRequestManager::new();
        let client = Arc::new(LlmClient::Disabled);
        let (tx, _rx) = mpsc::channel(16);

        let id = mgr.start(
            client,
            "system".into(),
            "user".into(),
            100,
            tx,
        );

        assert!(mgr.is_active(id));

        mgr.cancel(id);
        assert!(!mgr.is_active(id));
    }

    #[tokio::test]
    async fn cancel_all_clears_active() {
        let mut mgr = LlmRequestManager::new();
        let client = Arc::new(LlmClient::Disabled);
        let (tx, _rx) = mpsc::channel(16);

        let id1 = mgr.start(client.clone(), "s".into(), "u".into(), 100, tx.clone());
        let id2 = mgr.start(client, "s".into(), "u".into(), 100, tx);

        assert!(mgr.is_active(id1));
        assert!(mgr.is_active(id2));

        mgr.cancel_all();

        assert!(!mgr.is_active(id1));
        assert!(!mgr.is_active(id2));
    }
}
