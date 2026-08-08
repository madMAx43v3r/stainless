//! HTTP/JSON and WebSocket transport for Stainless applications.
//!
//! The network runtime converts transport activity into compact JSON events.
//! A Stainless application owns routing, authentication, protocol state, and
//! response bodies by consuming those events through [`Server::next_event`].

#![forbid(unsafe_code)]
#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]

extern crate self as stainless_http;

mod client;
mod server;

pub use client::{Client, ClientError};
pub use server::{Server, ServerError};

include!(concat!(env!("OUT_DIR"), "/http.stainless.rs"));
