# stainless-http

Safe HTTP/JSON, binary HTTP, and WebSocket transport for Stainless applications.

The Rust side runs Axum/Tokio on a background runtime and exposes a blocking,
JSON-event bridge. Stainless code receives HTTP requests, WebSocket opens,
text or binary messages, disconnects, and transport errors through
`http::receive`. HTTP bodies are available as `Vec<u8>` through
`request_bytes`; binary WebSocket events carry an opaque string token for
`take_message_bytes`. Applications answer with `respond_json` or
`respond_bytes`, and send WebSocket frames with the matching `send_*` and
`broadcast_*` functions. HTTP and WebSocket traffic share one listener;
request timeouts and body/message limits are enforced by the transport.

`http::listen` accepts an `http::ServerConfig`. Its built-in `optional<String>`
`websocket_path` has no value for HTTP-only servers and contains the upgrade
path when HTTP and WebSocket traffic should share the listener.

The crate also exposes a reusable synchronous client configured through
`http::ClientConfig`. Its `get_json` and `post_json` functions parse JSON,
while `get_bytes` and `post_bytes` preserve `Vec<u8>` bodies and use
`application/octet-stream` for binary POST requests. Applications can
populate the config's `Map<String, String>` headers without placing
authentication policy in the transport crate. The poker dealer uses this
generic interface for MMX WAPI calls.

The client owns a connection-pooling agent. A later request opens a new
connection when no reusable connection is available, but the HTTP facade does
not retry a failed request or add application-level backoff.

Run its Stainless integration test from the workspace root with:

```sh
stainlessc --run --package crates/stainless-http/test
```
