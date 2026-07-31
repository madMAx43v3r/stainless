use std::collections::BTreeSet;

use stainless_compiler::interop::{
    ArgumentAdaptation, BindingError, CallStyle, CallableBinding, NativeBindings,
    NativeErrorFormat, NativeTypeBinding, Receiver, RustLowering, TypeRef, WrapperTarget,
    parse_bindings_manifest, standard_bindings,
};

#[test]
fn standard_registry_contains_builtin_types_in_path_order() {
    let bindings = standard_bindings().unwrap();
    let paths = bindings
        .types()
        .map(|binding| binding.stainless_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        [
            "rust::String",
            "rust::Vec",
            "rust::stainless_runtime::JsonError",
            "rust::stainless_runtime::Var",
        ]
    );
}

#[test]
fn regex_uses_generated_wrappers_and_proven_error_formatting() {
    let external = parse_bindings_manifest(include_str!(
        "../../../docs/ref/17_external_regex_wrapper.bindings.toml"
    ))
    .unwrap();
    let bindings = standard_bindings().unwrap().merge(external).unwrap();
    let regex = bindings.type_by_path("rust::regex::Regex").unwrap();
    let error = bindings.type_by_path("rust::regex::Error").unwrap();
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));

    assert_eq!(error.error_format, Some(NativeErrorFormat::Display));
    let new = regex
        .find_callable(
            CallStyle::AssociatedFunction,
            "new",
            std::slice::from_ref(&string_ref),
        )
        .unwrap();
    let RustLowering::GeneratedWrapper {
        wrapper_name,
        target,
    } = &new.lowering
    else {
        panic!("Regex::new should use a generated wrapper");
    };
    assert!(wrapper_name.starts_with("__stainless_wrapper_v1_"));
    assert_eq!(
        target,
        &WrapperTarget::Function {
            rust_path: "::regex::Regex::new".to_owned(),
        }
    );
    assert_eq!(
        new.parameters[0].adaptation,
        ArgumentAdaptation::StringRefToStr
    );

    let is_match = regex
        .find_callable(
            CallStyle::Method,
            "is_match",
            std::slice::from_ref(&string_ref),
        )
        .unwrap();
    assert_eq!(is_match.receiver, Some(Receiver::Shared));
    let RustLowering::GeneratedWrapper {
        wrapper_name,
        target,
    } = &is_match.lowering
    else {
        panic!("Regex::is_match should use a generated wrapper");
    };
    assert!(wrapper_name.starts_with("__stainless_wrapper_v1_"));
    assert_eq!(
        target,
        &WrapperTarget::Method {
            rust_name: "is_match".to_owned(),
        }
    );
}

#[test]
fn generated_wrapper_names_are_deterministic() {
    let source = include_str!("../../../docs/ref/17_external_regex_wrapper.bindings.toml");
    let first = parse_bindings_manifest(source).unwrap();
    let second = parse_bindings_manifest(source).unwrap();

    assert_eq!(
        first
            .types()
            .flat_map(|binding| &binding.callables)
            .map(|callable| &callable.lowering)
            .collect::<Vec<_>>(),
        second
            .types()
            .flat_map(|binding| &binding.callables)
            .map(|callable| &callable.lowering)
            .collect::<Vec<_>>()
    );
}

