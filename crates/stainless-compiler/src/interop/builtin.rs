use super::model::{
    ArgumentAdaptation, CallStyle, CallableBinding, NativeBindings, NativeTypeBinding, Parameter,
    Receiver, RustLowering, TraitRequirement, TypeRef,
};

const T: &str = "T";

/// Returns the compiler-provided native Rust bindings implemented so far.
///
/// These bindings expose the real Rust `Vec<T>` and `String` representations.
/// They are semantic/code-generation metadata, not runtime wrapper newtypes.
///
/// # Errors
///
/// Returns an error if compiler-provided metadata violates registry
/// invariants. Such an error indicates a compiler implementation defect.
pub fn standard_bindings() -> Result<NativeBindings, super::BindingError> {
    NativeBindings::new(vec![string_binding(), vec_binding()])
}

fn vec_binding() -> NativeTypeBinding {
    let t = TypeRef::Parameter(T);
    let vec_t = vec_of(t.clone());
    let mut callables = vec_construction(&vec_t);
    callables.extend(vec_capacity_methods());
    callables.extend(vec_element_methods(&t, &vec_t));
    callables.extend(vec_trait_methods(&t, &vec_t));

    NativeTypeBinding {
        stainless_path: "rust::Vec",
        rust_path: "::std::vec::Vec",
        type_parameters: vec![T],
        callables,
    }
}

fn vec_construction(vec_t: &TypeRef) -> Vec<CallableBinding> {
    vec![
        constructor(
            "Vec",
            vec![],
            vec_t.clone(),
            RustLowering::AssociatedFunction {
                rust_path: "::std::vec::Vec::new",
            },
        ),
        associated(
            "with_capacity",
            vec![Parameter::new("capacity", TypeRef::Usize)],
            vec_t.clone(),
            "::std::vec::Vec::with_capacity",
        ),
    ]
}

