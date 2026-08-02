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
                arguments: Vec::new(),
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
fn resolves_typed_static_struct_constants_without_instance_storage() {
    let analysis = analyze(
        r"struct RecordKind {
    static const u8 Insert = 0;
    static const u8 Commit = 1;
};

u8 kind() {
    return RecordKind::Commit;
}
",
    );

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let structure = analysis
        .semantics
        .structs
        .iter()
        .find(|structure| structure.path == ["RecordKind"])
        .expect("RecordKind symbol");
    assert!(structure.fields.is_empty());
    assert_eq!(structure.static_constants.len(), 2);
    assert_eq!(structure.static_constants[1].name, "Commit");
    assert_eq!(structure.static_constants[1].ty, TypeRef::U8);
    assert_eq!(structure.static_constants[1].value, "1");
    assert_eq!(analysis.semantics.static_constant_references.len(), 1);
}

#[test]
fn diagnoses_invalid_static_struct_constant_forms() {
    let analysis = analyze(
        r"struct WrongType {
    static const f32 Value = 1;
};

struct RuntimeExpression {
    static const u8 Value = 1 + 2;
};

class WrongOwner {
public:
    static const u8 Value = 1;
};

struct Generic<T> {
    static const u8 Value = 1;
    T data;
};
",
    );

    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "RES125")
            .count(),
        4,
        "{:#?}",
        analysis.diagnostics
    );
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

#[test]
fn move_only_file_handles_require_class_storage() {
    let analysis = analyze(
        r"use rust::std::fs::File;

struct InvalidBlock {
    File file;
};

class ValidBlock {
    File file;
public:
    ValidBlock(File file);
};

ValidBlock::ValidBlock(File file) : file(move(file)) {}
",
    );

    assert!(
        analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "RES092"
                && diagnostic
                    .message
                    .contains("move-only ownership cannot be stored")
        }),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn resolves_exact_constructor_overloads_and_synthesized_defaults() {
    let source = r"use rust::Vec;

struct Value {
    i32 number;
    Value(i32 number);
    Value(u32 number);
};

Value::Value(i32 number) : number(number) {}
Value::Value(u32 number) : number(i32(number)) {}

struct Collection {
    Vec<i32> values;
};

i32 build() {
    Value signed_value = Value(1);
    Value unsigned_value = Value(2u32);
    Value copied_value = Value(signed_value);
    Collection collection;
    return copied_value.number + unsigned_value.number + i32(collection.values.len());
}
";
    let analysis = analyze(source);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.semantics.constructors.len(), 3);
    assert_eq!(
        analysis
            .semantics
            .constructors
            .iter()
            .filter(|constructor| constructor.synthesized)
            .count(),
        1
    );
    assert!(
        analysis
            .semantics
            .constructors
            .iter()
            .all(|constructor| !constructor.is_deleted)
    );
}

#[test]
fn diagnoses_deleted_and_undefined_default_constructors_when_selected() {
    let source = r"struct PrimitiveField {
    i32 value;
};

struct ExplicitlyDeleted {
    ExplicitlyDeleted() = delete;
};

struct Undefined {
    Undefined();
};

void invalid() {
    PrimitiveField primitive;
    ExplicitlyDeleted deleted;
    Undefined undefined;
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    assert_eq!(codes.iter().filter(|code| **code == "RES066").count(), 2);
    assert!(codes.contains(&"RES067"), "{:?}", analysis.diagnostics);
}

#[test]
fn diagnoses_invalid_constructor_signatures_and_initializer_lists() {
    let source = r"struct Value {
    i32 number;
    Value(i32 number);
    Value(const i32& number);
};

Value::Value(i32 number)
    : number(number), number(number), missing(number) {
}

struct MissingInitializer {
    i32 number;
    MissingInitializer();
};

MissingInitializer::MissingInitializer() {
}
";
    let analysis = analyze(source);
    let codes = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect::<Vec<_>>();

    for expected in ["RES055", "RES061", "RES062", "RES065"] {
        assert!(codes.contains(&expected), "{:?}", analysis.diagnostics);
    }
}