#[test]
fn invalid_generated_wrapper_metadata_is_rejected_before_lowering() {
    let wrapper = |source_name, style, receiver, target| CallableBinding {
        source_name,
        style,
        receiver,
        parameters: vec![],
        return_type: TypeRef::Bool,
        rust_result_error: None,
        return_borrow: None,
        requirements: vec![],
        lowering: RustLowering::GeneratedWrapper {
            wrapper_name: "__duplicate_wrapper".to_owned(),
            target,
        },
    };
    let binding = |callables| NativeTypeBinding {
        stainless_path: "rust::fixture::Type".to_owned(),
        rust_path: "::fixture::Type".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables,
    };

    let mismatched = wrapper(
        "bad".to_owned(),
        CallStyle::Method,
        Some(Receiver::Shared),
        WrapperTarget::Function {
            rust_path: "::fixture::Type::bad".to_owned(),
        },
    );
    assert!(matches!(
        NativeBindings::new(vec![binding(vec![mismatched])]),
        Err(BindingError::WrapperTargetMismatch { .. })
    ));

    let first = wrapper(
        "first".to_owned(),
        CallStyle::AssociatedFunction,
        None,
        WrapperTarget::Function {
            rust_path: "::fixture::Type::first".to_owned(),
        },
    );
    let second = wrapper(
        "second".to_owned(),
        CallStyle::AssociatedFunction,
        None,
        WrapperTarget::Function {
            rust_path: "::fixture::Type::second".to_owned(),
        },
    );
    assert_eq!(
        NativeBindings::new(vec![binding(vec![first, second])]),
        Err(BindingError::DuplicateWrapperName(
            "__duplicate_wrapper".to_owned()
        ))
    );
}

#[test]
fn vec_has_default_and_capacity_construction() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let vec_t = TypeRef::native("rust::Vec", vec![TypeRef::Parameter("T".to_owned())]);

    let constructor = vec_binding
        .find_callable(CallStyle::Constructor, "Vec", &[])
        .unwrap();
    assert_eq!(constructor.return_type, vec_t);
    assert_eq!(
        constructor.lowering,
        RustLowering::AssociatedFunction {
            rust_path: "::std::vec::Vec::new".to_owned()
        }
    );

    let with_capacity = vec_binding
        .find_callable(
            CallStyle::AssociatedFunction,
            "with_capacity",
            &[TypeRef::Usize],
        )
        .unwrap();
    assert_eq!(with_capacity.return_type, vec_t);
}

#[test]
fn vec_common_methods_have_expected_receiver_effects() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    let len = vec_binding
        .find_callable(CallStyle::Method, "len", &[])
        .unwrap();
    assert_eq!(len.receiver, Some(Receiver::Shared));
    assert_eq!(len.return_type, TypeRef::Usize);

    let push = vec_binding
        .find_callable(CallStyle::Method, "push", std::slice::from_ref(&t))
        .unwrap();
    assert_eq!(push.receiver, Some(Receiver::Mutable));
    assert_eq!(push.return_type, TypeRef::Void);

    let pop = vec_binding
        .find_callable(CallStyle::Method, "pop", &[])
        .unwrap();
    assert_eq!(pop.receiver, Some(Receiver::Mutable));
    assert_eq!(pop.return_type, TypeRef::native("rust::Option", vec![t]));
}

#[test]
fn vec_trait_requirements_are_preserved() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    let contains = vec_binding
        .find_callable(CallStyle::Method, "contains", &[TypeRef::shared_ref(t)])
        .unwrap();
    assert_eq!(contains.requirements.len(), 1);
    assert_eq!(contains.requirements[0].parameter, "T");
    assert_eq!(
        contains.requirements[0].rust_trait,
        "::core::cmp::PartialEq"
    );

    let sort = vec_binding
        .find_callable(CallStyle::Method, "sort", &[])
        .unwrap();
    assert_eq!(sort.requirements[0].rust_trait, "::core::cmp::Ord");
}

#[test]
fn string_copy_constructor_is_explicit_clone_lowering() {
    let bindings = standard_bindings().unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let string = TypeRef::native("rust::String", vec![]);

    let copy_constructor = string_binding
        .find_callable(
            CallStyle::Constructor,
            "String",
            &[TypeRef::shared_ref(string.clone())],
        )
        .unwrap();
    assert_eq!(copy_constructor.return_type, string);
    assert_eq!(
        copy_constructor.lowering,
        RustLowering::CloneArgument { index: 0 }
    );
}

