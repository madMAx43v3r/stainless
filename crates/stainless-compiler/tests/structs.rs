use stainless_compiler::analyze;
use stainless_compiler::interop::TypeRef;

#[test]
fn resolves_struct_layout_members_fields_and_data_inheritance() {
    let analysis = analyze(include_str!(
        "../../../docs/ref/02_structs_and_data_inheritance.stl"
    ));

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.semantics.structs.len(), 2);
    let point2 = &analysis.semantics.structs[0];
    let point3 = &analysis.semantics.structs[1];
    assert_eq!(point2.path, ["samples", "Point2"]);
    assert_eq!(point2.fields.len(), 2);
    assert_eq!(point3.base, Some(point2.id));
    assert_eq!(point3.fields.len(), 1);

    let methods = analysis
        .semantics
        .functions
        .iter()
        .filter(|function| function.receiver.is_some())
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    assert!(
        methods
            .iter()
            .all(|method| !method.receiver.as_ref().unwrap().mutable)
    );
    assert!(analysis.semantics.expressions.iter().any(|expression| {
        expression.ty
            == TypeRef::Struct {
                path: vec!["samples".to_owned(), "Point3".to_owned()],
            }
    }));
    assert!(analysis.semantics.expressions.iter().any(|expression| {
        expression
            .field
            .as_ref()
            .is_some_and(|field| field.access_path.len() == 2)
    }));
}

#[test]
fn diagnoses_invalid_struct_layout_and_member_forms() {
    let source = r"struct Base {
    i32 value;
    i32 inside() const { return value; }
};

struct Derived : Base {
    const i32& borrowed;
    i32 value;
};

i32 Derived::undeclared() const {
    return value;
}

i32 ambiguous(const Derived& derived) {
    Derived broken = Derived{Base{1}};
    return derived.value;
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert!(codes.contains(&"SEM009"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES043"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES051"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES047"), "{:?}", analysis.diagnostics);
    assert!(codes.contains(&"RES010"), "{:?}", analysis.diagnostics);
}

#[test]
fn ownership_checks_static_member_receivers_against_arguments() {
    let source = r"struct Counter {
    i32 value;
    void merge(const Counter& other);
};

void Counter::merge(const Counter& other) {
    value += other.value;
}

void conflicting(Counter& counter) {
    counter.merge(counter);
}
";
    let analysis = analyze(source);

    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "OWN003"),
        "{:?}",
        analysis.diagnostics
    );
}
