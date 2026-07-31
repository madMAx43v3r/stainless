//! Native Rust API metadata used by Stainless name and type resolution.

mod builtin;
mod manifest;
mod model;

pub use builtin::standard_bindings;
pub use manifest::{
    BINDINGS_MANIFEST_FILENAME, ManifestError, load_bindings_manifest, load_package_bindings,
    parse_bindings_manifest,
};
pub use model::{
    ArgumentAdaptation, BindingError, CallStyle, CallableBinding, CallbackEscape, CallbackKind,
    CallbackType, FunctionType, NativeBindings, NativeErrorFormat, NativeTypeBinding, Parameter,
    Receiver, ReturnBorrow, ReturnBorrowError, RustLowering, StoredFunctionKind, TraitRequirement,
    TypeRef, WrapperTarget,
};
