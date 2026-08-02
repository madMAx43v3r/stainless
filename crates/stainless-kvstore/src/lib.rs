//! A versioned, revertible key/value store implemented in Stainless.
//!
//! The generated implementation is intentionally included at crate root: it
//! is both an executable showcase and an integration test for using Stainless
//! as the implementation language of a Rust crate.

// Generated Rust favors a direct, auditable correspondence with Stainless
// control flow over hand-written Rust style. Lint the compiler/runtime and the
// small facade below, but do not treat stylistic Clippy rewrites as source
// diagnostics for generated code.
#![allow(clippy::all, clippy::pedantic, clippy::nursery)]

include!(concat!(env!("OUT_DIR"), "/kvstore.stainless.rs"));

mod typed;

pub use typed::{Codec, Error, OrderedKey, Table};

/// Runs the end-to-end Stainless store showcase at `path`.
///
/// # Errors
///
/// Returns the checked Stainless exception message if file I/O, a reader
/// thread, or a store invariant fails.
pub fn self_test(path: &str) -> Result<(), String> {
    let path = path.to_owned();
    stainless_kvstore_self_test(&path)
        .map_err(|error| error.to_string())
        .and_then(|status| {
            if status == 0 {
                Ok(())
            } else {
                Err(format!("Stainless kvstore self-test returned {status}"))
            }
        })
}
