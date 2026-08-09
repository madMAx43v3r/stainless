use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc as tokio_mpsc, oneshot, watch};

const JSON_CONTENT_TYPE: &str = "application/json";
const EVENT_QUEUE_CAPACITY: usize = 1024;
const CONNECTION_QUEUE_CAPACITY: usize = 256;

/// A blocking event bridge between Stainless and an asynchronous HTTP server.
///
/// Incoming HTTP requests and WebSocket activity are serialized as JSON by
/// [`Self::next_event`]. Binary payloads remain available through
/// [`Self::request_bytes`] and [`Self::take_message_bytes`]. The application
/// completes an HTTP request with [`Self::respond`] or [`Self::respond_bytes`]
/// and controls WebSocket peers with [`Self::send_text`], [`Self::send_bytes`],
/// and [`Self::close`].
pub struct Server {
    state: Arc<SharedState>,
    events: Mutex<mpsc::Receiver<String>>,
    local_addr: String,
    runtime_thread: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
struct AppState {
    shared: Arc<SharedState>,
}

struct SharedState {
    events: mpsc::SyncSender<String>,
    next_request_id: AtomicU64,
    next_connection_id: AtomicU64,
    next_binary_message_id: AtomicU64,
    pending_http: Mutex<HashMap<u64, PendingRequest>>,
    pending_binary_messages: Mutex<HashMap<String, Vec<u8>>>,
    connections: Mutex<HashMap<u64, tokio_mpsc::Sender<WsCommand>>>,
    shutdown: watch::Sender<bool>,
    request_timeout: Duration,
    max_body_bytes: usize,
}

struct PendingRequest {
    response: oneshot::Sender<ResponseSpec>,
    body: Vec<u8>,
}

struct ResponseSpec {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

enum WsCommand {
    Text(String),
    Binary(Vec<u8>),
    Close { code: u16, reason: String },
}

/// Failure to start or communicate with the native transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerError {
    message: String,
}

impl ServerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ServerError {}

impl Server {
    /// Binds one listener and starts its asynchronous runtime on a background
    /// thread. `address` may contain port zero to select a free local port. An
    /// empty `websocket_path` disables WebSocket upgrades.
    ///
    /// # Errors
    ///
    /// Returns an error when the address cannot be bound, the runtime cannot
    /// be created, or `websocket_path` is not an absolute HTTP path.
    pub fn bind(
        address: &str,
        websocket_path: &str,
        request_timeout_ms: u64,
        max_body_bytes: usize,
    ) -> Result<Self, ServerError> {
        if !websocket_path.is_empty()
            && (!websocket_path.starts_with('/') || websocket_path.contains(['?', '#']))
        {
            return Err(ServerError::new(
                "the WebSocket path must be an absolute path without a query or fragment",
            ));
        }
        let websocket_path = (!websocket_path.is_empty()).then(|| websocket_path.to_owned());
        if max_body_bytes == 0 {
            return Err(ServerError::new(
                "the maximum HTTP request body size must be greater than zero",
            ));
        }

        let listener = TcpListener::bind(address)
            .map_err(|error| ServerError::new(format!("failed to bind `{address}`: {error}")))?;
        listener.set_nonblocking(true).map_err(|error| {
            ServerError::new(format!("failed to configure the HTTP listener: {error}"))
        })?;
        let local_addr = listener
            .local_addr()
            .map_err(|error| ServerError::new(format!("failed to inspect listener: {error}")))?
            .to_string();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("stainless-http")
            .build()
            .map_err(|error| ServerError::new(format!("failed to create HTTP runtime: {error}")))?;

        let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let (shutdown, _) = watch::channel(false);
        let state = Arc::new(SharedState {
            events: event_sender,
            next_request_id: AtomicU64::new(1),
            next_connection_id: AtomicU64::new(1),
            next_binary_message_id: AtomicU64::new(1),
            pending_http: Mutex::new(HashMap::new()),
            pending_binary_messages: Mutex::new(HashMap::new()),
            connections: Mutex::new(HashMap::new()),
            shutdown,
            request_timeout: Duration::from_millis(request_timeout_ms.max(1)),
            max_body_bytes,
        });
        let runtime_state = Arc::clone(&state);
        let runtime_thread = std::thread::Builder::new()
            .name("stainless-http-runtime".to_owned())
            .spawn(move || {
                runtime.block_on(run_server(listener, websocket_path, runtime_state));
            })
            .map_err(|error| {
                ServerError::new(format!("failed to start HTTP runtime thread: {error}"))
            })?;

        Ok(Self {
            state,
            events: Mutex::new(event_receiver),
            local_addr,
            runtime_thread: Mutex::new(Some(runtime_thread)),
        })
    }

