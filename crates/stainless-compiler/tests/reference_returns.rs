use stainless_compiler::interop::{
    BindingError, CallStyle, CallableBinding, NativeBindings, NativeTypeBinding, Parameter,
    Receiver, ReturnBorrow, ReturnBorrowError, RustLowering, TypeRef,
};

const TYPE_PATH: &str = "rust::BorrowFixture";

#[test]
fn direct_reference_returns_accept_receiver_and_single_parameter_provenance() {
    let string = TypeRef::native("rust::String", vec![]);
    let callables = vec![
        method(
            "view",
            Receiver::Shared,
            TypeRef::shared_ref(string.clone()),
            ReturnBorrow::Receiver,
        ),
        method(
            "view_mut",
            Receiver::Mutable,
            TypeRef::mutable_ref(string.clone()),
            ReturnBorrow::Receiver,
        ),
        associated(
            "identity",
            vec![Parameter::new("value", TypeRef::shared_ref(string.clone()))],
            TypeRef::shared_ref(string),
            ReturnBorrow::Parameter(0),
        ),
    ];

    assert!(NativeBindings::new(vec![native_type(callables)]).is_ok());
}

#[test]
fn direct_reference_return_requires_provenance() {
    let mut callable = method(
        "view",
        Receiver::Shared,
        TypeRef::shared_ref(TypeRef::U8),
        ReturnBorrow::Receiver,
    );
    callable.return_borrow = None;

    assert_invalid(callable, ReturnBorrowError::MissingProvenance);
}

#[test]
fn mutable_reference_cannot_come_from_shared_receiver() {
    let callable = method(
        "view_mut",
        Receiver::Shared,
        TypeRef::mutable_ref(TypeRef::U8),
        ReturnBorrow::Receiver,
    );

    assert_invalid(callable, ReturnBorrowError::MutableReturnFromSharedSource);
}

#[test]
fn method_reference_return_cannot_be_tied_to_a_parameter() {
    let mut callable = method(
        "select",
        Receiver::Shared,
        TypeRef::shared_ref(TypeRef::U8),
        ReturnBorrow::Parameter(0),
    );
    callable.parameters = vec![Parameter::new("other", TypeRef::shared_ref(TypeRef::U8))];

    assert_invalid(callable, ReturnBorrowError::MethodReturnMustBorrowReceiver);
}

#[test]
fn nonmethod_reference_return_rejects_multiple_reference_parameters() {
    let reference = TypeRef::shared_ref(TypeRef::U8);
    let callable = associated(
        "select",
        vec![
            Parameter::new("first", reference.clone()),
            Parameter::new("second", reference.clone()),
        ],
        reference,
        ReturnBorrow::Parameter(0),
    );

    assert_invalid(
        callable,
        ReturnBorrowError::ExactlyOneReferenceParameterRequired,
    );
}

#[test]
fn reference_bearing_value_returns_remain_deferred() {
    let callable = associated(
        "maybe_view",
        vec![Parameter::new("value", TypeRef::shared_ref(TypeRef::U8))],
        TypeRef::native("rust::Option", vec![TypeRef::shared_ref(TypeRef::U8)]),
        ReturnBorrow::Parameter(0),
    );

    assert_invalid(callable, ReturnBorrowError::ReferenceBearingValuesDeferred);
}

fn method(
    name: &'static str,
    receiver: Receiver,
    return_type: TypeRef,
    return_borrow: ReturnBorrow,
) -> CallableBinding {
    CallableBinding {
        source_name: name.to_owned(),
        style: CallStyle::Method,
        receiver: Some(receiver),
        parameters: vec![],
        return_type,
        return_borrow: Some(return_borrow),
        requirements: vec![],
        lowering: RustLowering::Method {
            rust_name: name.to_owned(),
        },
    }
}

fn associated(
    name: &'static str,
    parameters: Vec<Parameter>,
    return_type: TypeRef,
    return_borrow: ReturnBorrow,
) -> CallableBinding {
    CallableBinding {
        source_name: name.to_owned(),
        style: CallStyle::AssociatedFunction,
        receiver: None,
        parameters,
        return_type,
        return_borrow: Some(return_borrow),
        requirements: vec![],
        lowering: RustLowering::AssociatedFunction {
            rust_path: "::core::convert::identity".to_owned(),
        },
    }
}

fn native_type(callables: Vec<CallableBinding>) -> NativeTypeBinding {
    NativeTypeBinding {
        stainless_path: TYPE_PATH.to_owned(),
        rust_path: "::borrow_fixture::BorrowFixture".to_owned(),
        type_parameters: vec![],
        error_format: None,
        callables,
    }
}

fn assert_invalid(callable: CallableBinding, reason: ReturnBorrowError) {
    let callable_name = callable.source_name.clone();
    assert_eq!(
        NativeBindings::new(vec![native_type(vec![callable])]),
        Err(BindingError::InvalidReturnBorrow {
            type_path: TYPE_PATH.to_owned(),
            callable: callable_name,
            reason,
        })
    );
}
