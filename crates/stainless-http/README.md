# stainless-http

Safe HTTP/JSON and WebSocket transport for Stainless applications.

The Rust side runs Axum/Tokio on a background runtime and exposes a blocking,
JSON-event bridge. Stainless code receives HTTP requests, WebSocket opens,
text messages, disconnects, and transport errors through `http::receive`, then
answers with `respond_json`, `send_json`, or `broadcast_json`. HTTP and
WebSocket traffic share one listener; request timeouts and body/message limits
are enforced by the transport.

`http::listen` accepts an `http::ServerConfig`. Its built-in `optional<String>`
`websocket_path` has no value for HTTP-only servers and contains the upgrade
path when HTTP and WebSocket traffic should share the listener.

The crate also exposes a reusable synchronous JSON client with optional
`x-api-token` authentication. It is used by the poker dealer for MMX WAPI
calls through generated Rust bindings.

Run its Stainless integration test from the workspace root with:

```sh
stainlessc --run --package crates/stainless-http/test
```
