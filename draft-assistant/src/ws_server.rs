// WebSocket server for communication with the Firefox extension.

use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

/// Events emitted by the WebSocket server to the application layer.
#[derive(Debug)]
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

        if tx.send(WsEvent::Connected { addr: addr_str.clone() }).await.is_err() {
            // Receiver dropped; stop the server.
            break;
        }

        let (_write, mut read) = ws_stream.split();

        loop {
            match read.next().await {
                Some(Ok(Message::Text(text))) => {
                    if tx.send(WsEvent::Message(text.to_string())).await.is_err() {
                        return Ok(());
                    }
                }
                Some(Ok(Message::Close(_))) => {
                    info!("Client {addr_str} sent close frame");
                    break;
                }
                Some(Err(e)) => {
                    warn!("WebSocket error from {addr_str}: {e}");
                    break;
                }
                None => {
                    // Stream ended (connection closed without close frame).
                    info!("Connection from {addr_str} ended");
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Helper: start the server on an ephemeral port and return the port number.
    async fn start_server(tx: mpsc::Sender<WsEvent>) -> u16 {
        // Bind to port 0 to get an ephemeral port, then extract the actual port.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Drop the listener so the server can re-bind on the same port.
        drop(listener);
        tokio::spawn(run(port, tx));
        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        port
    }

    #[tokio::test]
    async fn server_accepts_connection_and_emits_connected_event() {
        let (tx, mut rx) = mpsc::channel(64);
        let port = start_server(tx).await;

        let url = format!("ws://127.0.0.1:{port}");
        let (_ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for Connected event")
            .expect("channel closed unexpectedly");

        match event {
            WsEvent::Connected { addr } => {
                assert!(!addr.is_empty(), "addr should not be empty");
            }
            other => panic!("expected Connected event, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn text_message_arrives_on_channel() {
        let (tx, mut rx) = mpsc::channel(64);
        let port = start_server(tx).await;

        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let (mut write, _read) = ws.split();

        // Drain the Connected event.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        let payload = r#"{"type":"pick","player":"Mike Trout"}"#;
        write.send(WsMessage::Text(payload.into())).await.unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timed out waiting for Message event")
            .expect("channel closed unexpectedly");

        match event {
            WsEvent::Message(text) => {
                assert_eq!(text, payload);
            }
            other => panic!("expected Message event, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reconnection_works_after_disconnect() {
        let (tx, mut rx) = mpsc::channel(64);
        let port = start_server(tx).await;

        let url = format!("ws://127.0.0.1:{port}");

        // First connection.
        {
            let (ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut write, _read) = ws.split();

            // Drain Connected.
            let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(evt, WsEvent::Connected { .. }));

            // Close the connection.
            write.send(WsMessage::Close(None)).await.unwrap();

            // Wait for Disconnected.
            let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(evt, WsEvent::Disconnected));
        }

        // Small delay to let server loop back to accept.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Second connection.
        {
            let (ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut write, _read) = ws.split();

            // Should get a new Connected event.
            let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(evt, WsEvent::Connected { .. }));

            // Send a message on the new connection to prove it works.
            let payload = r#"{"type":"reconnected"}"#;
            write.send(WsMessage::Text(payload.into())).await.unwrap();

            let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap();
            match evt {
                WsEvent::Message(text) => assert_eq!(text, payload),
                other => panic!("expected Message event, got: {other:?}"),
            }
        }
    }
}
