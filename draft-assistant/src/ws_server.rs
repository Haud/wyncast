// WebSocket server for communication with the Firefox extension.

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

/// Events emitted by the WebSocket server to the application layer.
#[derive(Debug, PartialEq)]
pub enum WsEvent {
    /// A new WebSocket client has connected.
    Connected { addr: String },
    /// The current WebSocket client has disconnected.
    Disconnected,
    /// A text message was received from the client (raw JSON string).
    Message(String),
}

/// A connection that can be read for WebSocket messages.
#[async_trait]
pub trait WsConnection: Send {
    /// Read the next WebSocket message, or `None` if the connection is closed.
    async fn next_message(&mut self) -> Option<Result<Message, String>>;
}

/// A listener that accepts incoming WebSocket connections.
#[async_trait]
pub trait WsListener: Send {
    /// The connection type produced by this listener.
    type Connection: WsConnection;

    /// Wait for and accept the next client connection. Returns the connection
    /// and a human-readable address string.
    async fn accept(&mut self) -> anyhow::Result<(Self::Connection, String)>;
}

/// Run the WebSocket server using the provided listener, forwarding events
/// through `tx`.
///
/// Accepts one connection at a time. For each connection it reads text messages
/// and forwards them as [`WsEvent::Message`]. The server runs forever (until
/// the task is cancelled, the channel is closed, or an accept error occurs).
pub async fn run<L: WsListener>(
    mut listener: L,
    tx: mpsc::Sender<WsEvent>,
) -> anyhow::Result<()> {
    loop {
        let (mut conn, addr_str) = listener.accept().await?;
        info!("Accepted connection from {addr_str}");

        if tx
            .send(WsEvent::Connected {
                addr: addr_str.clone(),
            })
            .await
            .is_err()
        {
            break;
        }

        // Read messages until the connection closes or errors.
        while let Some(msg_result) = conn.next_message().await {
            match msg_result {
                Ok(Message::Text(text)) => {
                    if tx.send(WsEvent::Message(text.to_string())).await.is_err() {
                        return Ok(());
                    }
                }
                Ok(Message::Close(_)) => {
                    info!("Client {addr_str} sent close frame");
                    break;
                }
                Err(e) => {
                    warn!("WebSocket error from {addr_str}: {e}");
                    break;
                }
                _ => {
                    // Ignore Binary, Ping, Pong, Frame variants.
                }
            }
        }

        if tx.send(WsEvent::Disconnected).await.is_err() {
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Production implementation: real TCP + tungstenite
// ---------------------------------------------------------------------------

/// A real WebSocket connection backed by a TCP stream and tungstenite.
pub struct TungsteniteConnection {
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    >,
}

#[async_trait]
impl WsConnection for TungsteniteConnection {
    async fn next_message(&mut self) -> Option<Result<Message, String>> {
        self.read
            .next()
            .await
            .map(|r| r.map_err(|e| e.to_string()))
    }
}

/// A real TCP listener that performs WebSocket handshakes via tungstenite.
pub struct TungsteniteListener {
    listener: TcpListener,
}

impl TungsteniteListener {
    /// Bind a TCP listener on `127.0.0.1:{port}` and return a new
    /// `TungsteniteListener`.
    pub async fn bind(port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
        let local_addr = listener.local_addr()?;
        info!("WebSocket server listening on {local_addr}");
        Ok(Self { listener })
    }
}

#[async_trait]
impl WsListener for TungsteniteListener {
    type Connection = TungsteniteConnection;

    async fn accept(&mut self) -> anyhow::Result<(TungsteniteConnection, String)> {
        loop {
            let (stream, addr) = self.listener.accept().await?;
            let addr_str = addr.to_string();

            match tokio_tungstenite::accept_async(stream).await {
                Ok(ws) => {
                    let (_write, read) = ws.split();
                    return Ok((TungsteniteConnection { read }, addr_str));
                }
                Err(e) => {
                    warn!("WebSocket handshake failed for {addr_str}: {e}");
                    continue;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tokio_tungstenite::tungstenite::Error as WsError;

    // -----------------------------------------------------------------------
    // Mock implementations
    // -----------------------------------------------------------------------

    /// A mock WebSocket connection that yields pre-configured messages.
    struct MockConnection {
        messages: VecDeque<Result<Message, String>>,
    }

    impl MockConnection {
        fn new(messages: Vec<Result<Message, String>>) -> Self {
            Self {
                messages: messages.into(),
            }
        }
    }

    #[async_trait]
    impl WsConnection for MockConnection {
        async fn next_message(&mut self) -> Option<Result<Message, String>> {
            self.messages.pop_front()
        }
    }

    /// A mock listener that yields pre-configured connections, then errors.
    struct MockListener {
        connections: VecDeque<(MockConnection, String)>,
    }

    impl MockListener {
        fn new(connections: Vec<(MockConnection, String)>) -> Self {
            Self {
                connections: connections.into(),
            }
        }
    }

    #[async_trait]
    impl WsListener for MockListener {
        type Connection = MockConnection;

        async fn accept(&mut self) -> anyhow::Result<(MockConnection, String)> {
            self.connections
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no more mock connections"))
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Collect all events from the channel (non-blocking after server exits).
    fn drain_events(rx: &mut mpsc::Receiver<WsEvent>) -> Vec<WsEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        events
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn single_text_message_forwarded() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn = MockConnection::new(vec![Ok(Message::Text("hello".into()))]);
        let listener = MockListener::new(vec![(conn, "mock:1234".into())]);

        // run() will process one connection then fail on next accept (no more mocks).
        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            WsEvent::Connected {
                addr: "mock:1234".into()
            }
        );
        assert_eq!(events[1], WsEvent::Message("hello".into()));
        assert_eq!(events[2], WsEvent::Disconnected);
    }

    #[tokio::test]
    async fn multiple_messages_forwarded_in_order() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn = MockConnection::new(vec![
            Ok(Message::Text("first".into())),
            Ok(Message::Text("second".into())),
            Ok(Message::Text("third".into())),
        ]);
        let listener = MockListener::new(vec![(conn, "mock:5678".into())]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert_eq!(events[1], WsEvent::Message("first".into()));
        assert_eq!(events[2], WsEvent::Message("second".into()));
        assert_eq!(events[3], WsEvent::Message("third".into()));
    }

    #[tokio::test]
    async fn close_frame_stops_connection_processing() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn = MockConnection::new(vec![
            Ok(Message::Text("before_close".into())),
            Ok(Message::Close(None)),
            Ok(Message::Text("should_not_appear".into())),
        ]);
        let listener = MockListener::new(vec![(conn, "mock:1".into())]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert!(events.contains(&WsEvent::Message("before_close".into())));
        assert!(!events.contains(&WsEvent::Message("should_not_appear".into())));
        assert!(events.contains(&WsEvent::Disconnected));
    }

    #[tokio::test]
    async fn error_stops_connection_processing() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn = MockConnection::new(vec![
            Ok(Message::Text("before_error".into())),
            Err(WsError::ConnectionClosed.to_string()),
            Ok(Message::Text("should_not_appear".into())),
        ]);
        let listener = MockListener::new(vec![(conn, "mock:2".into())]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert!(events.contains(&WsEvent::Message("before_error".into())));
        assert!(!events.contains(&WsEvent::Message("should_not_appear".into())));
        assert!(events.contains(&WsEvent::Disconnected));
    }

    #[tokio::test]
    async fn binary_ping_pong_messages_are_ignored() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn = MockConnection::new(vec![
            Ok(Message::Binary(vec![1, 2, 3].into())),
            Ok(Message::Ping(vec![].into())),
            Ok(Message::Pong(vec![].into())),
            Ok(Message::Text("after_ignored".into())),
        ]);
        let listener = MockListener::new(vec![(conn, "mock:3".into())]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        // Should only have Connected, Message("after_ignored"), Disconnected
        let msg_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, WsEvent::Message(_)))
            .collect();
        assert_eq!(msg_events.len(), 1);
        assert_eq!(msg_events[0], &WsEvent::Message("after_ignored".into()));
    }

    #[tokio::test]
    async fn server_stops_when_channel_closed() {
        let (tx, rx) = mpsc::channel(64);
        drop(rx); // Close the receiver immediately.

        let conn = MockConnection::new(vec![Ok(Message::Text("orphan".into()))]);
        let listener = MockListener::new(vec![(conn, "mock:4".into())]);

        // run() should return Ok(()) because channel-closed is a graceful exit.
        let result = run(listener, tx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn empty_connection_emits_connected_and_disconnected() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn = MockConnection::new(vec![]); // No messages at all.
        let listener = MockListener::new(vec![(conn, "mock:5".into())]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert_eq!(
            events[0],
            WsEvent::Connected {
                addr: "mock:5".into()
            }
        );
        assert_eq!(events[1], WsEvent::Disconnected);
    }

    #[tokio::test]
    async fn multiple_sequential_connections() {
        let (tx, mut rx) = mpsc::channel(64);
        let conn1 = MockConnection::new(vec![Ok(Message::Text("from_client_1".into()))]);
        let conn2 = MockConnection::new(vec![Ok(Message::Text("from_client_2".into()))]);
        let listener = MockListener::new(vec![
            (conn1, "mock:100".into()),
            (conn2, "mock:200".into()),
        ]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert_eq!(
            events[0],
            WsEvent::Connected {
                addr: "mock:100".into()
            }
        );
        assert_eq!(events[1], WsEvent::Message("from_client_1".into()));
        assert_eq!(events[2], WsEvent::Disconnected);
        assert_eq!(
            events[3],
            WsEvent::Connected {
                addr: "mock:200".into()
            }
        );
        assert_eq!(events[4], WsEvent::Message("from_client_2".into()));
        assert_eq!(events[5], WsEvent::Disconnected);
    }

    #[tokio::test]
    async fn json_payload_preserved_exactly() {
        let (tx, mut rx) = mpsc::channel(64);
        let payload = r#"{"type":"STATE_UPDATE","timestamp":123,"payload":{"picks":[]}}"#;
        let conn = MockConnection::new(vec![Ok(Message::Text(payload.into()))]);
        let listener = MockListener::new(vec![(conn, "mock:6".into())]);

        let _ = run(listener, tx).await;

        let events = drain_events(&mut rx);
        assert_eq!(events[1], WsEvent::Message(payload.to_string()));
    }
}
