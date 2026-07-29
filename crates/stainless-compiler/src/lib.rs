//! Compiler infrastructure for the Stainless language.
//!
//! The first implemented component is the native Rust binding registry. It
//! describes the Rust APIs that Stainless can resolve and lower without
//! introducing wrapper newtypes.

pub mod interop;
