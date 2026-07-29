use std::collections::BTreeSet;

use stainless_compiler::interop::{
    ArgumentAdaptation, CallStyle, Receiver, RustLowering, TypeRef, standard_bindings,
};

#[test]
fn standard_registry_contains_string_and_vec_in_path_order() {
    let bindings = standard_bindings().unwrap();
    let paths = bindings
        .types()
        .map(|binding| binding.stainless_path)
        .collect::<Vec<_>>();

    assert_eq!(paths, ["rust::String", "rust::Vec"]);
}

#[test]
fn vec_has_default_and_capacity_construction() {
    let bindings = standard_bindings().unwrap();
    let vec_binding = bindings.type_by_path("rust::Vec").unwrap();
    let vec_t = TypeRef::native("rust::Vec", vec![TypeRef::Parameter("T")]);

    let constructor = vec_binding
        .find_callable(CallStyle::Constructor, "Vec", &[])
        .unwrap();
    assert_eq!(constructor.return_type, vec_t);
    assert_eq!(
        constructor.lowering,
        RustLowering::AssociatedFunction {
            rust_path: "::std::vec::Vec::new"
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
    let t = TypeRef::Parameter("T");

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
    let t = TypeRef::Parameter("T");

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
    let t = TypeRef::Parameter("T");

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

fn method_names(
    binding: &stainless_compiler::interop::NativeTypeBinding,
) -> BTreeSet<&'static str> {
    binding
        .callables
        .iter()
        .filter(|callable| callable.style == CallStyle::Method)
        .map(|callable| callable.source_name)
        .collect()
}