fn vec_capacity_methods() -> Vec<CallableBinding> {
    vec![
        method("len", Receiver::Shared, vec![], TypeRef::Usize),
        method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
        method("capacity", Receiver::Shared, vec![], TypeRef::Usize),
        method(
            "reserve",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "reserve_exact",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "shrink_to",
            Receiver::Mutable,
            vec![Parameter::new("minimum_capacity", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method("shrink_to_fit", Receiver::Mutable, vec![], TypeRef::Void),
    ]
}

fn vec_element_methods(t: &TypeRef, vec_t: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method(
            "push",
            Receiver::Mutable,
            vec![Parameter::new("value", t.clone())],
            TypeRef::Void,
        ),
        method("pop", Receiver::Mutable, vec![], option_of(t.clone())),
        method("clear", Receiver::Mutable, vec![], TypeRef::Void),
        method(
            "truncate",
            Receiver::Mutable,
            vec![Parameter::new("length", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "insert",
            Receiver::Mutable,
            vec![
                Parameter::new("index", TypeRef::Usize),
                Parameter::new("value", t.clone()),
            ],
            TypeRef::Void,
        ),
        method(
            "remove",
            Receiver::Mutable,
            vec![Parameter::new("index", TypeRef::Usize)],
            t.clone(),
        ),
        method(
            "swap_remove",
            Receiver::Mutable,
            vec![Parameter::new("index", TypeRef::Usize)],
            t.clone(),
        ),
        method(
            "append",
            Receiver::Mutable,
            vec![Parameter::new("other", TypeRef::mutable_ref(vec_t.clone()))],
            TypeRef::Void,
        ),
        method("reverse", Receiver::Mutable, vec![], TypeRef::Void),
    ]
}

fn vec_trait_methods(t: &TypeRef, vec_t: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method_with_requirements(
            "clone",
            Receiver::Shared,
            vec![],
            vec_t.clone(),
            vec![requirement(T, "::core::clone::Clone")],
        ),
        method_with_requirements(
            "contains",
            Receiver::Shared,
            vec![Parameter::new("value", TypeRef::shared_ref(t.clone()))],
            TypeRef::Bool,
            vec![requirement(T, "::core::cmp::PartialEq")],
        ),
        method_with_requirements(
            "sort",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
            vec![requirement(T, "::core::cmp::Ord")],
        ),
        method_with_requirements(
            "dedup",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
            vec![requirement(T, "::core::cmp::PartialEq")],
        ),
    ]
}

fn string_binding() -> NativeTypeBinding {
    let string = string_type();
    let string_ref = TypeRef::shared_ref(string.clone());
    let mut callables = string_construction(&string, &string_ref);
    callables.extend(string_capacity_methods());
    callables.extend(string_mutation_methods(&string_ref));
    callables.extend(string_query_methods(&string, &string_ref));

    NativeTypeBinding {
        stainless_path: "rust::String",
        rust_path: "::std::string::String",
        type_parameters: vec![],
        callables,
    }
}

fn string_construction(string: &TypeRef, string_ref: &TypeRef) -> Vec<CallableBinding> {
    vec![
        constructor(
            "String",
            vec![],
            string.clone(),
            RustLowering::AssociatedFunction {
                rust_path: "::std::string::String::new",
            },
        ),
        constructor(
            "String",
            vec![Parameter::new("value", string_ref.clone())],
            string.clone(),
            RustLowering::CloneArgument { index: 0 },
        ),
        associated(
            "with_capacity",
            vec![Parameter::new("capacity", TypeRef::Usize)],
            string.clone(),
            "::std::string::String::with_capacity",
        ),
        method("clone", Receiver::Shared, vec![], string.clone()),
        method("into_bytes", Receiver::Value, vec![], vec_of(TypeRef::U8)),
    ]
}

fn string_capacity_methods() -> Vec<CallableBinding> {
    vec![
        method("len", Receiver::Shared, vec![], TypeRef::Usize),
        method("is_empty", Receiver::Shared, vec![], TypeRef::Bool),
        method("capacity", Receiver::Shared, vec![], TypeRef::Usize),
        method(
            "reserve",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "reserve_exact",
            Receiver::Mutable,
            vec![Parameter::new("additional", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method(
            "shrink_to",
            Receiver::Mutable,
            vec![Parameter::new("minimum_capacity", TypeRef::Usize)],
            TypeRef::Void,
        ),
        method("shrink_to_fit", Receiver::Mutable, vec![], TypeRef::Void),
        method(
            "truncate",
            Receiver::Mutable,
            vec![Parameter::new("length", TypeRef::Usize)],
            TypeRef::Void,
        ),
    ]
}

fn string_mutation_methods(string_ref: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method("clear", Receiver::Mutable, vec![], TypeRef::Void),
        method(
            "push",
            Receiver::Mutable,
            vec![Parameter::new("value", TypeRef::Char)],
            TypeRef::Void,
        ),
        string_ref_method(
            "push_str",
            Receiver::Mutable,
            vec![("value", string_ref.clone())],
            TypeRef::Void,
        ),
        method("pop", Receiver::Mutable, vec![], option_of(TypeRef::Char)),
        method(
            "insert",
            Receiver::Mutable,
            vec![
                Parameter::new("index", TypeRef::Usize),
                Parameter::new("value", TypeRef::Char),
            ],
            TypeRef::Void,
        ),
        string_ref_method(
            "insert_str",
            Receiver::Mutable,
            vec![("index", TypeRef::Usize), ("value", string_ref.clone())],
            TypeRef::Void,
        ),
        method(
            "remove",
            Receiver::Mutable,
            vec![Parameter::new("index", TypeRef::Usize)],
            TypeRef::Char,
        ),
        method(
            "make_ascii_lowercase",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
        ),
        method(
            "make_ascii_uppercase",
            Receiver::Mutable,
            vec![],
            TypeRef::Void,
        ),
    ]
}

fn string_query_methods(string: &TypeRef, string_ref: &TypeRef) -> Vec<CallableBinding> {
    vec![
        method("is_ascii", Receiver::Shared, vec![], TypeRef::Bool),
        string_ref_method(
            "contains",
            Receiver::Shared,
            vec![("pattern", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "starts_with",
            Receiver::Shared,
            vec![("pattern", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "ends_with",
            Receiver::Shared,
            vec![("pattern", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "eq_ignore_ascii_case",
            Receiver::Shared,
            vec![("other", string_ref.clone())],
            TypeRef::Bool,
        ),
        string_ref_method(
            "replace",
            Receiver::Shared,
            vec![("from", string_ref.clone()), ("to", string_ref.clone())],
            string.clone(),
        ),
        method(
            "repeat",
            Receiver::Shared,
            vec![Parameter::new("count", TypeRef::Usize)],
            string.clone(),
        ),
        method("to_lowercase", Receiver::Shared, vec![], string.clone()),
        method("to_uppercase", Receiver::Shared, vec![], string.clone()),
    ]
}

fn constructor(
    source_name: &'static str,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    lowering: RustLowering,
) -> CallableBinding {
    CallableBinding {
        source_name,
        style: CallStyle::Constructor,
        receiver: None,
        parameters,
        return_type,
        return_borrow: None,
        requirements: vec![],
        lowering,
    }
}

fn associated(
    source_name: &'static str,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    rust_path: &'static str,
) -> CallableBinding {
    CallableBinding {
        source_name,
        style: CallStyle::AssociatedFunction,
        receiver: None,
        parameters,
        return_type,
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::AssociatedFunction { rust_path },
    }
}

fn method(
    source_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
) -> CallableBinding {
    method_with_requirements(source_name, receiver, parameters, return_type, vec![])
}

fn method_with_requirements(
    source_name: &'static str,
    receiver: Receiver,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    requirements: Vec<TraitRequirement>,
) -> CallableBinding {
    CallableBinding {
        source_name,
        style: CallStyle::Method,
        receiver: Some(receiver),
        parameters,
        return_type,
        return_borrow: None,
        requirements,
        lowering: RustLowering::Method {
            rust_name: source_name,
        },
    }
}

fn string_ref_method(
    source_name: &'static str,
    receiver: Receiver,
    parameters: Vec<(&'static str, TypeRef)>,
    return_type: TypeRef,
) -> CallableBinding {
    method(
        source_name,
        receiver,
        parameters
            .into_iter()
            .map(|(name, ty)| {
                let adaptation = if ty == TypeRef::shared_ref(string_type()) {
                    ArgumentAdaptation::StringRefToStr
                } else {
                    ArgumentAdaptation::Identity
                };
                Parameter::adapted(name, ty, adaptation)
            })
            .collect(),
        return_type,
    )
}

fn requirement(parameter: &'static str, rust_trait: &'static str) -> TraitRequirement {
    TraitRequirement {
        parameter,
        rust_trait,
    }
}

fn string_type() -> TypeRef {
    TypeRef::native("rust::String", vec![])
}

fn vec_of(element: TypeRef) -> TypeRef {
    TypeRef::native("rust::Vec", vec![element])
}

fn option_of(value: TypeRef) -> TypeRef {
    TypeRef::native("rust::Option", vec![value])
}
