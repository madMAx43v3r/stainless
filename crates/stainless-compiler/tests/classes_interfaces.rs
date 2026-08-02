use stainless_compiler::analyze;
use stainless_compiler::ast::UserTypeKind;
use stainless_compiler::interop::{PointerKind, TypeRef};
use stainless_compiler::resolution::CallTarget;

#[test]
fn resolves_class_and_interface_reference_programs() {
    for source in [
        include_str!("../../../docs/ref/03_interfaces.stl"),
        include_str!("../../../docs/ref/09_value_semantics.stl"),
    ] {
        let analysis = analyze(source);
        assert!(
            analysis.diagnostics.is_empty(),
            "{:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn records_interface_conformance_and_dynamic_calls() {
    let analysis = analyze(include_str!("../../../docs/ref/03_interfaces.stl"));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );

    let named_answer = analysis
        .semantics
        .structs
        .iter()
        .find(|structure| structure.path == ["samples", "NamedAnswer"])
        .expect("NamedAnswer interface");
    let constant_answer = analysis
        .semantics
        .structs
        .iter()
        .find(|structure| structure.path == ["samples", "ConstantAnswer"])
        .expect("ConstantAnswer class");

    assert_eq!(named_answer.kind, UserTypeKind::Interface);
    assert_eq!(constant_answer.kind, UserTypeKind::Class);
    assert!(
        analysis
            .semantics
            .interface_implementations
            .iter()
            .any(|implementation| {
                implementation.implementer == constant_answer.id
                    && implementation.interface == named_answer.id
            })
    );
    assert!(
        analysis
            .semantics
            .calls
            .iter()
            .any(|call| { matches!(call.target, CallTarget::InterfaceMethod(_)) })
    );
    assert!(analysis.semantics.functions.iter().any(|function| {
        function.path == ["samples", "make_answer"]
            && matches!(
                &function.return_type,
                TypeRef::Pointer {
                    kind: PointerKind::Unique,
                    target,
                } if matches!(target.as_ref(), TypeRef::Interface { path, .. } if path == &["samples", "NamedAnswer"])
            )
    }));
}

#[test]
fn diagnoses_invalid_inheritance_missing_contracts_and_class_copies() {
    let analysis = analyze(
        r"interface Contract {
    i32 evaluate(i32 value) const;
};

class Base {
public:
    Base();
    Base(const Base& other);
};

Base::Base() {}

class Derived : Base {
};

class Incomplete : Contract {
public:
    i32 evaluate(u32 value) const;
};

i32 Incomplete::evaluate(u32 value) const {
    return i32(value);
}

void copy_class() {
    Base original;
    Base copied = original;
    original = move(copied);
}
",
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in ["RES118", "RES119", "RES120"] {
        assert!(
            codes.contains(&expected),
            "missing {expected}: {:#?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn enforces_cpp_style_member_access() {
    let analysis = analyze(
        r"class Secret {
    Secret();
private:
    i32 value;
    i32 read() const;
};

Secret::Secret() : value(7) {}

i32 Secret::read() const {
    return value;
}

i32 expose() {
    Secret secret;
    return secret.value + secret.read();
}
",
    );
    let private_diagnostics = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "RES121")
        .count();

    assert_eq!(private_diagnostics, 2, "{:#?}", analysis.diagnostics);
}

#[test]
fn class_members_are_public_by_default() {
    let analysis = analyze(
        r"class Visible {
    Visible();
    i32 value;
    i32 read() const;
};

Visible::Visible() : value(7) {}

i32 Visible::read() const {
    return value;
}

i32 expose() {
    Visible visible;
    return visible.value + visible.read();
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn rejects_access_labels_in_interfaces_and_redundant_sealed_classes() {
    let analysis = analyze(
        r"interface Invalid {
public:
    i32 value() const;
};

class sealed Redundant {
};
",
    );
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(codes.contains(&"SEM014"), "{:#?}", analysis.diagnostics);
    assert!(codes.contains(&"SEM015"), "{:#?}", analysis.diagnostics);
}

#[test]
fn interface_values_require_a_reference_or_owner() {
    let analysis = analyze(
        r"interface Value {
    i32 read() const;
};

Value invalid(Value input) {
    Value local = input;
    return local;
}
",
    );

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RES122"),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn recursive_class_owners_have_finite_send_sync_analysis() {
    let analysis = analyze(
        r"interface NodeView {
    bool empty() const;
};

class Node : NodeView {
    unique_nullptr<Node> next;
public:
    bool empty() const;
};

bool Node::empty() const {
    return !next;
}

unique_ptr<NodeView> make_node() {
    return make_unique<Node>();
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
}

#[test]
fn structs_implement_interfaces_statically_but_cannot_be_erased() {
    let analysis = analyze(
        r"interface Readable {
    i32 read() const;
};

struct Value : Readable {
    i32 number;
    i32 read() const;
};

i32 Value::read() const {
    return number;
}

unique_ptr<Readable> invalid_erasure() {
    return make_unique<Value>{7};
}
",
    );

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "RES028"),
        "{:#?}",
        analysis.diagnostics
    );
}
