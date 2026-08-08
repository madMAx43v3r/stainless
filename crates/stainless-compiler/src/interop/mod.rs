//! Native Rust API metadata used by Stainless name and type resolution.

mod builtin;
mod manifest;
mod model;

pub(crate) use builtin::VAR_TYPE_PATH;
pub use builtin::standard_bindings;
pub use manifest::{
    BINDINGS_MANIFEST_FILENAME, CargoDependency, ManifestError, load_bindings_manifest,
    load_package_bindings, load_package_dependencies, load_package_external_bindings,
    parse_bindings_manifest,
};
pub use model::{
    ArgumentAdaptation, BindingError, CallStyle, CallableBinding, CallbackEscape, CallbackKind,
    CallbackType, FunctionType, NativeBindings, NativeErrorFormat, NativeTypeBinding, Parameter,
    PointerKind, Receiver, ReturnAdaptation, ReturnBorrow, ReturnBorrowError, RustLowering,
    StoredFunctionKind, TraitRequirement, TypeRef, WrapperTarget,
};
