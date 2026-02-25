// WebSocket server for communication with the Firefox extension.

use futures_util::stream::{SplitStream, Stream};
use futures_util::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
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

/// Run the WebSocket server on the given port, forwarding events through `tx`.
///
/// Binds a TCP listener on `127.0.0.1:{port}` and accepts one connection at
/// a time. For each connection it performs the WebSocket handshake, then reads
/// text messages and forwards them as [`WsEvent::Message`]. The server runs
/// forever (until the task is cancelled or the process exits).
pub async fn run(port: u16, tx: mpsc::Sender<WsEvent>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    let local_addr = listener.local_addr()?;
    info!("WebSocket server listening on {local_addr}");

    loop {
        let (stream, addr) = listener.accept().await?;
        let addr_str = addr.to_string();
        info!("Accepted TCP connection from {addr_str}");

        let ws_stream = match tokio_tungstenite::accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                warn!("WebSocket handshake failed for {addr_str}: {e}");
                continue;
            }
        };

        if tx
            .send(WsEvent::Connected {
                addr: addr_str.clone(),
            })
            .await
            .is_err()
        {
            break;
        }

        let (_write, read) = ws_stream.split();
        if process_messages(read, &tx, &addr_str).await.is_err() {
            break;
        }

        if tx.send(WsEvent::Disconnected).await.is_err() {
            break;
        }
    }

    Ok(())
}

/// Process incoming WebSocket messages from a read stream, forwarding text
/// messages through `tx`. Returns `Err(())` if the channel is closed (receiver
/// dropped), signalling the caller to stop.
///
/// This function is generic over the stream type so it can be tested with
/// in-memory streams without opening TCP ports.
pub async fn process_messages<S>(
    mut read: SplitStream<WebSocketStream<S>>,
    tx: &mpsc::Sender<WsEvent>,
    addr: &str,
) -> Result<(), ()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(msg_result) = read.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                if tx.send(WsEvent::Message(text.to_string())).await.is_err() {
                    return Err(());
                }
            }
            Ok(Message::Close(_)) => {
                info!("Client {addr} sent close frame");
                break;
            }
            Err(e) => {
                warn!("WebSocket error from {addr}: {e}");
                break;
            }
            _ => {
                // Ignore Binary, Ping, Pong, Frame variants.
            }
        }
    }
    Ok(())
}

/// Process raw WebSocket [`Message`] items from any [`Stream`], forwarding
/// text payloads through `tx`. This is a pure-logic function that requires
/// no I/O and is the primary unit-test target.
pub async fn process_message_stream<St>(
    mut stream: St,
    tx: &mpsc::Sender<WsEvent>,
    addr: &str,
) -> Result<(), ()>
where
    St: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(msg_result) = stream.next().await {
        match msg_result {
            Ok(Message::Text(text)) => {
                if tx.send(WsEvent::Message(text.to_string())).await.is_err() {
                    return Err(());
                }
            }
            Ok(Message::Close(_)) => {
                info!("Client {addr} sent close frame");
                break;
            }
            Err(e) => {
                warn!("WebSocket error from {addr}: {e}");
                break;
            }
            _ => {
                // Ignore Binary, Ping, Pong, Frame variants.
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use tokio_tungstenite::tungstenite::Error as WsError;

    /// Helper: create a stream of Message results from a vec.
    fn mock_stream(
        messages: Vec<Result<Message, WsError>>,
    ) -> impl Stream<Item = Result<Message, WsError>> + Unpin {
        stream::iter(messages)
    }

    #[tokio::test]
    async fn text_message_forwarded_to_channel() {
        let (tx, mut rx) = mpsc::channel(64);
        let messages = vec![Ok(Message::Text("hello".into()))];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        let event = rx.recv().await.unwrap();
        assert_eq!(event, WsEvent::Message("hello".to_string()));
    }

    #[tokio::test]
    async fn multiple_messages_forwarded_in_order() {
        let (tx, mut rx) = mpsc::channel(64);
        let messages = vec![
            Ok(Message::Text("first".into())),
            Ok(Message::Text("second".into())),
            Ok(Message::Text("third".into())),
        ];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        assert_eq!(rx.recv().await.unwrap(), WsEvent::Message("first".into()));
        assert_eq!(
            rx.recv().await.unwrap(),
            WsEvent::Message("second".into())
        );
        assert_eq!(rx.recv().await.unwrap(), WsEvent::Message("third".into()));
    }

    #[tokio::test]
    async fn close_frame_stops_processing() {
        let (tx, mut rx) = mpsc::channel(64);
        let messages = vec![
            Ok(Message::Text("before_close".into())),
            Ok(Message::Close(None)),
            Ok(Message::Text("after_close_should_not_appear".into())),
        ];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            WsEvent::Message("before_close".into())
        );
        // Channel should have no more messages (close stopped processing).
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn error_stops_processing() {
        let (tx, mut rx) = mpsc::channel(64);
        let messages = vec![
            Ok(Message::Text("before_error".into())),
            Err(WsError::ConnectionClosed),
            Ok(Message::Text("after_error_should_not_appear".into())),
        ];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            WsEvent::Message("before_error".into())
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn binary_and_ping_messages_are_ignored() {
        let (tx, mut rx) = mpsc::channel(64);
        let messages = vec![
            Ok(Message::Binary(vec![1, 2, 3].into())),
            Ok(Message::Ping(vec![].into())),
            Ok(Message::Pong(vec![].into())),
            Ok(Message::Text("after_ignored".into())),
        ];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            WsEvent::Message("after_ignored".into())
        );
        // No other events should be in the channel.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn returns_err_when_channel_closed() {
        let (tx, rx) = mpsc::channel(64);
        drop(rx); // Close the receiver.

        let messages = vec![Ok(Message::Text("orphan".into()))];

        let result = process_message_stream(mock_stream(messages), &tx, "test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn empty_stream_completes_normally() {
        let (tx, mut rx) = mpsc::channel(64);
        let messages: Vec<Result<Message, WsError>> = vec![];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn json_payload_preserved_exactly() {
        let (tx, mut rx) = mpsc::channel(64);
        let payload = r#"{"type":"STATE_UPDATE","timestamp":123,"payload":{"picks":[]}}"#;
        let messages = vec![Ok(Message::Text(payload.into()))];

        process_message_stream(mock_stream(messages), &tx, "test")
            .await
            .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            WsEvent::Message(payload.to_string())
        );
    }
}