    /// Returns the listener address, including an operating-system-selected
    /// port when port zero was requested.
    #[must_use]
    pub fn local_addr(&self) -> String {
        self.local_addr.clone()
    }

    /// Waits for the next application event, returning an empty string when
    /// the timeout expires.
    ///
    /// # Errors
    ///
    /// Returns an error if another thread poisoned the event receiver lock or
    /// if the runtime event stream stopped unexpectedly.
    pub fn next_event(&self, timeout_ms: u64) -> Result<String, ServerError> {
        let events = self
            .events
            .lock()
            .map_err(|_| ServerError::new("the HTTP event receiver lock is poisoned"))?;
        match events.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(String::new()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(ServerError::new("the HTTP runtime event stream stopped"))
            }
        }
    }

    /// Completes one pending HTTP request. Returns `false` if it already
    /// timed out, was answered, or does not exist.
    #[must_use]
    pub fn respond(&self, request_id: u64, status: u16, content_type: &str, body: &str) -> bool {
        self.respond_bytes(request_id, status, content_type, body.as_bytes())
    }

    /// Returns the exact body of one pending HTTP request.
    ///
    /// The bytes remain available until the request is answered or times out.
    ///
    /// # Errors
    ///
    /// Returns an error when the request is unknown, answered, or timed out.
    pub fn request_bytes(&self, request_id: u64) -> Result<Vec<u8>, ServerError> {
        lock(&self.state.pending_http)
            .get(&request_id)
            .map(|request| request.body.clone())
            .ok_or_else(|| ServerError::new(format!("HTTP request {request_id} is not pending")))
    }

    /// Completes one pending HTTP request with an arbitrary binary body.
    /// Returns `false` if it already timed out, was answered, or does not
    /// exist.
    #[must_use]
    pub fn respond_bytes(
        &self,
        request_id: u64,
        status: u16,
        content_type: &str,
        body: &[u8],
    ) -> bool {
        let Some(request) = lock(&self.state.pending_http).remove(&request_id) else {
            return false;
        };
        request
            .response
            .send(ResponseSpec {
                status,
                content_type: content_type.to_owned(),
                body: body.to_owned(),
            })
            .is_ok()
    }

    /// Queues a UTF-8 WebSocket text message for one peer.
    #[must_use]
    pub fn send_text(&self, connection_id: u64, text: &str) -> bool {
        lock(&self.state.connections)
            .get(&connection_id)
            .is_some_and(|connection| {
                connection
                    .try_send(WsCommand::Text(text.to_owned()))
                    .is_ok()
            })
    }

    /// Queues a binary WebSocket message for one peer.
    #[must_use]
    pub fn send_bytes(&self, connection_id: u64, body: &[u8]) -> bool {
        lock(&self.state.connections)
            .get(&connection_id)
            .is_some_and(|connection| {
                connection
                    .try_send(WsCommand::Binary(body.to_owned()))
                    .is_ok()
            })
    }

    /// Queues one UTF-8 text message for every connected WebSocket peer and
    /// returns the number of queues that accepted it.
    #[must_use]
    pub fn broadcast_text(&self, text: &str) -> usize {
        lock(&self.state.connections)
            .values()
            .filter(|connection| {
                connection
                    .try_send(WsCommand::Text(text.to_owned()))
                    .is_ok()
            })
            .count()
    }

    /// Queues one binary message for every connected WebSocket peer and
    /// returns the number of queues that accepted it.
    #[must_use]
    pub fn broadcast_bytes(&self, body: &[u8]) -> usize {
        lock(&self.state.connections)
            .values()
            .filter(|connection| {
                connection
                    .try_send(WsCommand::Binary(body.to_owned()))
                    .is_ok()
            })
            .count()
    }

    /// Takes the payload associated with one `ws_binary` event.
    ///
    /// # Errors
    ///
    /// Returns an error when the message token is unknown or was already
    /// consumed.
    pub fn take_message_bytes(&self, message_id: &str) -> Result<Vec<u8>, ServerError> {
        lock(&self.state.pending_binary_messages)
            .remove(message_id)
            .ok_or_else(|| {
                ServerError::new(format!(
                    "WebSocket binary message `{message_id}` is not pending"
                ))
            })
    }

