//! Native Rust API metadata used by Stainless name and type resolution.

mod builtin;
mod model;

pub use builtin::standard_bindings;
pub use model::{
    ArgumentAdaptation, BindingError, CallStyle, CallableBinding, NativeBindings,
    NativeErrorFormat, NativeTypeBinding, Parameter, Receiver, ReturnBorrow, ReturnBorrowError,
    RustLowering, TraitRequirement, TypeRef, WrapperTarget,
};