#[test]
fn string_borrow_arguments_adapt_to_str_after_resolution() {
    let bindings = standard_bindings().unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));

    for method_name in [
        "push_str",
        "contains",
        "starts_with",
        "ends_with",
        "eq_ignore_ascii_case",
    ] {
        let callable = string_binding
            .find_callable(
                CallStyle::Method,
                method_name,
                std::slice::from_ref(&string_ref),
            )
            .unwrap();
        assert_eq!(
            callable.parameters[0].adaptation,
            ArgumentAdaptation::StringRefToStr
        );
    }

    let replace = string_binding
        .find_callable(
            CallStyle::Method,
            "replace",
            &[string_ref.clone(), string_ref],
        )
        .unwrap();
    assert!(
        replace
            .parameters
            .iter()
            .all(|parameter| parameter.adaptation == ArgumentAdaptation::StringRefToStr)
    );
}

#[test]
fn string_into_bytes_consumes_the_receiver() {
    let bindings = standard_bindings().unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let into_bytes = string_binding
        .find_callable(CallStyle::Method, "into_bytes", &[])
        .unwrap();

    assert_eq!(into_bytes.receiver, Some(Receiver::Value));
    assert_eq!(
        into_bytes.return_type,
        TypeRef::native("rust::Vec", vec![TypeRef::U8])
    );
}

#[test]
fn native_call_matching_uses_exact_stainless_types() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();
    let t = TypeRef::Parameter("T".to_owned());

    assert!(
        vec_binding
            .find_callable(CallStyle::Method, "push", std::slice::from_ref(&t),)
            .is_some()
    );
    assert!(
        vec_binding
            .find_callable(CallStyle::Method, "push", &[TypeRef::shared_ref(t)],)
            .is_none()
    );
    assert!(
        string_binding
            .find_callable(CallStyle::Method, "push", &[TypeRef::Char])
            .is_some()
    );
    assert!(
        string_binding
            .find_callable(CallStyle::Method, "push", &[TypeRef::U8])
            .is_none()
    );
}

#[test]
fn var_exposes_checked_shared_mutation_methods() {
    let bindings = standard_bindings().unwrap();
    let var_binding = bindings
        .type_by_path("rust::stainless_runtime::Var")
        .unwrap();
    let var = TypeRef::native("rust::stainless_runtime::Var", vec![]);
    let json_error = TypeRef::native("rust::stainless_runtime::JsonError", vec![]);

    let push = var_binding
        .find_callable(CallStyle::Method, "push", std::slice::from_ref(&var))
        .expect("var arrays expose push");
    assert_eq!(push.receiver, Some(Receiver::Mutable));
    assert_eq!(push.return_type, TypeRef::Void);
    assert_eq!(push.rust_result_error, Some(json_error.clone()));

    let string_ref = TypeRef::shared_ref(TypeRef::native("rust::String", vec![]));
    let set_field = var_binding
        .find_callable(CallStyle::Method, "set", &[string_ref, var])
        .expect("var objects expose dynamic member set");
    assert_eq!(set_field.rust_result_error, Some(json_error));
    assert!(matches!(
        &set_field.lowering,
        RustLowering::Method { rust_name } if rust_name == "set_field"
    ));
}

#[test]
fn unsupported_borrowing_and_iterator_methods_are_not_exposed() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let string_binding = bindings.type_by_path("rust::String").unwrap();

    let vec_methods = method_names(vec_binding);
    for unsupported in ["get", "first", "last", "iter", "iter_mut", "drain"] {
        assert!(!vec_methods.contains(unsupported));
    }

    let string_methods = method_names(string_binding);
    for unsupported in ["as_str", "as_bytes", "trim", "split", "chars", "bytes"] {
        assert!(!string_methods.contains(unsupported));
    }
}

fn method_names(binding: &stainless_compiler::interop::NativeTypeBinding) -> BTreeSet<&str> {
    binding
        .callables
        .iter()
        .filter(|callable| callable.style == CallStyle::Method)
        .map(|callable| callable.source_name.as_str())
        .collect()
}