    /// Starts a WebSocket close handshake with one peer.
    #[must_use]
    pub fn close(&self, connection_id: u64, code: u16, reason: &str) -> bool {
        lock(&self.state.connections)
            .get(&connection_id)
            .is_some_and(|connection| {
                connection
                    .try_send(WsCommand::Close {
                        code,
                        reason: reason.to_owned(),
                    })
                    .is_ok()
            })
    }

    /// Requests graceful shutdown. Calling this more than once is harmless.
    pub fn shutdown(&self) {
        let _ = self.state.shutdown.send_replace(true);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(thread) = lock(&self.runtime_thread).take() {
            let _ = thread.join();
        }
    }
}

async fn run_server(
    listener: TcpListener,
    websocket_path: Option<String>,
    state: Arc<SharedState>,
) {
    let listener = match tokio::net::TcpListener::from_std(listener) {
        Ok(listener) => listener,
        Err(error) => {
            emit(
                &state,
                json!({"type": "server_error", "message": error.to_string()}),
            );
            return;
        }
    };
    let app_state = AppState {
        shared: Arc::clone(&state),
    };
    let app = match websocket_path {
        Some(path) => Router::new().route(&path, get(upgrade_websocket)),
        None => Router::new(),
    }
    .fallback(handle_http)
    .with_state(app_state);
    let mut shutdown = state.shutdown.subscribe();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            while !*shutdown.borrow() && shutdown.changed().await.is_ok() {}
        })
        .await;
    if let Err(error) = result {
        emit(
            &state,
            json!({"type": "server_error", "message": error.to_string()}),
        );
    }
}

async fn handle_http(State(state): State<AppState>, request: Request<Body>) -> Response {
    if request.method() == axum::http::Method::OPTIONS {
        return cors_response(StatusCode::NO_CONTENT, "text/plain", Vec::new());
    }
    let request_id = state.shared.next_request_id.fetch_add(1, Ordering::Relaxed);
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, state.shared.max_body_bytes).await {
        Ok(body) => body,
        Err(error) => {
            return json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_body_rejected",
                &error.to_string(),
            );
        }
    };
    let body = body.to_vec();
    let body_text = std::str::from_utf8(&body).ok();
    let (response_sender, response_receiver) = oneshot::channel();
    let event = json!({
        "type": "http",
        "request_id": request_id,
        "method": parts.method.as_str(),
        "path": parts.uri.path(),
        "query": parts.uri.query().unwrap_or_default(),
        "headers": headers_json(&parts.headers),
        "body": body_text,
        "body_is_utf8": body_text.is_some(),
    });
    lock(&state.shared.pending_http).insert(
        request_id,
        PendingRequest {
            response: response_sender,
            body,
        },
    );
    if let Err(error) = state.shared.events.try_send(event.to_string()) {
        lock(&state.shared.pending_http).remove(&request_id);
        return match error {
            mpsc::TrySendError::Full(_) => json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "event_queue_full",
                "the application event queue is full",
            ),
            mpsc::TrySendError::Disconnected(_) => json_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "event_stream_stopped",
                "the application event stream is unavailable",
            ),
        };
    }
    match tokio::time::timeout(state.shared.request_timeout, response_receiver).await {
        Ok(Ok(response)) => application_response(response),
        Ok(Err(_)) => json_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "response_channel_closed",
            "the application stopped before answering",
        ),
        Err(_) => {
            lock(&state.shared.pending_http).remove(&request_id);
            json_error(
                StatusCode::GATEWAY_TIMEOUT,
                "response_timeout",
                "the application did not answer before its request timeout",
            )
        }
    }
}

async fn upgrade_websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    let path = uri.path().to_owned();
    let query = uri.query().unwrap_or_default().to_owned();
    let headers = headers_json(&headers);
    upgrade
        .max_message_size(state.shared.max_body_bytes)
        .max_frame_size(state.shared.max_body_bytes)
        .on_upgrade(move |socket| serve_websocket(socket, state.shared, path, query, headers))
}

