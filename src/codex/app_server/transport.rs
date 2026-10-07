//! WebSocket transport for the shared Codex app-server daemon.
//!
//! The managed daemon listens on a Unix-domain control socket and requires a
//! WebSocket upgrade (`ws://localhost/rpc`) before JSON-RPC traffic.  Each
//! JSON-RPC message is carried in one WebSocket text frame.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt, stream::{SplitSink, SplitStream}};
use tokio::{net::UnixStream, sync::Mutex};
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::Message as WsMessage};
use tracing::{debug, warn};

use super::protocol::{JsonRpcError, Message};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const UDS_WEBSOCKET_HANDSHAKE_URL: &str = "ws://localhost/rpc";

type DaemonWebSocket = WebSocketStream<UnixStream>;
type DaemonWriter = SplitSink<DaemonWebSocket, WsMessage>;
pub(crate) type DaemonReader = SplitStream<DaemonWebSocket>;

pub(crate) struct StdioTransport {
    writer: Mutex<DaemonWriter>,
    reader: Mutex<Option<DaemonReader>>,
}

impl StdioTransport {
    /// Connect to the official app-server daemon control socket and complete
    /// the WebSocket upgrade.  The other parameters are retained in the
    /// signature so the supervisor wiring stays stable while the transport no
    /// longer owns a child process.
    pub(crate) async fn connect(
        _codex_binary: &std::path::Path,
        codex_home: &std::path::Path,
        _sqlite_home: &std::path::Path,
        _extra_path: Option<&std::ffi::OsStr>,
    ) -> Result<Self> {
        let socket_path = codex_home
            .join("app-server-control")
            .join("app-server-control.sock");
        let endpoint = format!("unix://{}", socket_path.display());

        let stream = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(&socket_path))
            .await
            .with_context(|| format!("timed out connecting to {endpoint}"))?
            .with_context(|| format!("failed to connect to {endpoint}"))?;

        let (websocket, _) = tokio::time::timeout(
            CONNECT_TIMEOUT,
            client_async(UDS_WEBSOCKET_HANDSHAKE_URL, stream),
        )
        .await
        .with_context(|| format!("timed out upgrading {endpoint} to WebSocket"))?
        .with_context(|| format!("failed to upgrade {endpoint} to WebSocket"))?;

        let (writer, reader) = websocket.split();
        Ok(Self {
            writer: Mutex::new(writer),
            reader: Mutex::new(Some(reader)),
        })
    }

    pub(crate) async fn take_reader(&self) -> Option<DaemonReader> {
        self.reader.lock().await.take()
    }

    pub(crate) async fn write_message(&self, value: serde_json::Value) -> Result<()> {
        let payload = serde_json::to_string(&value).context("serialize JSON-RPC message")?;
        self.writer
            .lock()
            .await
            .send(WsMessage::Text(payload.into()))
            .await
            .context("write daemon websocket")?;
        Ok(())
    }
}

/// Try to parse a single JSON text frame into a `Message`.
fn parse_frame(text: &str) -> Result<Message, ParseError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(ParseError::Invalid)?;
    let obj = value.as_object().ok_or(ParseError::NotObject)?;
    let method = obj
        .get("method")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let id = obj.get("id").cloned();

    if let (Some(method), Some(id)) = (method.clone(), id.clone()) {
        let params = obj
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        return Ok(Message::Request { id, method, params });
    }
    if let Some(method) = method {
        let params = obj
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        return Ok(Message::Notification { method, params });
    }
    if let Some(id) = id {
        let outcome = if let Some(err) = obj.get("error") {
            let err: JsonRpcError = serde_json::from_value(err.clone())
                .map_err(|e| ParseError::BadError(e.to_string()))?;
            Err(err)
        } else {
            let result = obj
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Ok(result)
        };
        return Ok(Message::Response { id, outcome });
    }
    Err(ParseError::Indeterminate)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ParseError {
    #[error("invalid JSON: {0}")]
    Invalid(serde_json::Error),
    #[error("message not a JSON object")]
    NotObject,
    #[error("message has no id or method")]
    Indeterminate,
    #[error("invalid error payload: {0}")]
    BadError(String),
}