async fn serve_websocket(
    socket: WebSocket,
    state: Arc<SharedState>,
    path: String,
    query: String,
    headers: Value,
) {
    let connection_id = state.next_connection_id.fetch_add(1, Ordering::Relaxed);
    let (command_sender, mut command_receiver) = tokio_mpsc::channel(CONNECTION_QUEUE_CAPACITY);
    lock(&state.connections).insert(connection_id, command_sender);
    if !emit(
        &state,
        json!({
            "type": "ws_open",
            "connection_id": connection_id,
            "path": path,
            "query": query,
            "headers": headers,
        }),
    ) {
        lock(&state.connections).remove(&connection_id);
        return;
    }

    let (mut output, mut input) = socket.split();
    let mut shutdown = state.shutdown.subscribe();
    let mut close_code_value = close_code::NORMAL;
    let mut close_reason = String::new();
    loop {
        tokio::select! {
            incoming = input.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if !emit(
                            &state,
                            json!({
                                "type": "ws_message",
                                "connection_id": connection_id,
                                "text": text.as_str(),
                            }),
                        ) {
                            close_code_value = close_code::ERROR;
                            close_reason = "application event queue is unavailable".to_owned();
                            let _ = output
                                .send(close_message(close_code_value, &close_reason))
                                .await;
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if output.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(body))) => {
                        let message_id = state
                            .next_binary_message_id
                            .fetch_add(1, Ordering::Relaxed)
                            .to_string();
                        let can_queue = {
                            let mut messages = lock(&state.pending_binary_messages);
                            if messages.len() >= EVENT_QUEUE_CAPACITY {
                                false
                            } else {
                                messages.insert(
                                    message_id.clone(),
                                    body.to_vec(),
                                );
                                true
                            }
                        };
                        if !can_queue {
                            close_code_value = close_code::ERROR;
                            close_reason = "pending binary message queue is full".to_owned();
                            let _ = output.send(close_message(close_code_value, &close_reason)).await;
                            break;
                        }
                        if !emit(
                            &state,
                            json!({
                                "type": "ws_binary",
                                "connection_id": connection_id,
                                "message_id": message_id,
                            }),
                        ) {
                            lock(&state.pending_binary_messages).remove(&message_id);
                            close_code_value = close_code::ERROR;
                            close_reason = "application event queue is unavailable".to_owned();
                            let _ = output
                                .send(close_message(close_code_value, &close_reason))
                                .await;
                            break;
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        if let Some(frame) = frame {
                            close_code_value = frame.code;
                            close_reason = frame.reason.to_string();
                        }
                        break;
                    }
                    Some(Err(error)) => {
                        close_code_value = close_code::ERROR;
                        close_reason = error.to_string();
                        break;
                    }
                    None => break,
                }
            }
            command = command_receiver.recv() => {
                match command {
                    Some(WsCommand::Text(text)) => {
                        if output.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(WsCommand::Binary(body)) => {
                        if output.send(Message::Binary(body.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(WsCommand::Close { code, reason }) => {
                        close_code_value = code;
                        close_reason = reason;
                        let _ = output.send(close_message(code, &close_reason)).await;
                        break;
                    }
                    None => break,
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    close_code_value = close_code::AWAY;
                    close_reason = "server shutdown".to_owned();
                    let _ = output.send(close_message(close_code_value, &close_reason)).await;
                    break;
                }
            }
        }
    }
    lock(&state.connections).remove(&connection_id);
    emit(
        &state,
        json!({
            "type": "ws_close",
            "connection_id": connection_id,
            "code": close_code_value,
            "reason": close_reason,
        }),
    );
}

fn close_message(code: u16, reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code,
        reason: reason.to_owned().into(),
    }))
}

fn application_response(response: ResponseSpec) -> Response {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    cors_response(status, &response.content_type, response.body)
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    cors_response(
        status,
        JSON_CONTENT_TYPE,
        json!({"error": {"code": code, "message": message}})
            .to_string()
            .into_bytes(),
    )
}

fn cors_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    if let Ok(value) = content_type.parse() {
        headers.insert(axum::http::header::CONTENT_TYPE, value);
    }
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        axum::http::HeaderValue::from_static("*"),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_HEADERS,
        axum::http::HeaderValue::from_static("content-type, authorization"),
    );
    headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_METHODS,
        axum::http::HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    response
}

fn headers_json(headers: &HeaderMap) -> Value {
    let mut result = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(value) = value.to_str() {
            result.insert(name.as_str().to_owned(), Value::String(value.to_owned()));
        }
    }
    Value::Object(result.into_iter().collect())
}

fn emit(state: &SharedState, event: Value) -> bool {
    state.events.try_send(event.to_string()).is_ok()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