/// Spawn a task that reads daemon WebSocket frames and invokes `handler` for
/// each parsed JSON-RPC message. Returns when the WebSocket closes.
pub(crate) fn spawn_reader<F>(
    mut reader: DaemonReader,
    mut handler: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut(Message) + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(frame) = reader.next().await {
            match frame {
                Ok(WsMessage::Text(text)) => match parse_frame(text.as_ref()) {
                    Ok(msg) => handler(msg),
                    Err(err) => {
                        warn!(error = %err, frame = %text, "dropping unparseable app-server frame");
                    }
                },
                Ok(WsMessage::Close(frame)) => {
                    debug!(?frame, "app-server websocket closed");
                    break;
                }
                Ok(WsMessage::Binary(_))
                | Ok(WsMessage::Ping(_))
                | Ok(WsMessage::Pong(_))
                | Ok(WsMessage::Frame(_)) => {}
                Err(err) => {
                    warn!(error = %err, "error reading app-server websocket");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_without_jsonrpc_field() {
        let m = parse_frame(r#"{"id":1,"result":{"ok":true}}"#).unwrap();
        match m {
            Message::Response { id, outcome } => {
                assert_eq!(id, serde_json::json!(1));
                assert!(outcome.is_ok());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_notification() {
        let m = parse_frame(r#"{"method":"turn/started","params":{"threadId":"t"}}"#).unwrap();
        match m {
            Message::Notification { method, params } => {
                assert_eq!(method, "turn/started");
                assert_eq!(params["threadId"], "t");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn parses_server_request_with_id() {
        let m = parse_frame(
            r#"{"id":0,"method":"item/commandExecution/requestApproval","params":{"threadId":"t"}}"#,
        )
        .unwrap();
        match m {
            Message::Request { id, method, .. } => {
                assert_eq!(id, serde_json::json!(0));
                assert_eq!(method, "item/commandExecution/requestApproval");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn rejects_non_object() {
        let err = parse_frame("[1,2,3]").unwrap_err();
        assert!(matches!(err, ParseError::NotObject));
    }

    #[test]
    fn rejects_indeterminate() {
        let err = parse_frame("{}").unwrap_err();
        assert!(matches!(err, ParseError::Indeterminate));
    }


    #[cfg(unix)]
    #[tokio::test]
    async fn connects_to_daemon_socket_with_websocket_upgrade() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::net::UnixListener;
        use tokio_tungstenite::{accept_async, tungstenite::Message as WsMessage};

        let home = tempfile::tempdir().unwrap();
        let control_dir = home.path().join("app-server-control");
        std::fs::create_dir_all(&control_dir).unwrap();
        let socket_path = control_dir.join("app-server-control.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut websocket = accept_async(stream).await.unwrap();
            let request = websocket.next().await.unwrap().unwrap();
            let WsMessage::Text(text) = request else {
                panic!("expected JSON-RPC text frame");
            };
            let value: serde_json::Value = serde_json::from_str(text.as_ref()).unwrap();
            assert_eq!(value["id"], 1);
            assert_eq!(value["method"], "initialize");
            websocket
                .send(WsMessage::Text(
                    serde_json::json!({"id":1,"result":{"ok":true}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        });

        let transport = StdioTransport::connect(
            std::path::Path::new("/unused/codex"),
            home.path(),
            &home.path().join("sqlite"),
            None,
        )
        .await
        .unwrap();
        let reader = transport.take_reader().await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);
        let reader_task = spawn_reader(reader, move |message| {
            if let Some(tx) = tx.take() {
                let _ = tx.send(message);
            }
        });
        transport
            .write_message(serde_json::json!({
                "jsonrpc":"2.0",
                "id":1,
                "method":"initialize",
                "params":{}
            }))
            .await
            .unwrap();

        match tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .unwrap()
            .unwrap()
        {
            Message::Response { id, outcome } => {
                assert_eq!(id, serde_json::json!(1));
                assert_eq!(outcome.unwrap()["ok"], true);
            }
            other => panic!("unexpected message: {other:?}"),
        }
        reader_task.abort();
        server.await.unwrap();
    }
}
