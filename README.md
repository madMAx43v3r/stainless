# Stainless

Stainless is a new C++-like language that transpiles to Rust.

> **Status:** early implementation. The language design remains provisional;
> the Rust workspace now contains validated native bindings, a lossless lexer
> and Rowan parser, typed CST views, compiler-owned AST lowering, structural
> diagnostics, an initial name/type/call resolver, a typed Rust-shaped HIR, and
> structured Rust emission for the supported function/control-flow,
> struct/class/interface, constructor, checked-exception, standard collections,
> `Vec`/`String`, native
> JSON, the local/function ownership-pointer family, mutex/RW-lock/condition
> synchronization, owned/scoped threads, and first external-wrapper subset. The
> workspace now includes the compact
> `stainless-runtime` used by generated JSON code. An initial move/borrow
> dataflow pass validates that subset. Classes are move-only concrete values;
> interfaces lower to object-safe Rust traits with checked conformance and
> class-owner erasure.

## Hello World

Run the checked-in Stainless program directly:

```sh
cargo run -p stainlessc -- --run examples/hello-world/main.stl
```

After installing `stainlessc`, the same command is simply
`stainlessc --run main.stl` from a project directory.

Output:

```text
Hello, world!
```

Its Stainless source is intentionally small:

```cpp
use rust::println;

i32 main() {
    println!("Hello, world!");
    return 0;
}
```

See [`examples/hello-world`](examples/hello-world/) for the complete
program. It contains no Rust `main.rs`, `build.rs`, or generated source file;
`stainlessc --run` supplies the small Rust entry point and invokes `rustc`.

For standalone checking or Rust generation:

```sh
cargo run -p stainlessc -- --check examples/hello-world/main.stl
cargo run -p stainlessc -- examples/hello-world/main.stl -o hello.rs
```

## Visual Studio Code

The repository includes a code-free VS Code extension for `.stl` syntax
highlighting and basic editor behavior in [`editors/vscode`](editors/vscode).
Try it directly from the repository root:

```sh
code --new-window --extensionDevelopmentPath="$(pwd)/editors/vscode"
```

Packaging and installation instructions are in the
[extension README](editors/vscode/README.md).

## Project charter

Stainless should feel as close to modern C++ as practical while having a small,
predictable translation to stable Rust. It is a new language, not a mode of C++
and not a promise that existing C++ programs will compile unchanged.

The central design rule is:

> Every accepted Stainless feature must have a documented, semantics-preserving
> lowering to Rust. If a C++ feature cannot map cleanly, Stainless must reject or
> omit it instead of approximating it, hiding runtime machinery, or silently
> changing its meaning.

The implementation itself will be Rust so it can be shipped as ordinary Cargo
crates and embedded in existing Rust projects. The compiler should expose a
library API as well as a small command-line program. Its normal output should be
readable, deterministic, stable Rust and should avoid `unsafe`. A feature that
requires `unsafe` is out of scope until its safety contract and use case have
been explicitly designed.

Correct semantics and useful diagnostics take priority over accepting more C++
syntax.

## Provisional syntax examples

The `.stl` files in [`docs/ref`](docs/ref/) show the intended source style.
Stainless source uses a 120-column limit. Function definitions place the
opening brace on the following line; control-flow blocks and lambdas keep the
opening brace on the same line as their header.
They are design references rather than a stable language specification; files
become executable parser or transpilation fixtures as their language slice is
implemented:

- [`01_basics.stl`](docs/ref/01_basics.stl) — functions, namespaces, local
  variables, and control flow.
- [`02_structs_and_data_inheritance.stl`](docs/ref/02_structs_and_data_inheritance.stl)
  — out-of-type function definitions, data inheritance, and base-reference
  coercion.
- [`03_interfaces.stl`](docs/ref/03_interfaces.stl) — static struct interfaces,
  dynamically dispatched class interfaces, and fluent `void` member calls.
- [`04_exact_overloads.stl`](docs/ref/04_exact_overloads.stl) — free-function
  and member-function overloads.
- [`05_ownership_and_containers.stl`](docs/ref/05_ownership_and_containers.stl)
  — Stainless unique/shared owners, `Vec`, guarded `require` bindings, moves,
  and borrows.
- [`06_threads_and_globals.stl`](docs/ref/06_threads_and_globals.stl) —
  namespace-scope storage, synchronized shared-pointer slots, thread-local
  state, and thread handles.
- [`07_rust_interop.stl`](docs/ref/07_rust_interop.stl) — native APIs reached
  through `rust::` and generated bindings for external Cargo dependencies.
- [`08_numeric_types.stl`](docs/ref/08_numeric_types.stl) — fixed-width
  integers, `usize`, `f32`/`f64`, inference defaults, and literal suffixes.
- [`09_value_semantics.stl`](docs/ref/09_value_semantics.stl) — default
  construction, implicit struct copies, explicit moves, and explicit class
  cloning.
- [`10_checked_exceptions.stl`](docs/ref/10_checked_exceptions.stl) —
  C++-style exception structs, `throw`/`try`/`catch`, Java-style checked
  `throws` declarations, propagation, and partial handling.
- [`11_vec_and_string.stl`](docs/ref/11_vec_and_string.stl) — the initial
  compiler-supported `rust::Vec<T>` and `rust::String` surface.
- [`12_reference_returns.stl`](docs/ref/12_reference_returns.stl) — direct
  reference returns tied to a receiver or one reference parameter.
- [`13_range_for.stl`](docs/ref/13_range_for.stl) — shared, mutable, copied,
  and explicitly consumed C++-style range loops.
- [`14_constructors.stl`](docs/ref/14_constructors.stl) — user-defined
  constructors, base/member initializer lists, default member initializers,
  defaulted/deleted constructors, and synthesized struct defaults.
- [`15_checked_exception_subset.stl`](docs/ref/15_checked_exception_subset.stl)
  — the currently compiler-supported checked exception, throwing constructor,
  typed catch, base catch, and bare-rethrow subset.
- [`16_native_result_unwrap.stl`](docs/ref/16_native_result_unwrap.stl) — the
  compiler-supported explicit and target-typed native `Result<T, E>` conversion
  to checked `stainless::RustError`.
- [`17_external_regex_wrapper.stl`](docs/ref/17_external_regex_wrapper.stl) —
  compiler-generated, Cargo-checked wrappers for `regex::Regex::new` and
  `Regex::is_match`, selected by its companion
  [`17_external_regex_wrapper.bindings.toml`](docs/ref/17_external_regex_wrapper.bindings.toml)
  manifest.
- [`18_external_callbacks.stl`](docs/ref/18_external_callbacks.stl) —
  explicit-copy, borrow, and move captures passed to Rust `Fn`, `FnMut`,
  `FnOnce`, function-pointer, and async callback parameters selected by
  [`18_external_callbacks.bindings.toml`](docs/ref/18_external_callbacks.bindings.toml).
- [`19_stored_functions.stl`](docs/ref/19_stored_functions.stl) — non-null
  shared `function<R(A...)>` and unique move-only
  `function_mut<R(A...)>` values, including exact named-function conversion,
  owned lambda captures, invocation, copying, passing, and returning.
- [`20_formatting_macros.stl`](docs/ref/20_formatting_macros.stl) — imported
  `eprintln!`, `format!`, `write!`, and `writeln!`, including checked
  `stainless::FormatError` failures from string writes.
- [`21_json_support.stl`](docs/ref/21_json_support.stl) — compiler-native `var`
  and `null`, JSON array/object literals, null-safe access, reference-counted
  shared aggregate mutation, structural data-struct conversion, parsing
  strings/files, serialization, scalar coercion, and checked
  `stainless::JsonError` failures.
- [`22_pointer_family.stl`](docs/ref/22_pointer_family.stl) — the implemented
  unique/nullable/shared/weak/atomic ownership-pointer subset, including
  nullable guards and synchronized slots.
- [`23_mutex_and_condition.stl`](docs/ref/23_mutex_and_condition.stl) —
  mutex/RW-lock-protected state, scoped inferred guards, condition waits, and
  one/all waiter notification.
- [`24_threads.stl`](docs/ref/24_threads.stl) — owned and scoped Rust threads,
  `Send` capture validation, move-only join handles, typed results, and checked
  thread-panic conversion.
- [`25_collections.stl`](docs/ref/25_collections.stl) — doubly linked lists,
  double-ended queues, and ordered maps and sets.
- [`26_file_io.stl`](docs/ref/26_file_io.stl) — whole-file text/byte I/O,
  filesystem copying, renaming and existence checks, directory operations, and
  checked `stainless::IoError` failures.
- [`27_tuples.stl`](docs/ref/27_tuples.stl) — compiler-known heterogeneous
  tuples used as lexicographically ordered compound map keys.
- [`28_generic_types.stl`](docs/ref/28_generic_types.stl) — invariant generic
  structs and classes with concrete type substitution.
- [`29_class_inheritance.stl`](docs/ref/29_class_inheritance.stl) — single
  class inheritance, explicit base calls, and owner upcasts.
- [`30_switch.stl`](docs/ref/30_switch.stl) — exhaustive, non-fallthrough
  `switch` expressions with scalar and string literal arms, `|` alternatives,
  and a final `else` fallback.
- [`31_while.stl`](docs/ref/31_while.stl) — condition-controlled loops with
  `break` and `continue`.
- [`32_arrays.stl`](docs/ref/32_arrays.stl) — compiler-native fixed-size
  `Array<T, N>`, `usize` const generics, aggregate/default initialization,
  indexing, methods, and range iteration.
- [`33_enums.stl`](docs/ref/33_enums.stl) — scoped, explicitly valued enums,
  fixed-width unsigned representations, enum switch patterns, and implicit
  same-signed widening integer conversion, plus checked integer/String parsing
  and generated member names.

`01_basics.stl`, `02_structs_and_data_inheritance.stl`,
`11_vec_and_string.stl`, `13_range_for.stl`, and `14_constructors.stl` are
currently parsed, resolved, lowered to HIR, emitted as Rust, and compiled by
`rustc` in the test suite, as is the focused
`15_checked_exception_subset.stl` sample, the native-Result
`16_native_result_unwrap.stl` sample, and the formatting-macro
`20_formatting_macros.stl` sample, and the ownership-pointer
`22_pointer_family.stl` sample and the mutex/RW-lock/condition
`23_mutex_and_condition.stl` sample, the owned/scoped thread
`24_threads.stl` sample, and the standard-collection `25_collections.stl`
sample. The `30_switch.stl` and `31_while.stl` control-flow samples and the
`33_enums.stl` scoped-enum sample are also compiled, with focused execution
tests. The JSON
`21_json_support.stl` sample is compiled and executed through Cargo against
the real `stainless-runtime` and `serde_json`. The external
`17_external_regex_wrapper.stl` sample is compiled and executed through Cargo
against the real `regex` crate. The external callback sample is compiled and
executed against a local Rust fixture crate so its generic closure bounds are
checked by Cargo. The stored-function sample is compiled and executed directly
with `rustc`. Other samples remain forward-looking and must become explicit
parser, diagnostic, and transpilation fixtures rather than being allowed to
drift from the implementation.

## Language boundaries

The initial grammar and semantic boundary are frozen below and focus on
constructs with direct Rust equivalents:

- functions, blocks, local bindings, control flow, structs, classes, enums, and
  methods;
- namespaces/modules and explicit imports rather than textual inclusion;
- vtable-free structs with data-only inheritance implemented as composition;
- move-only classes with optional single public class inheritance and interface
  implementation; classes cannot inherit structs;
- interfaces and interface inheritance implemented as Rust traits,
  supertraits, and trait objects where dynamic dispatch is required;
- Stainless ownership types with deliberately restricted Rust lowerings,
  including `unique_ptr<T>`, `shared_ptr<T>`, and their nullable, weak, and
  synchronized counterparts;
- other safe Rust library types under their real names, imported through the
  reserved `rust` namespace, including `rust::Option<T>`, `rust::Result<T>`,
  `rust::Vec<T>`, `rust::String`, `rust::List<T>`, `rust::Queue<T>`,
  `rust::Map<K, V>`, `rust::MultiMap<K, V>`, and `rust::Set<T>`;
- compiler-native `var`, `null`, and JSON literals backed by the compact
  runtime, with checked parsing/mutation and reference-counted object/array
  identity;
- built-in `optional<T>` values with empty and value construction,
  `has_value()`, `value_or()`, and `clone()`; these lower to Rust `Option<T>`
  and contextually convert to `bool` through `is_some()` in conditions, `!`,
  `&&`, and `||`;
- direct use of safe Rust `core`, `alloc`, and `std` APIs plus generated,
  compile-checked wrappers for external Cargo dependencies, all reached through
  the reserved `rust` namespace;
- value semantics, explicit moves, borrowing, and references governed by
  Rust-like ownership rules;
- type inference where C++ would commonly use `auto`;
- function and method overloads resolved by exact parameter types;
- checked exceptions with C++-style control flow, Java-style `throws` effects,
  and generated Rust `Result` values;
- the deliberately constrained generic type declarations specified below.

The following C++ features should not be accepted in the initial language
because they have no direct, general safe-Rust translation:

- the preprocessor, textual `#include`, and C/C++ macros;
- multiple, private, protected, and virtual class inheritance; virtual
  concrete-class methods outside interface contracts; method overriding
  between classes; and C++ RTTI;
- default arguments and C++'s general implicit-conversion sequences; only the
  narrow reference, pointer, and interface bindings explicitly specified below
  are permitted;
- unrestricted templates, specialization, SFINAE, and template metaprogramming;
- raw-pointer arithmetic and manual `new`/`delete` memory management;
- unchecked exceptions, platform exception ABIs, and catching Rust panics;
- C-style variadics, `goto`, and unrestricted unions.

Destructors/`Drop`, operator traits, FFI, general-purpose futures/tasks, and other less-direct
mappings are deferred. Each needs a written source-level semantic model and
lowering rule before it becomes part of the language.

### Initial grammar and semantic freeze

The first compiler uses the following conservative grammar policy:

- Source identifiers are ASCII and match `[A-Za-z_][A-Za-z0-9_]*`. Unicode
  identifiers may be added later without affecting type semantics.
- User generic type declarations use `struct Name<T>` or const parameters such
  as `struct Buffer<T, usize N>`. Type parameters must precede const
  parameters. Parameters are invariant and arguments are explicit. Const
  parameters are compile-time `usize` values and may be supplied by an integer
  literal, another declared const parameter, or a qualified
  `static const usize` member. Generic fields, constructors, member signatures,
  out-of-body definitions, nested type instances, references, and concrete
  construction are implemented. An out-of-body member repeats the owner
  parameters in C++ position, for example
  `const T& Box<T>::get() const`. The repeated arguments must exactly name the
  declared parameters in order.
- Generic arguments must be storable value types; `void`, references, and
  reference-bearing types are rejected. A generic struct receives Rust's
  conditional derived `Clone` implementation, so a concrete instance has
  Stainless struct-copy semantics only when all of its stored concrete values
  can be cloned. Classes remain move-only for every instantiation.
- Generic single data/class bases may use the derived declaration's type
  parameters. Generic interfaces, interface implementation by a generic type,
  default generic arguments, non-`usize` const parameters, specialization,
  generic free functions, and user-written trait bounds are deferred. Compiler
  metadata may still expose supported generic Rust types and methods such as
  `Vec<T>`.
- `sealed` is valid after `interface` and prevents inheritance or
  implementation outside the module. It is also valid after `struct` or
  `class` and prevents use as a data/class base outside the module. `native` is
  not a declaration modifier.
- Lambdas require an explicit capture list. `[value]` copies a copyable value,
  `[name = expression]` creates a new owned capture using normal initialization
  semantics, and `[&value]` creates a non-escaping borrow. A non-copy value in
  an initializer therefore requires `move(value)`. C++-positioned `mutable`
  permits the body to modify by-value captures. Default captures are rejected.
  A lambda may be stored only in an exact `function<R(A...)>` or
  `function_mut<R(A...)>` context; every stored capture must be owned. The
  native-callback slice accepts imported `escape = "call"` and
  `escape = "thread"` contracts. A call callback finishes before its native
  call returns. A thread callback must be `FnOnce`, `Send`, and `'static`, so
  borrowed captures are rejected and every owned capture is checked for
  thread transfer.
- Async is initially a Rust-interop boundary feature. `async` functions and
  trailing-`async` lambdas may use postfix `.await`, but only on direct async
  Stainless calls or Rust calls marked `async = true` in binding metadata.
  Futures are not source-level types: they cannot be named, stored, or spawned
  directly.
- User-defined destructors and `Drop` implementations are not accepted.
  Generated Rust automatically drops locals and fields using Rust scope and
  field-declaration order. Resource-owning native Rust fields retain their
  ordinary Rust `Drop` behavior.
- Stainless `switch (value) { pattern => expression, else => fallback }` is an
  exhaustive expression. It accepts integer, character, boolean, and string
  literal patterns; `pattern1 | pattern2` selects one arm for multiple literal
  alternatives. It requires the final `else` fallback, never falls through,
  requires every alternative to have the exact scrutinee type, rejects
  duplicate alternatives, and lowers directly to a Rust `match`.
  Binding/destructuring patterns, Rust-style `match`, and `if let` remain
  deferred. Native `Result` values initially use ordinary non-consuming query
  methods, compiler-adapted `.unwrap()`, target-typed checked unwrap, or a
  purpose-built Rust adapter.

These restrictions freeze the first parser boundary; accepting more syntax
requires an explicit semantic and lowering extension rather than permissive
parsing followed by rejection.

### Inheritance model

Stainless gives `struct`, `class`, and `interface` distinct roles instead of
using them as nearly interchangeable spellings.

An `interface` is behavior-only:

- It contains method signatures but no instance data, constructors, or
  destructors. Function bodies cannot appear inside the interface.
- Interface inheritance lowers directly to Rust supertraits.
- An interface may inherit only from other interfaces.
- A struct or class may implement one or more interfaces.
- Interface calls on a concrete struct always use static dispatch. Generic
  interface constraints are deferred. A struct cannot be converted to an
  interface reference or owning interface pointer.
- Interface calls may use static dispatch when the concrete class is known or
  Rust trait-object dispatch when a class is converted to a dynamic interface
  value.

A `struct` has a data-only representation:

- Its declaration may contain fields, data-base and interface declarations,
  data-related metadata, and member-function declarations. A member-function
  body cannot appear inside the struct.
- A struct may inherit data only from another struct. This is not subtype
  inheritance: it lowers to an embedded Rust field containing the base value.
- A struct may have at most one direct struct data base. It may additionally
  implement any number of interfaces, and a chain of single data bases is
  allowed, but multiple struct inheritance is rejected.
- A struct may implement interfaces using static dispatch, but cannot inherit
  from a class.
- Inherited fields may use convenient source-level lookup, but the compiler
  lowers that access to the corresponding embedded-field path. Ambiguous field
  names must be diagnosed rather than selected by an implicit precedence rule.
- A reference to a derived struct implicitly coerces to a reference to its data
  base. This lowers to a safe reference projection such as `&derived.base` or
  `&mut derived.base`, follows the normal mutability/reborrowing rules, and may
  traverse multiple levels of the single data-base chain.
- This reference coercion never converts or slices an owned derived value,
  permits a base-to-derived downcast, or introduces runtime type information.
- There are no virtual data bases, compiler-inserted vtable pointers, C++ object
  slicing, or C++ layout/ABI guarantees. Bare interface values are not valid
  fields or locals; dynamic interface metadata is carried by an explicit
  reference or owning pointer.
- Implementing an interface does not change a struct's representation. The
  generated Rust uses an ordinary trait implementation with static dispatch,
  and Stainless prohibits creating a `dyn Interface` from the struct.

A `class` combines move-only identity with behavior and optional single class
inheritance:

- A class may declare its own fields, member functions, and implementations of
  interface methods. As with a struct, only declarations appear inside the
  class; function bodies are defined outside it.
- A class may publicly inherit one class and may additionally implement
  interfaces. It cannot inherit a struct. Multiple, private, protected, and
  virtual class inheritance are rejected.
- Concrete class methods are inherited with C++ name lookup: if a derived class
  declares any function with a given name, that declaration hides the complete
  same-named base overload set. It does not override a base function. When the
  derived class declares no such name, lookup continues through the single base
  chain.
- A base implementation can be selected explicitly as
  `Base::function(arguments...)`. The equivalent
  `this.Base::function(arguments...)` form is also accepted. Calls remain
  statically dispatched; only interface calls participate in virtual dispatch.
- An ordinary class method is statically dispatched and cannot be marked
  `virtual`. An interface method implementation supplies behavior for its
  interface slot; it does not override a method inherited from another class.
- A class base is represented by a compiler-owned reference-counted base
  subobject. This permits safe derived-to-base references and `shared_ptr`
  conversions without unsafe alias pointers. Stainless has no class downcast
  or user destructor, so retaining the base subobject independently does not
  expose a different lifetime or destruction order.
- Classes may therefore require vtable-based dispatch, while structs are
  guaranteed never to do so.

“Vtable pointer” describes the semantic distinction using C++ terminology, not
an object-layout promise. In generated Rust, the vtable metadata will normally
be carried by a `dyn Interface` fat pointer rather than stored inside the
concrete class value. Stainless must not expose or depend on the physical vtable
layout.

Member functions are declared inside their struct or class but defined outside
it using a qualified C++-style name. Stainless has no Rust-style `impl` syntax:

```cpp
use rust::Vec;

struct Buffer : Sequence<i32> {
    Vec<i32> values;
    usize len() const;
};

usize Buffer::len() const {
    return values.len();
}
```

The declaration and definition signatures must match exactly. Struct member
functions are not inherited by data-derived structs and cannot be virtual or
overridden. Reuse of behavior must be explicit: call a function on the embedded
base value or extract a helper function. Listing an interface creates an
implementation obligation: matching member declarations and their out-of-type
definitions must satisfy every required interface function. Rust items imported
through the interop mechanism are external declarations and do not use
out-of-type Stainless definitions.

A data-derived struct may independently declare a function with the same name
and parameter types as a function declared by its data base. This is not an
override and does not create dynamic dispatch. A call on the derived struct uses
the derived declaration; a call through a projected base reference uses the base
declaration.

For example, data inheritance lowers conceptually as follows:

```cpp
struct Point2 {
    f32 x;
    f32 y;
};

struct Point3 : Point2 {
    f32 z;
};
```

```rust
struct Point2 {
    x: f32,
    y: f32,
}

struct Point3 {
    base: Point2,
    z: f32,
}
```

Consequently, a Stainless base-reference argument conceptually lowers as
follows:

```cpp
f32 planar_length(const Point2& point);

Point3 derived = /* ... */;
f32 length = planar_length(derived);
```

```rust
fn planar_length(point: &Point2) -> f32;

let derived: Point3 = /* ... */;
let length = planar_length(&derived.base);
```

An inherited field may be selected explicitly with a C++-style qualified member
access:

```cpp
f32 x = point.Point2::x;
```

`receiver.Base::field` names the `Base` subobject in the receiver's single
data-base chain and lowers to the corresponding embedded-field path. Ordinary
`receiver.field` remains available when lookup finds exactly one field. If a
derived struct and one of its data bases declare the same field name, the
unqualified access is rejected and the source must use the explicit base
qualifier.

### Scoped enums

Stainless enums are fieldless value types with scoped members. Their
fixed-width unsigned representation and every member value are explicit:

```cpp
enum RecordKind : u8 {
    Insert = 0,
    Commit = 1,
    Revert = 2,
};

RecordKind kind = RecordKind::Insert;
u8 encoded = kind;
u32 widened = kind;
RecordKind decoded = RecordKind(encoded);       // throws stainless::EnumError
RecordKind named = RecordKind("Insert");         // throws stainless::EnumError
String name = kind.name();                      // "Insert"
String same_name = String(kind);                // "Insert"
```

The representation must be `u8`, `u16`, `u32`, or `u64`. Member names and
values must be unique, and every value must fit the declared representation.
Enum members have the enum type. They convert implicitly to their declared
representation or a wider fixed-width integer with the same signedness. For
example, a `u8` enum converts to `u8`, `u16`, `u32`, `u64`, or `u128`, but not
to any signed integer. Narrowing and platform-sized `usize`/`isize`
conversions are also rejected. The compiler inserts the corresponding Rust
`as` cast when an integer binding, return, or argument requires it. These
conversions never select between competing overloads. Integer-to-enum
and String-to-enum construction use compiler-generated checked wrappers. An
integer must equal a declared discriminant and a String must equal a member
name; otherwise the wrapper throws `stainless::EnumError`. `String(value)` and
`value.name()` both return the declared member name and cannot throw.

Enums lower to Rust `#[repr(...)] enum` declarations and have copy value
semantics. They cannot contain fields, functions, constructors, generic
parameters, inheritance, or interface implementations. Scoped enum members
may be used as `switch` alternatives, including `|` patterns:

```cpp
u32 weight(RecordKind kind)
{
    return switch (kind) {
        RecordKind::Insert | RecordKind::Commit => 1,
        RecordKind::Revert => 2,
        else => 0,
    };
}
```

The final `else` remains mandatory under the general Stainless `switch` rule,
even when all currently declared enum members are listed.

### Static struct constants

A data struct may group typed compile-time integer constants without adding
fields to any instance:

```cpp
struct RecordTag {
    static const u8 Insert = 0;
    static const u8 Commit = 1;
    static const u8 Revert = 2;
};

u8 kind = RecordTag::Insert;
```

`static` is contextual in this member position and remains an ordinary
identifier elsewhere. These declarations lower to Rust associated constants,
have their declared integer type (`u8` above), and are accessed through the
declaring type with `::`; they occupy no instance storage and cannot be
mutated. Instance access such as `record.Insert` is not supported.

The initial implementation permits `static const` only on non-generic
`struct` declarations, with an explicitly named primitive integer type and an
integer-literal initializer. Runtime expressions, floating-point or object
types, classes, interfaces, and mutable static members are rejected. This
narrow form is sufficient for typed wire-format discriminants without adding
mutable global state. Closed sets of named values should normally use a scoped
enum instead.

### Static member functions

Structs and classes may declare associated functions with contextual `static`:

```cpp
struct Record {
    static Record read(const Vec<u8>& bytes) throws ParseError;
    Vec<u8> write() const;
};

Record Record::read(const Vec<u8>& bytes) {
    // implementation
}

Record record = Record::read(bytes);
```

`static` and `throws` appear only on the in-body declaration. The out-of-body
definition inherits both properties and omits them. A static member has no
`self` receiver, cannot have trailing `const`, and is called through
`Type::function(...)`. Interfaces cannot declare static members.

### Declaration syntax and contextual modifiers

`sealed` follows the declaration kind:

```cpp
interface sealed Sequence<T> {
    usize len() const;
};
```

`sealed` is a contextual declaration modifier, not a globally reserved
keyword; outside the modifier position it remains an ordinary identifier. It
prevents code outside the defining module from inheriting or implementing the
declaration.

The former `native` declaration modifier and bundled API facade are removed.
Rust items are imported through `rust::` paths or exposed through generated
interop bindings; Stainless source does not redeclare the Rust standard
library. The ownership types remain compiler-defined language types and do not
require source-level `native` declarations.

No other declaration-kind modifier is accepted initially. These contextual
spellings are parsed only in their documented post-kind positions, never as
general prefix modifiers.

### Namespaces, imports, access control, and `auto`

Namespace declarations retain C++ syntax. Import declarations use a Rust-like
`use` syntax with `::` paths:

```cpp
use geometry::Point;
use geometry::{length, Point3};
use geometry::Point as Position;
use geometry::*;
```

`crate`, `self`, and `super` provide Rust-like absolute and relative path
anchors. Imports are namespace-scoped, order-independent, and affect name
lookup only; they do not textually include or execute another file. Explicit,
grouped, aliased, and glob imports are supported. If imports make an
unqualified name ambiguous, use is a compile error rather than a
first-declaration-wins choice.

Every native Rust item is rooted under the compiler-provided `rust` namespace:

```cpp
use rust::Vec;
use rust::{Option, Result, String};
use rust::{List, Map, Queue, Set};
use rust::regex::Regex;
```

The virtual root exposes Rust prelude items such as `Vec` and `String` directly,
the Stainless collection aliases `List`, `Queue`, `Map`, and `Set`, standard
crates below `rust::core`, `rust::alloc`, and `rust::std`, and each Cargo
dependency below `rust::<dependency>`. Native Rust names do not enter a
Stainless namespace automatically. Importing `use rust::Vec;` makes the short
name `Vec` available in that scope; without the import it must be written
`rust::Vec`.

Receiver calls remain ordinary after import: a value whose type resolved to
`rust::Vec` may call `values.push(...)` and `values.len()`. Associated
functions, free functions, constants, traits, and macros likewise first enter
name resolution through a `rust::` path or an explicit import from one. This
keeps native items separate from Stainless declarations with identical names.
For example, a project-defined `Vec` can coexist with native `rust::Vec`; an
attempt to import both under the same unqualified name is an ordinary import
collision and requires qualification or an alias.

Crates and source files follow Rust's module layout with `.stl` replacing
`.rs`:

- `src/lib.stl` is a library crate root and `src/main.stl` is a binary crate
  root. A package containing both defines separate library and binary crates,
  as in Rust.
- `mod geometry;` declares a child module and loads either
  `geometry.stl` or `geometry/mod.stl` relative to the declaring module. Both
  forms existing at once is an ambiguity error.
- A `mod child;` declaration inside `geometry.stl` loads
  `geometry/child.stl` or `geometry/child/mod.stl`.
- Additional binary crate roots use Rust's `src/bin/name.stl` and
  `src/bin/name/main.stl` conventions.
- A module file contains that module's body and does not wrap its contents in a
  same-named namespace block.

`namespace name { ... }` is the C++-style Stainless spelling for an inline
module, while `mod name;` declares a file-backed module. A module name may be
declared only once within a parent; C++-style reopening of the same namespace
is rejected so the module tree remains identical to Rust's. Source paths do not
implicitly create modules without the corresponding `mod` declaration.
Unsupported Rust path overrides such as `#[path = ...]` are not accepted.

The transpiler mirrors this tree in generated `.rs` files. `crate` always means
the current mixed Stainless/Rust crate. A dependency in the package's
`Cargo.toml` enters the virtual `rust` root under Cargo's normal dependency
name. External-crate items are exposed through the generated interop mechanism
described below; the raw dependency crate never appears as an unqualified
Stainless namespace.

Member access control uses C++ access-label syntax with a safer public default:

- Members of both a `struct` and a `class` are public by default.
- `public:` and `private:` labels change access for subsequent declarations.
  `protected:` is not supported because Stainless has no behavioral class
  inheritance.
- Interface functions are always public and access labels are rejected inside
  an interface.
- An out-of-type member-function definition has access to its type's private
  members. Private data-base fields remain inaccessible to a derived struct.

Namespace-scope visibility and linkage continue to use the C++-like rules
described for globals below; access labels are not valid at namespace scope.

`auto` is supported for local value bindings when an initializer determines one
exact type:

```cpp
auto count = values.len();
const auto name = "stainless";
```

It is not initially accepted for fields, parameters, function return types, or
declarations without an initializer. Ordinary `auto&` and `const auto&` local
declarations are rejected. The compiler-native guarded `require` declaration
and a range-for binding are the two inferred local-reference forms. A typed
`catch (const Error& error)` binder is a separate compiler-managed reference.
Copy and move rules apply after deduction: a named copyable initializer is
copied, while a named non-copy value still requires `move(value)`.
`auto value = nullptr;` is rejected because the pointee type cannot be inferred,
and a fluent `void` chain receiver cannot be captured with `auto`.

### `while` loops

Stainless supports C++-style `while (condition) statement` loops. The condition
must be `bool` or a nullable pointer test and is evaluated before every
iteration. The body has its own statement scope, and `break` and `continue`
target the nearest enclosing `while`, classic `for`, or range `for` loop.

```cpp
while (current < limit) {
    current += 1;
}
```

### Range-based `for` loops

Stainless supports C++ range-for syntax. The binding determines whether
iteration borrows or copies its elements:

```cpp
for (const auto& value : values) {
    inspect(value);
}

for (auto& value : values) {
    value.normalize();
}

for (auto value : values) {
    consume_copy(value);
}
```

- `const auto& value` creates a shared element borrow and lowers to borrowed
  Rust iteration such as `for value in &values`. The range remains immutably
  borrowed until the loop's last use.
- `auto& value` creates a mutable element borrow and requires a mutable lvalue
  range. It lowers to iteration through `&mut values` and excludes every other
  access to the range while each borrow is active.
- `auto value` does not implicitly consume a named range. It applies ordinary
  Stainless copy construction to each borrowed element, so the element type
  must be copyable.
- `auto value : move(values)` explicitly consumes a named range and moves its
  elements. A fresh temporary range may likewise be consumed without `move`
  because it has no source binding that could be used afterward.
- Explicit `T`, `const T&`, and `T&` bindings are also accepted, but their
  canonical element type must match exactly. Range iteration never introduces
  an implicit conversion for the binding.

The range expression is evaluated exactly once. Its iterator and binding remain
compiler-managed locals, the binding's scope is the loop body, and normal
`break`/`continue` control flow applies. A borrowed loop binding may escape only
through a direct reference return already proven to originate from the range's
owner; it cannot be stored or captured independently.

Range iteration is implemented for `rust::Vec<T>`, `rust::List<T>`,
`rust::Queue<T>`, `rust::Set<T>`, `rust::Map<K, V>`, and
`rust::MultiMap<K, V>`. Ordered-set iteration permits shared, copied, and
consuming bindings but rejects mutable element references because changing an
element could invalidate the set's order. Ordered maps and multimaps use C++
structured bindings:

```cpp
for (const auto& [key, value] : values) {
    println!("{} = {}", key, value);
}
```

Dynamic `var` values can be iterated as JSON arrays with an explicit `var`
binding. The operation takes an owned element snapshot and therefore raises
the checked `stainless::JsonError` when the runtime value is not an array.
`const var&` borrows each snapshot element, while `var` consumes the snapshot's
owned clones. The original array may change during either loop without
changing the current iteration. Mutable `var&` bindings are rejected because
they would expose references through the runtime array lock.

`auto& [key, value]` permits mutation through `value` while `key` remains a
constant reference, preserving key order. A multimap yields one flat pair per
association, including repeated keys; it does not expose its private per-key
storage. Copied and explicitly consuming map bindings are supported when both
component types have the required value semantics. Consuming a multimap also
requires a cloneable key because one owned key can produce multiple flat
pairs. Rust `String` is not implicitly treated as a character range because it
does not implement Rust's owned `IntoIterator` API. Index-aware range syntax
and C++ forwarding references such as `auto&&` are deferred.

### Reserved implementation identifiers

Every identifier beginning with `__` is reserved for the Stainless compiler
and generated Rust bindings. Project source cannot declare a
function, type, namespace, field, parameter, local variable, or other symbol
whose name begins with this prefix. A leading single underscore remains
available to project code.

Generated Rust symbols use a `__stainless` prefix, giving the backend and
wrapper generator a namespace that cannot collide with source declarations.
The lexer still treats these spellings as identifiers; name validation
enforces the reservation and reports their declaration as an error. A public
Rust item whose real name begins with `__` may still be reached through a
generated non-reserved alias.

Project code may call documented compiler-provided operations whose names begin
with `__`, currently the `atomic_ptr` and `atomic_nullptr` slot operations, but
cannot declare, overload, shadow, or replace them.

The top-level `stainless` namespace is also reserved for compiler-provided,
source-visible language types such as `stainless::Exception`. Project code may
refer to documented members of that namespace but cannot declare the namespace
or add declarations to it. A registered non-language facility named
`stainless::X` lowers to `::stainless_runtime::X`. This is a facade over the
compiler's native-binding whitelist, not open Rust lookup: an unregistered
`stainless::X` remains a compile error.

The top-level `rust` namespace is reserved for native Rust name resolution.
Project code cannot declare, reopen, shadow, or add declarations to it. It is a
compiler-owned source view over Rust items and is not emitted as a Rust module.
This reservation does not prevent project code from declaring names such as
`Vec`, `String`, or `Regex`; those remain distinct from `rust::Vec`,
`rust::String`, and `rust::regex::Regex`.

### Primitive numeric types

Stainless uses Rust's explicit primitive numeric names:

- Signed integers: `i8`, `i16`, `i32`, `i64`, `i128`, and `isize`.
- Unsigned integers: `u8`, `u16`, `u32`, `u64`, `u128`, and `usize`.
- Floating-point numbers: `f32` and `f64`.

C++ spellings such as `short`, `int`, `long`, `unsigned`, `size_t`, `float`,
`double`, and `long double` are not type names in Stainless. `usize` and
`isize` have the target pointer width and should primarily be used for indices,
collection lengths, and address-sized quantities. Public data formats should
prefer an explicitly sized integer.

Integer literals use Rust-style expected-type inference. Context wins, so
`u8 value = 1;` is valid. Without a context, a non-negative literal defaults to
`u32`, or to `u64` when its value exceeds `u32::MAX`. Unary `-` similarly gives
an unsuffixed literal an `i32` or `i64` context according to its magnitude.
Thus `auto positive = 1;` is `u32`, `auto large = 4294967296;` is `u64`,
`auto negative = -1;` is `i32`, and `i64 value = 1;` remains `i64`. Values that
do not fit the selected or contextual type are compiler errors; wider types
such as `u128` and `i128` require an explicit context or suffix. Floating
literals follow C++ spelling: an unsuffixed literal such as `3.0` always has
type `f64`, while the `f` suffix in `3.0f` selects `f32`. Rust literal suffixes
such as `3.0f32` and `3.0f64` are not accepted in Stainless source. An expected
`f32` type does not change an unsuffixed literal, so `f32 value = 3.0;` is a
type error and must be written `f32 value = 3.0f;`.

`f32` and `f64` lower directly to Rust's IEEE-754 binary32 and binary64 types,
including their infinities, signed zero, and NaN behavior. There is no implicit
promotion between numeric types: mixed-width arithmetic and overload arguments
must use an explicit conversion.

Explicit primitive casts use constructor-style syntax:

```cpp
u32 narrowed = u32(wide_value);
f64 measurement = f64(integer_value);
```

The allowed primitive conversions and their results are those of Rust's safe
primitive `as` casts, but Rust's `as` spelling is not exposed in Stainless.
This includes Rust's truncation behavior for narrowing integers and saturating
float-to-integer behavior, with NaN converting to zero. Pointer casts are not
primitive conversions and remain forbidden. C-style casts such as
`(u32)value`, `static_cast`, and implicit numeric widening are not supported.
For a non-primitive target, `Type(arguments...)` invokes a constructor rather
than a cast.

Implicit coercion follows the restricted Rust-like rules documented elsewhere:
literal type inference, mutable-to-const reborrowing, safe pointer receiver
borrowing, data-base reference projection, shared-to-weak observation, and
owning interface coercion.
Stainless does not add C++ conversion sequences or use coercions to select an
overload.

Ordinary integer arithmetic follows Rust exactly and is emitted using ordinary
Rust operators. Its overflow behavior therefore follows the generated Cargo
crate's `overflow-checks` setting: with checks enabled, overflow panics; with
checks disabled, Rust's wrapping behavior applies. The usual Cargo development
and release profile defaults may consequently behave differently, and this is
an intentional part of Stainless semantics rather than an accidental backend
difference. Constant-expression overflow and operations such as division by
zero receive the same diagnostics or runtime behavior as the corresponding
generated Rust.

Stainless does not insert its own arithmetic checks or silently select
wrapping operations. Explicit checked, wrapping, saturating, or overflowing
arithmetic may be added later only through documented Stainless APIs with
direct Rust equivalents.

### Characters

Stainless `char` is Rust's `char`: one Unicode scalar value, not a C++ byte-sized
integer. A character literal such as `'x'` or `'🦀'` has type `char`. Character
literals containing zero or multiple scalar values are rejected, and `char`
does not implicitly convert to or from an integer. Byte-oriented data uses
`u8`; the first compiler does not expose Rust byte-literal syntax.

### Strings

Stainless source exposes Rust's owned `rust::String` type with C++ construction
and reference syntax. An ordinary literal such as `"text"` has that type, and
the compiler emits the required owned Rust construction automatically. The
short spelling requires an import:

```cpp
use rust::String;

String owned = "hello";
auto inferred = "world"; // String
usize bytes = owned.len();
```

A borrowed string parameter is written `const String&` after that import, or
`const rust::String&` when fully qualified. When a selected Rust API expects
`&str`, the compiler or generated wrapper borrows the `String` and passes its
UTF-8 string slice. This borrow adaptation happens only after the call target
is known and therefore never participates in overload selection. A string
literal can use the same adaptation without an additional source-level
conversion.

Explicit duplication or conversion to an owned string uses C++ constructor
syntax:

```cpp
String copy = String(path);
```

For an exact supported conversion, this lowers to the corresponding Rust
`String::from(...)` or `From` implementation. Rust construction functions are
backend details and are not written as `String::from(...)` in Stainless source.

`String` is not implicitly deep-copied. A consuming Stainless context still
uses the language's explicit `move(value)` operation, while duplication calls
the Rust API explicitly with `value.clone()`. Rust methods such as `len`,
`is_empty`, `push`, `push_str`, and `into_bytes` keep their real names and
behavior. `as_str` is used inside generated Rust when an API requires `&str`,
but it is not source-callable in the initial subset because Stainless does not
expose Rust's unsized `str` type. Integer indexing remains unavailable because
Rust `String` is UTF-8 and a byte offset need not identify a character
boundary. Embedded null bytes are ordinary data and no implicit C string is
created.

### Initialization, copying, and moving

Stainless never permits uninitialized storage. A declaration without an
explicit initializer requests a Stainless default constructor; it does not
create an indeterminate value. For example, this is initialized by the mapped
zero-argument `Vec` constructor:

```cpp
use rust::Vec;

Vec<i32> values;
```

The explicit form is `Vec()`, never Rust's `Vec::new()`. A safe Rust associated
`new` function is exposed as C++ constructor syntax when it returns `Self`
directly and its parameters are representable. A zero-argument Rust type that
has no such function may use `Default::default()` as its mapped Stainless
default constructor. This mapping is recorded in the compiler's Rust metadata
and rechecked by Cargo. A Rust factory that returns `Result<Self, E>` remains a
fallible factory rather than becoming a default constructor. Primitive numeric
types have no implicit default constructor and therefore require an
initializer.

When no constructor prevents it, a default constructor may be synthesized only
if the struct data base, when present, and every field can themselves be
default-constructed or has a default member initializer. `Type() = default;`
explicitly requests that same compiler-generated parameterless constructor;
parameterized constructors cannot be defaulted. Otherwise the constructor is
implicitly deleted and a default-construction attempt is a compile error.
Stainless may use C++-style `= delete` syntax to make a constructor unavailable
explicitly. Aggregate initialization remains valid only when it initializes
every required field.

User constructors retain C++ declaration and initializer-list syntax:

```cpp
struct Rectangle {
    i32 width;
    i32 height;
    Rectangle(i32 width, i32 height);
};

Rectangle::Rectangle(i32 width, i32 height)
    : width(width), height(height) {
}
```

The declaration is required inside the struct, while its implementation must
be outside. Constructor overloads use the same exact canonical-parameter-type
rules as functions. Generated Rust uses a deterministic free function,
constructs the data base and fields in representation order, creates a hidden
mutable reference only after the value is fully assembled, runs the body, and
returns the completed struct. The current implementation supports this
struct subset for both effect-free and checked throwing constructors.

Like Java, a Stainless constructor may declare checked exceptions:

```cpp
class Session {
    Session(bool available) throws OpenError;
};

Session::Session(bool available) {
    if (!available) {
        throw OpenError{/* ... */};
    }
}
```

The `throws` clause has the same checking rules as a function's clause. Every
checked exception raised by the constructor body, a delegated constructor, a
struct data-base constructor, or a field initializer must be caught within the
constructor or covered by its declared set. The out-of-type definition inherits
the declaration's normalized set, so it does not repeat `throws`. Constructor
overload selection continues to use only the exact canonical parameter types;
`throws` does not distinguish overloads.

Construction is a throwing expression at its call site. The caller must catch
or feed forward every declared constructor exception:

```cpp
Session create_session(bool available) throws OpenError {
    return Session(available);
}
```

If construction fails, no `Session` value is produced. Base and field values
whose initialization completed are destroyed normally; other side effects that
occurred before the throw are not rolled back. The Rust lowering constructs
the data base and fields in declaration order using hidden locals and `?`,
assembles `Self` only after all required values exist, and then executes the
constructor body. An error from the body drops the assembled value. This
avoids partially initialized Rust values and does not require `unsafe`.

A compiler-synthesized default constructor has an empty `throws` set, matching
Java's rule. It is therefore synthesized only when every implicit data-base and
field default construction is non-throwing. If an in-class field initializer
can throw, the type must explicitly declare at least one constructor and every
constructor must declare a covering exception type, again following Java's
checked-initializer rule.

`make_unique<T>(arguments...)` and `make_shared<T>(arguments...)` carry the
selected `T` constructor's checked effect. Their brace forms,
`make_unique<T>{initializers...}` and `make_shared<T>{initializers...}`, use
direct-list initialization. For a Stainless data struct this is the same
field-wise initialization as `T{initializers...}`. For a compiler wrapper such
as `mutex<U>`, the brace list initializes the wrapped `U` before constructing
the mutex. Generated Rust constructs the complete `T` first and places it in
`Box` or `Arc` only after construction succeeds. A failed constructor therefore
produces no owning pointer.

Structs are value types:

- Every struct receives an implicit memberwise copy constructor and copy
  assignment operator when all of its fields are copyable.
- If any field is not copyable, the corresponding synthesized operation is
  implicitly deleted and the compiler reports which field caused the deletion.
- A synthesized copy may lower to Rust `Copy` for trivially copyable fields or
  to generated `Clone` calls for fields that require a deep copy. Stainless
  copyability is therefore not limited to Rust's bitwise `Copy` trait.
- A data base is copied or assigned as another field before the derived
  struct's own fields.

Classes are identity-oriented and never copyable:

- Stainless does not synthesize or permit a class copy constructor or class
  assignment operator.
- A class value may be explicitly move-constructed, passed, or returned, but an
  existing class value cannot be assigned another class value.
- Explicit deep duplication is ordinary class behavior, conventionally a
  `clone() const` member returning `unique_ptr<Class>`. This creates a new
  object; `unique_ptr` itself is not cloneable.
- Copying a `shared_ptr<Class>` or `shared_nullptr<Class>` duplicates only its
  shared ownership handle and continues to refer to the same immutable class
  object; it does not clone that object.

A named value does not move implicitly when passed, used as an initializer, or
assigned: those operations still require `move(value)` for a non-copy value. A
direct local or by-value parameter returned by value is the deliberate
exception. Because control flow ends, `return value;` moves it automatically;
`return move(value);` is accepted but produces a non-blocking redundancy
warning. A fresh temporary can initialize, pass, or return directly because it
has no binding that could be used afterward. Using a moved binding is a compile
error until it is explicitly reinitialized in a context where assignment is
allowed. An active borrow also prevents moving its owner.

`unique_ptr<T>` and `unique_nullptr<T>` are non-copyable owning values that
transfer through `move(value)`. `shared_ptr<T>` and `shared_nullptr<T>` are
implicitly copyable: copying increments the underlying `Arc` strong count and
leaves the source valid. `move(value)` transfers a shared handle without that
increment and invalidates the source binding.

The initial Stainless ownership pass now tracks moves, reinitialization,
conditional control-flow joins, and local reference borrows for the implemented
function subset, so these errors are reported against `.stl` source. It also
checks potentially repeated loop moves conservatively. Lowering `move(value)`
to an actual Rust move lets `rustc` independently verify the result, but
generated-Rust errors are a backstop rather than the primary Stainless
diagnostic mechanism.

### Compiler-native fixed-size arrays

`Array<T, N>` is a fixed-size inline value with no runtime wrapper or heap
allocation. It lowers directly to Rust `[T; N]` and is available without a
`use` declaration. `T` must be a storable value type and `N` must be a
compile-time `usize` value:

```cpp
struct Limits {
    static const usize Width = 4;
};

Array<u32, Limits::Width> values = Array<u32, Limits::Width>{1, 2};
values[2] = 3;
```

Aggregate initialization constructs the supplied leading elements and
default-constructs the remaining elements. Too many elements are rejected.
An array whose length is a const parameter can be default-constructed; a
non-empty aggregate requires a concrete length so the compiler can prove its
arity. Zero-length arrays are valid. `Array<T, N>()` and an uninitialized-looking
declaration both perform Stainless default construction, so they are valid only
when `T` has a non-throwing default constructor.

Arrays support `operator[]`-style indexing with `u8`, `u16`, `u32`, `u64`, or
`usize`, plus `len()`, `is_empty()`, `fill(value)`, and C++-style range
iteration. Unsigned indices are converted to Rust `usize` with a checked
conversion before ordinary bounds checking; signed integers and `u128` are
rejected. `fill()` requires a mutable array and a copyable element type.
Copy/assignment, equality, ordering, and structural `Send`/`Sync` availability
follow the element type. A generic declaration writes const parameters after
type parameters:

```cpp
struct Buffer<T, usize N> {
    Array<T, N> values;
};
```

The compiler emits `struct Buffer<T, const N: usize> { values: [T; N] }`.

### Compiler-known tuples

`tuple<T, U, ...>` is a heterogeneous value type with between two and twelve
elements. It lowers directly to the corresponding Rust tuple; there is no
runtime wrapper or heap allocation. Construction names every element type and
uses ordinary Stainless initialization rules:

```cpp
tuple<u32, String> key = tuple<u32, String>(7u32, "alice");
Map<tuple<u32, String>, i32> values;
values.insert(move(key), 42);
```

Tuples are copied only when every element is copyable; otherwise their elements
obey the ordinary explicit-`move()` rules. Rust supplies lexicographic
`PartialEq`, `Eq`, `PartialOrd`, and `Ord` implementations when every element
supports the corresponding trait, so tuples can be compound keys for
`Map<K, V>` and `Set<T>`. Tuple elements cannot contain references. Rust-style
numeric projection accesses an element directly (`key.0`, `key.1`, and so on),
with the usual shared or mutable place semantics. Map structured bindings can
bind a tuple key as one value but do not recursively destructure it.

### References and borrowed returns

References may be function parameters, direct function return types, and
explicitly typed local bindings. The implicit receiver of a member function is
also a reference:

```cpp
void inspect(const Config& value); // lowers to &Config
void update(Config& value);        // lowers to &mut Config

struct Config {
    const String& name() const; // lowers to &self -> &String
    String& name_mut();         // lowers to &mut self -> &mut String
};
```

A reference cannot be declared as a field, namespace-scope variable, container
element, or type-alias target. A direct reference return is permitted only when
its lifetime has one unambiguous source:

- A member-function or interface-function reference return is tied exclusively
  to its receiver. A `const T&` result may come from a const or mutable
  receiver; a `T&` result requires a mutable receiver. A value-consuming
  receiver cannot return a reference.
- A free or static function returning a reference must have exactly one
  reference parameter, and the result is tied to that parameter. Functions
  with zero or multiple reference parameters cannot return a reference in the
  initial language.
- Constructors cannot return references.
- Every returned expression must be a borrow of the declared source or one of
  its subobjects. Returning a reference to a local, temporary, by-value
  parameter, guarded `require` binding, or unrelated global is rejected.
- Only a direct `T&` or `const T&` return is initially accepted. Values that
  contain references, such as `Option<const T&>`, tuples containing references,
  borrowing iterators, and references to references are deferred.

These rules let the compiler infer the Rust lifetime without adding lifetime
syntax to Stainless. They lower directly to Rust lifetime elision for methods
and single-borrow functions:

```cpp
const String& identity(const String& value) {
    return value;
}

const String& Config::name() const {
    return name_;
}
```

```rust
fn identity(value: &String) -> &String;
fn name(&self) -> &String;
```

An explicit local reference must be initialized immediately from a compatible
reference-valued expression and cannot be rebound:

```cpp
const String& name = config.name();
```

The borrow of `config` remains active until the last use of `name`. Moving or
mutating the owner while that borrow is active is rejected; a mutable returned
borrow additionally excludes every other access to the owner. Rust's
non-lexical lifetime analysis verifies the emitted borrow boundaries, while
the Stainless semantic pass remains responsible for source-level diagnostics.
Ordinary `auto&` and `const auto&` inference remains unavailable; guarded
`require` declarations and range-for bindings retain their dedicated
inferred-reference syntax.

Reference binding follows these rules:

- `T&` requires an exclusive mutable argument and prevents all conflicting
  access for the duration of the call.
- `const T&` creates a shared borrow. A `T&` may implicitly reborrow as
  `const T&`.
- References are non-null. A parameter reference may escape only as the direct
  return of a function whose declared return is tied to that parameter.
  Guarded and exception references cannot escape their lexical scope or hidden
  owner lifetime.
- A derived-struct argument may project to a data-base reference after the
  function has been selected.
- Passing a `unique_ptr<T>` to a `T&` or `const T&` parameter borrows its
  pointee. A `unique_nullptr<T>` may bind to either reference only where it is
  proven non-null. Passing a `shared_ptr<T>` may bind to `const T&`; a
  `shared_nullptr<T>` may do so only where it is proven non-null.

References to ownership handles are forbidden in parameters, local bindings,
and return types:

```cpp
void invalid(unique_ptr<T>& value);        // rejected
void invalid(unique_nullptr<T>& value);    // rejected
void invalid(const shared_ptr<T>& value);  // rejected
void invalid(shared_nullptr<T>& value);    // rejected
const shared_ptr<T>& invalid_result();       // rejected
```

Owning handles must be passed by value: unique owners require an explicit move,
while shared owners copy implicitly. The synchronized pointer slots
`atomic_ptr<T>` and `atomic_nullptr<T>` are the exceptions and may appear in
otherwise valid parameter, local, or direct-return reference positions because
their APIs synchronize access to the binding.

### Guarded non-null reference bindings

Stainless provides a compiler-native declaration that combines nullable-owner
checking with a named, non-null pointee reference:

```cpp
void configure(unique_nullptr<Config> maybe) {
    auto& config = require(maybe);
    config.version = 2;
}
```

In a `void` function, the semicolon form implicitly executes `return;` when the
owner is null. A failure block in a `void` function may perform work before
that implicit return:

```cpp
auto& config = require(maybe) {
    rust::println!("configuration is missing");
}
```

If the owner is null, this prints the message and then returns from the
enclosing function. A function returning a value must instead provide an
explicitly diverging failure block:

```cpp
auto& config = require(maybe) {
    return -1;
}
```

`const auto&` is also accepted. The shorthand is an ordinary declaration
statement ending in a semicolon. In the explicit form, the failure block
replaces that semicolon. `require` is recognized contextually only in these
initializer forms; it is not a runtime function and cannot be overloaded or
shadowed for this use.

The initial form has these rules:

- The operand must have type `unique_nullptr<T>` or `shared_nullptr<T>` and is
  evaluated exactly once. Applying `require` to a non-null owner or an
  unrelated `Option` value is a type error.
- If the owner is null, the semicolon form returns from its enclosing `void`
  function. It is rejected in a value-returning function.
- When an explicit failure block is present, it executes on null. In a `void`
  function, reaching the end of that block implicitly returns from the
  function. An explicit `return`, `break`, `continue`, or other diverging
  operation takes precedence.
- In a value-returning function, the failure block must not fall through. Every
  path must return a value, `break` or `continue` an enclosing loop, or perform
  another proven-diverging operation.
- `auto&` applied to a mutable `unique_nullptr<T>` binds a mutable `T&`;
  `const auto&` or a const unique owner binds `const T&`.
- A shared owner always binds `const T&`, even when `auto&` is written, because
  pointees owned through `shared_ptr` and `shared_nullptr` are immutable.
- The binding refers to the pointee, not to the ownership handle, so this form
  does not weaken the prohibition on pointer-handle references.

The reference may be used for member access or passed to compatible reference
parameters, but it cannot be returned, stored, captured by an escaping closure,
or sent to another thread. It cannot be rebound. The owner cannot be assigned
or moved while the reference is live; a mutable unique-owner binding also
excludes other access to its pointee. The borrow ends after its last use where
Rust's non-lexical lifetime analysis permits that result.

When the operand is a temporary, the compiler materializes a hidden owner whose
lifetime covers the reference. This makes an atomic nullable-pointer snapshot
concise and safe:

```cpp
auto& config = require(slot.__load()) {
    return -1;
}
```

For a shared snapshot this lowers conceptually to Rust `let ... else`:

```rust
let __stainless_owner = slot.__load();
let Some(config) = __stainless_owner.as_deref() else {
    return -1;
};
```

A semicolon-form declaration in a `void` function uses the same lowering with
`return;` in the `else` branch. For a failure block that can fall through, the
compiler appends `return;` to that branch:

```rust
let Some(config) = maybe.as_deref() else {
    println!("configuration is missing");
    return;
};
```

A mutable named unique owner instead uses `as_deref_mut()`. The lowering does
not clone an `Arc<T>` merely to create the reference;
`atomic_nullptr::__load` has already produced the owning snapshot whose
lifetime is being extended.

### Fluent `void` member calls

Every non-static member function declared with a `void` return type is
implicitly chainable:

```cpp
Buffer buffer;
buffer.push(10).push(20);
usize count = buffer.push(30).len();
```

This is conceptually similar to returning `self` by reference, but it does not
create a Stainless reference value or participate in the returned-borrow
rules. The result of a `void` member call is a restricted *chain receiver* that
may only be the immediate receiver of another member function call. It cannot
be stored, passed as an argument, returned, or used for field access:

```cpp
auto invalid = buffer.push(10); // rejected: a chain receiver cannot escape
```

A non-const `void` member produces a mutable chain receiver; a `void` member
declared `const` produces a const chain receiver. Consequently, a mutating
function cannot follow a const function in the same chain. Free functions and
static member functions have no receiver and retain ordinary `void` semantics.
Function bodies still use `return;` and cannot return an expression.

This fluent rule applies only to Stainless-declared `void` members. A Rust
method returning `()` retains its exact Rust API and cannot be chained unless
its Rust return type itself supports the next operation.

The compiler evaluates the original receiver exactly once, invokes each member
from left to right, and reuses that receiver until a non-`void` call terminates
the chain. Conceptually:

```cpp
buffer.push(10).push(20);
```

lowers to:

```rust
Buffer::push(&mut buffer, 10);
Buffer::push(&mut buffer, 20);
```

Generated Rust functions continue to return `()`: they do not return `&Self` or
`&mut Self`. This avoids extending a borrow across calls and also supports
dynamic interface calls without adding reference-returning trait methods. A
complex or temporary receiver is first evaluated into a hidden local so its
side effects occur once and it remains alive for the chain.

For `unique_nullptr<T>` or `shared_nullptr<T>`, the receiver must already be
proven non-null. A nullable unique owner reuses a hidden `&mut T` or `&T` based
on receiver mutability. A nullable shared owner reuses its hidden `&T`, and
only const member functions are available through it.

### Function overloading

Stainless supports function and method overloads with deliberately simpler
rules than C++:

- Overload identity and candidate matching use canonical value types. Type
  aliases are normalized and an outer parameter `T`, `T&`, or `const T&` all
  contribute the same canonical value type `T` to the overload key.
- An overload is selected using the exact canonical value types of its argument
  expressions. The compiler must not insert numeric widening, pointee
  dereferencing, data-base projection, user-defined conversions, or other
  conversions to make a candidate match.
- After selection, the compiler validates and applies the chosen declaration's
  value-copy, move, or reference-binding semantics. These semantics never rank
  candidates.
- Data-base reference coercion is likewise not used to make an overload
  candidate match. It may be applied after a function has already been selected
  unambiguously, such as for a call to a non-overloaded function.
- Two functions cannot overload one another when their corresponding canonical
  value types are identical and only value/reference passing mode or `const`
  differs. For example, `f(T)`, `f(T&)`, and `f(const T&)` are conflicting
  declarations rather than an overload set.
- The same rule applies to the implicit member-function receiver:
  `T::f()` and `T::f() const` cannot coexist.
- The implicit `T&` to `const T&` reborrow is applied only after a function has
  been selected; it never ranks or selects an overload.
- The return type does not participate in overload selection.
- A call that has no exact match, or more than one exact match, is a compile
  error with the candidate signatures shown in its diagnostic.
- Default arguments and variadic overloads are not supported.
- Every overload lowers to a unique, deterministic Rust name derived from its
  fully qualified Stainless name and canonical parameter types.

For example, these Stainless declarations:

```cpp
use rust::String;

i32 parse(i32 value);
i32 parse(String value);
```

use the initial versioned textual encoding:

```text
__stainless_v1_f_2_7_samples_5_parse__p_1_i32
__stainless_v1_f_2_7_samples_5_parse__p_1_n_2_4_rust_6_String
```

The prefix contains the mangling version and item kind. A path is encoded as
its segment count followed by each segment's decimal byte length and ASCII
contents. `p` introduces the canonical parameter count. Primitive types use
their Stainless spelling; `n` introduces a named-type path; and `g` introduces
a named generic type followed by `a`, its argument count, and recursive type
encodings. A member's owning type is part of its path. Return types, `throws`,
reference passing mode, and parameter `const` are excluded because they do not
distinguish legal overloads. The ASCII identifier restriction makes every
encoded name a valid Rust identifier, length prefixes make the encoding
unambiguous, and the reserved `__` prefix prevents source collisions. Any
incompatible encoding change increments `v1`; no randomized or
implementation-defined hash participates.

### Checked exceptions

Stainless preserves C++-style `throw`, `try`, and `catch` control flow but makes
exceptions checked in the Java sense. Every potentially escaping exception
type appears in the function signature:

```cpp
use rust::String;

Config load(const String& path) throws IoError, ParseError;

Config load(const String& path) {
    String source = read_file(path);
    return parse_config(move(source));
}
```

For a const member-function declaration, `throws` follows the C++-style member
qualifier:

```cpp
Config Loader::load(const String& path) const throws IoError;
```

Constructor declarations place the same clause after their parameter list:

```cpp
Loader::Loader(const String& path) throws IoError;
```

Constructor invocation participates in the same mandatory catch-or-feed-forward
analysis as an ordinary throwing call. Constructor bodies, data-base and field
initialization, failed-construction cleanup, and the non-throwing synthesized
default constructor are specified in the initialization section above.

Every exception type is a struct whose single data-base chain ends at the
compiler-provided `stainless::Exception` struct:

```cpp
struct IoError : stainless::Exception {
    String path;
};

throw IoError{
    stainless::Exception("input could not be read"),
    String(path)
};
```

`stainless::Exception` is a compiler-provided data struct, not a source-level
interface and not an attempt to add a type to Rust's `std` namespace. It stores
an owned `String` message. The backend supplies `Display` and
`std::error::Error` implementations for generated exception types, so ordinary
Rust error formatting and `.to_string()` work on the Rust side. Exception
structs remain ordinary vtable-free Stainless structs and may add fields or
derive through the normal single data-inheritance chain. A struct that does not
ultimately derive from `stainless::Exception` cannot appear in `throws`,
`throw`, or a typed `catch`.

The compiler also provides native-error exceptions:

```cpp
// Conceptual compiler-provided declaration; project code cannot redeclare it.
namespace stainless {
    struct RustError : Exception {
    };

    struct IoError : Exception {
    };

    struct FormatError : Exception {
    };

    struct JsonError : Exception {
    };

    struct EnumError : Exception {
    };

    struct ThreadError : Exception {
    };
}
```

`stainless::RustError` represents an error value crossing from a native Rust
`Result` into Stainless's checked exception model. It retains an owned
human-readable message but not the native Rust error's concrete type or fields.
Consequently project code catches `stainless::RustError`; it cannot catch or
downcast to the Rust `E` type.

`stainless::IoError` is the narrower exception produced by standard filesystem
operations. Its inherited `message` contains Rust's human-readable
`std::io::Error` display text. Platform-specific error codes and Rust
`ErrorKind` values are not exposed in the initial API.

`stainless::FormatError` is the narrower checked failure produced by
`write!` and `writeln!`. Its message is obtained from Rust's
`std::fmt::Error`. Like every checked exception, it must be caught or listed in
the enclosing function's `throws` clause.

`stainless::EnumError` is produced by checked enum construction when an
integer does not equal a declared discriminant or a String does not equal a
declared member name. The inherited message identifies the enum and rejected
value.

`stainless::ThreadError` is produced when an owned thread fails during
`join()`, or when a panic escapes a lexical `thread::scope`. String and
`String` panic payloads become its message; other Rust panic payloads use a
stable fallback message.

The `throws` clause is an unordered set of canonical exception-struct types.
Duplicate entries and entries made redundant by another listed data base are
rejected. A declared base exception covers every exception derived from it, as
in Java; otherwise generated ordering and diagnostics use fully qualified type
identity so they are deterministic. Omitting `throws` from a declaration means
the exception set is empty: the function cannot allow any Stainless exception
to escape. A definition matched to an earlier declaration instead inherits its
declared exception set. `noexcept` is therefore redundant and is not part of
the initial syntax.

An ordinary call keeps C++/Java syntax. There is no source-level Rust `?`
operator:

```cpp
Config reload(const String& path) throws IoError, ParseError {
    return load(path); // either exception is automatically fed forward
}
```

The compiler computes the effect of each call. Every exception must be handled
by a lexically enclosing `try` statement or covered by the same type or one of
its data bases in the enclosing function's `throws` set. Calling a throwing
function from an effect-free function without catching every possible type is
a compile error. Propagating into a declared base or superset is allowed and
does not slice or otherwise convert the exception object.

`throw expression;` requires an exception-struct value and preserves its
concrete type. That type must be caught within the function or covered by its
declared `throws` set. Throwing a named value follows the ordinary Stainless
copy/move rules, so a named non-copy value requires `throw move(error);`. Bare
`throw;` is accepted only inside a `catch` handler and rethrows the currently
handled allocation without copying or reallocating it.

Handling uses C++ syntax:

```cpp
try {
    Config config = load(path);
    use(config);
} catch (const IoError& error) {
    report_io_error(error);
} catch (const ParseError& error) {
    report_parse_error(error);
} catch (...) {
    report_unknown_error();
}
```

Typed catches use the exception struct's data-inheritance chain. A handler
matches when its declared type is the concrete exception type or one of that
type's exception data bases. Handlers are tested in source order, so a base
handler placed before a derived handler makes the latter unreachable and is a
compile error. Interface conformance, numeric conversion, and other coercions
do not participate. `catch (const stainless::Exception& error)` catches every
Stainless exception; `catch (...)` catches every remaining exception without a
binder and must be last.

Only `catch (const E& error)` and `catch (...)` are initially supported. A
typed catch binder is a non-null, compiler-managed const reference to a hidden
owned exception value. It exists only for the handler body and cannot be
returned, stored, moved, or sent to another thread. A bare rethrow retains the
hidden owned value; it does not copy through that reference.

A `try` statement may handle only part of a call's exception set:

```cpp
Config load_or_default(const String& path) throws IoError {
    try {
        return load(path);
    } catch (const ParseError& error) {
        return Config{/* defaults */};
    }
}
```

Here `ParseError` is consumed by the handler and `IoError` is fed forward
because it is declared by `load_or_default`. An exception raised by a catch
handler is considered by an enclosing `try` or by the function's `throws`
clause; sibling handlers do not catch one another's exceptions.

Rust structs do not have C++ derived-to-base object subtyping. A Rust
`Box<Exception>` could contain only an `Exception` value; converting a derived
struct to it would discard the derived fields. The backend therefore gives
each exception struct a compiler-private trait implementation and uses one
erased owning carrier:

```rust
trait __StainlessException: std::error::Error {
    fn __project(
        &self,
        target: std::any::TypeId,
    ) -> Option<&dyn std::any::Any>;
}

type __ExceptionBox = Box<dyn __StainlessException>;

fn load(path: &String)
    -> Result<Config, __ExceptionBox>
{
    let source = read_file(path)?;
    let config = parse_config(source)?;
    Ok(config)
}
```

Conceptually, `Result<T, Box<stainless::Exception>>` is therefore close, but the
actual Rust type is `Result<T, Box<dyn __StainlessException>>`. The compiler
implements Rust's `Debug`, `Display`, and `std::error::Error` traits for each
exception struct using its `stainless::Exception` base, making the generated
errors usable by ordinary Rust tooling without exposing those Rust traits to
Stainless source.

The private `__project` operation is generated for every exception struct. It
returns a safe `Any` reference to the concrete value or one of its embedded
exception data bases. This permits a typed base catch without pointer
arithmetic or `unsafe`; it uses compiler-private Rust `TypeId`/`Any` inspection
that Stainless source cannot access. The catch site downcasts that projected
reference to its statically known handler type. The source compiler still
checks the exact `throws` hierarchy even though all generated Rust functions
share the same erased error type.

Creating a new exception with `throw` boxes the complete concrete struct and
therefore normally performs one allocation. Propagation and bare rethrow move
the existing box without reallocating it. Catching borrows the value stored in
that box. The compiler may eliminate an allocation only when doing so preserves
these observable lifetime and destruction semantics.

The trait-object vtable metadata is stored in the `Box` pointer, not in the
exception struct. Consequently exception structs retain the same no-vtable
representation guarantee as every other Stainless struct. This compiler-only
erasure at a `throw` boundary does not permit source code to convert arbitrary
structs into interface pointers.

Normal `return value;` in a throwing function lowers to `Ok(value)`, while
`throw error;` lowers to `Err(Box::new(error))`. A throwing `void` function
lowers to `Result<(), __ExceptionBox>`; `return;` and normal fallthrough produce
`Ok(())`. Propagation uses an explicit generated `match` equivalent to Rust's
`?`, and `try`/`catch` lowers to structured inspection of the boxed error. The
compiler must preserve C++
control-flow behavior for `return`, `break`, and `continue` inside a `try`
block rather than accidentally targeting a generated Rust closure.

Rust drops already-initialized locals when `Result` propagation returns early.
This supplies the cleanup behavior expected while unwinding an exception,
without using Rust panic unwinding. Stainless `catch` handles only declared
Stainless exceptions. Rust panics, bounds-check panics, allocation failure, and
foreign exceptions are outside this model and cannot normally be caught. The
one compiler-defined boundary is thread joining: a worker panic becomes a
checked `stainless::ThreadError`, as described below.

The `throws` set is part of Stainless type checking and interface compatibility
but not of overload identity or deterministic overload mangling. Two
declarations cannot overload only by changing `throws`. An out-of-type
definition inherits its declaration's normalized set and does not repeat the
clause. If a definition nevertheless spells an explicit `throws` clause, it
must match. An interface implementation may expose a subset or derived
specialization of the interface function's declared exceptions; generated
trait code uses the same `__ExceptionBox` carrier at the dispatch boundary.

Rust functions returning `Result<T, E>` retain that type in Stainless.
The `unwrap()` method on native `rust::Result<T, E>` is compiler-adapted: it
consumes its receiver, returns the `T` inside `Ok`, and throws
`stainless::RustError` when it contains `Err`. The checked effect must be caught
or listed in the enclosing function's `throws` clause. No per-error adapter is
required, and the backend never calls Rust's panicking implementation.

A fresh result temporary is consumed directly, avoiding a named intermediate:

```cpp
Regex expression = Regex::new("^stainless$").unwrap();
```

A method call on a named result explicitly consumes and invalidates that
binding:

```cpp
Result<Regex, RustRegexError> compiled = Regex::new("^stainless$");
Regex expression = compiled.unwrap();
```

Using `compiled` after the call is a use-after-consume error. This is a
specified consuming operation rather than an implicit move caused by ordinary
function argument passing. An unrelated Stainless member function named
`unwrap` remains an ordinary method.

Stainless also inserts the same checked unwrap when a target of exact type `T`
is initialized or assigned from `rust::Result<T, E>`:

```cpp
Regex expression = Regex::new("^stainless$");
```

This is not an implicit conversion used by overload resolution. It is a
target-typed assignment rule applied only after the destination type and
right-hand expression type are known. The default inserted operation has
checked effect `throws stainless::RustError`, exactly like an explicit
`.unwrap()`, so the enclosing function must catch or declare it. A
compiler-owned native subsystem may select a more precise checked exception:
the JSON runtime's native error type maps to `stainless::JsonError` for parsing
and mutation.

The initial rule applies to explicitly typed variable and field initialization,
aggregate field initialization, and assignment to an existing value. It does
not apply to function arguments, return expressions, overload matching, or
`auto` deduction. Thus `auto expression = Regex::new(...)` deduces the native
`Result`, while `auto expression = Regex::new(...).unwrap()` deduces `Regex`.
Normal copy/move rules still apply to a named result: a non-copyable named
`result` requires `Regex expression = move(result);`, after which the compiler
inserts the checked unwrap. A fresh result temporary needs no explicit move.

When the compiler can prove that `E` implements Rust `Display`, the generated
shim uses that representation as the exception message. Otherwise it uses
`Debug` when that trait is proven. If neither formatting trait is available,
the operation still works but uses the fixed message `"native Rust operation
failed"`. This choice is made statically from verified Rust trait metadata; it
does not use unsafe inspection or unstable specialization. The original `E`
value is consumed and dropped after the message is produced.

The initial implementation proves `Display` for primitive error values and
`rust::String`, and consumes declared `Display`/`Debug` capability from
validated native binding metadata. The first such external proof is
`rust::regex::Error: Display`; the compiler-owned JSON error also proves
`Display`. Types without a proof receive the fixed fallback.

This compiler-generated match replaces Rust's panicking `Result::unwrap`,
keeping panic unwinding outside the Stainless exception model. The standalone
CLI entry shim catches an unhandled checked exception, prints it, and exits
with failure; other generated Rust boundaries retain their declared result.

After receiver type and method resolution, a `.unwrap()` call whose receiver is
native `rust::Result<T, E>` becomes a dedicated `UnwrapRustResult` HIR
operation. Detection uses the resolved receiver type and Rust item identity,
not method spelling alone, so project-defined methods with the same name are
unaffected. In the simplest feed-forward context, the generated Rust is
conceptually:

```rust
let expression = match compiled {
    Ok(value) => value,
    Err(error) => {
        let message = /* Display, Debug, or the fixed fallback */;
        return Err(Box::new(__RustError::from_message(message)));
    }
};
```

The actual backend uses the compiler-private erased exception carrier described
above. Within a Stainless `try` block, the `Err` branch targets that block's
generated handler boundary instead of returning past it. The wrapper is
specialized and emitted inline at the call site; there is no user-visible
conversion function and no requirement that all native error types share a
Rust representation.

A native call returning `Result<T, E>` remains an ordinary `Result` unless
source calls its compiler-adapted `.unwrap()` method or places it in one of the
exact target-typed initialization or assignment contexts above. Both forms
lower to the same `UnwrapRustResult` HIR operation.

### Dynamic allocation and owning pointers

Stainless keeps its deliberately restricted pointer vocabulary even though
other library APIs use Rust names. These are compiler-known ownership types,
not aliases that expose every method of their Rust lowering:

| Stainless | Rust lowering | Semantics |
| --- | --- | --- |
| `unique_ptr<T>` | `Box<T>` | One non-null owner; movable but not copyable |
| `unique_nullptr<T>` | `Option<Box<T>>` | Nullable unique owner |
| `shared_ptr<T>` | `Arc<T>` | Non-null, implicitly copyable, immutable shared ownership |
| `shared_nullptr<T>` | `Option<Arc<T>>` | Nullable shared ownership |
| `weak_ptr<T>` | `Weak<T>` | Non-owning observation |
| `atomic_ptr<T>` | Initially `RwLock<Arc<T>>` | Replaceable non-null shared slot |
| `atomic_nullptr<T>` | Initially `RwLock<Option<Arc<T>>>` | Replaceable nullable shared slot |

The compiler implements all rows in this table for local, parameter, and return
values. `make_unique<T>(...)` and `make_shared<T>(...)` run the selected
Stainless or native constructor before allocating, while their `{...}` forms
direct-list initialize the pointee; checked construction exceptions propagate
normally. Pointee fields and methods use `.`, shared and weak handles copy by
cloning their Rust handle, and unique plus atomic values participate in the
move checker. Nullable facts are tracked for named bindings through
construction, assignment, guards, `nullptr` comparisons, and continuing branch
merges. Namespace-scope pointer initialization, interface pointees, and the
`require(...)` declaration shorthand remain later slices.

Native `rust::Option<T>` remains available for ordinary optional values, but
Stainless rejects a pointer or synchronized pointer-slot type as its direct
`T`. Pointer nullability must use `unique_nullptr<T>` or
`shared_nullptr<T>`, preserving the ownership-specific refinement and
conversion rules instead of allowing both representations.

Allocation uses `make_unique<T>(...)`, `make_shared<T>(...)`, or their brace
forms. There is no owning `new`, `delete`, placement allocation, or dynamically
allocated C-style array. `drop(move(value))` may consume a named owning handle
early; otherwise destruction is automatic. Allocation failure follows ordinary
Rust allocation behavior and aborts rather than throwing a Stainless exception.

`unique_ptr<T>` and `unique_nullptr<T>` provide exclusive ownership:

- `unique_ptr<T>` is non-null, has no default constructor, and cannot be
  assigned `nullptr`.
- Default construction or assignment from `nullptr` makes a
  `unique_nullptr<T>` null.
- A mutable unique owner permits mutable pointee access when present and not
  borrowed.
- Neither type is copyable. Moving transfers its `Box<T>` or null state and
  invalidates the source.

`shared_ptr<T>` and `shared_nullptr<T>` deliberately provide immutable shared
data:

- `shared_ptr<T>` is non-null, has no default constructor, and cannot be
  assigned `nullptr`.
- Default construction or assignment from `nullptr` makes a
  `shared_nullptr<T>` null.
- Dereferencing yields only a shared/const reference. Fields cannot be assigned
  and mutating methods cannot be called through the handle.
- The source API does not expose `Arc::get_mut`, `Arc::make_mut`, or an
  interior-mutability escape hatch through these pointer types.
- A shared pointee must normally be deeply share-immutable. The compiler-known
  `mutex<T>` and `condition` types described below are the explicit
  synchronization exceptions. Other Rust types containing `UnsafeCell`-based
  interior mutability, including native `Mutex`, `RwLock`, and atomic cells,
  cannot be used directly as `T`; synchronization of the pointer binding
  itself uses `atomic_ptr` or `atomic_nullptr`. The isolated
  `frozen_adapter` foreign-newtype contract described under Rust interop is the
  planned escape hatch for an opaque native representation.
- Reassigning a mutable handle changes only that handle, not the allocation
  observed by other handles.
- Copy construction, assignment, and pass-by-value implicitly clone the
  underlying `Arc`; copying a null `shared_nullptr<T>` remains null.
- `move(pointer)` transfers the handle without incrementing its reference count.
- A `shared_ptr<T>` implicitly converts to `weak_ptr<T>` during initialization,
  assignment, argument passing, or return. `shared.__downgrade()` is the
  equivalent explicit spelling; neither operation changes the strong count.
- `observer.lock()` returns a `shared_nullptr<T>` promoted from a
  `weak_ptr<T>`.

#### Nullable owner conversions and refinement

Conversions between unique-owner representations are moving conversions:

```cpp
unique_ptr<Config> definite =
    make_unique<Config>(/* ... */);
unique_nullptr<Config> maybe =
    unique_nullptr<Config>(move(definite));

if (maybe) {
    unique_ptr<Config> recovered =
        unique_ptr<Config>(move(maybe));
}
```

The first conversion consumes `unique_ptr<T>` and wraps its `Box<T>` in
`Some`; the reverse consumes a proven-non-null `unique_nullptr<T>`. Omitting
`move` or extracting from a source not proven non-null is a compile error.

Shared conversions are explicit constructor-style conversions:

```cpp
shared_ptr<Config> definite = make_shared<Config>(/* ... */);
shared_nullptr<Config> maybe = shared_nullptr<Config>(definite);

if (maybe) {
    shared_ptr<Config> copied = shared_ptr<Config>(maybe);
    shared_ptr<Config> moved = shared_ptr<Config>(move(maybe));
}
```

Converting a named `shared_ptr<T>` to `shared_nullptr<T>` copies its `Arc`
unless `move` is written. Converting back requires a non-null proof; the copy
form clones the contained `Arc`, while the move form extracts it without a
reference-count increment. These conversions never participate in overload
selection.

The compiler tracks nullable pointer bindings as definitely null, definitely
non-null, or unknown. Construction, assignment, `move`, `if (pointer)`,
`!pointer`, and comparisons with `nullptr` update or refine that fact.
Member access and conversion to a pointee reference require a non-null fact:

```cpp
shared_nullptr<Config> config;

if (!config) {
    config = shared_nullptr<Config>(
        make_shared<Config>(/* ... */));
}

i32 version = config.version;
```

At control-flow merges a fact survives only if it agrees on every incoming
path. After a successful guard, code generation introduces a hidden borrowed
`&T` and routes pointee access through it. The compiler must not clone or move
the contained `Arc` merely to represent that proof.

#### Member access and owning-pointer operations

Stainless does not have C++'s `->` member-access operator. The `.` operator
automatically dereferences references and Stainless ownership types:

```cpp
unique_ptr<Config> unique = make_unique<Config>(/* ... */);
unique.version = 2;

shared_ptr<Config> shared = make_shared<Config>(/* ... */);
i32 version = shared.version;
```

Automatic receiver dereferencing is a member-access rule, not a general
implicit conversion used during overload selection. A mutable unique owner may
provide mutable member access when borrowing permits it; a shared owner provides
only const access. A nullable owner must first be proven non-null.

The pointer wrappers do not expose the methods of `Box`, `Arc`, `Option`, or
`Weak`. Their operations are the compiler-defined `move` and `drop` functions,
`shared.__downgrade()`, `observer.lock()`, the documented constructors, and
atomic slot operations. There is no `get`, `release`, `reset`, pointer
arithmetic, or raw-pointer escape.

#### Thread safety

Atomic reference counting makes the ownership control block thread-safe; it
does not make concurrent mutation of one pointer binding safe. Stainless
therefore applies all of the following rules:

- A `unique_ptr<T>` or `unique_nullptr<T>` may move to another thread when
  `T` is `Send`. The source binding is invalidated, and the unique handle is
  never shared between threads.
- A `shared_ptr<T>` or `shared_nullptr<T>` may cross a thread boundary only
  when `T` is both `Send` and `Sync`. Atomic reference counting alone is
  insufficient.
- These properties are structural for generated types: a struct or class is
  sendable and shareable only when all of its fields are. Stainless code cannot
  provide an unchecked manual implementation of either property.
- A non-null shared interface pointer lowers to
  `Arc<dyn Interface + Send + Sync>`; its nullable counterpart lowers to
  `Option<Arc<dyn Interface + Send + Sync>>`. Every concrete class stored in a
  handle must satisfy both bounds.
- Passing a shared handle to a thread copies it by value unless the caller
  explicitly uses `move(pointer)`. Separate handles may then be copied and
  dropped concurrently because the underlying reference count is atomic.
- A thread may reassign its own local handle. The same mutable pointer binding
  cannot be concurrently accessed or reassigned by multiple threads.
- Namespace-scope pointer storage is subject to the compile-time-initialization
  rule below. In the initial implementation this permits a null
  `shared_nullptr<T>` or `atomic_nullptr<T>`, but not an allocated
  `shared_ptr<T>` or non-null `atomic_ptr<T>`.

Globally replaceable handles use compiler-known synchronized slots:

- `atomic_ptr<T>` lowers initially to `RwLock<Arc<T>>`. `__load()` returns a
  copied `shared_ptr<T>`, `__store(value)` replaces it, and `__swap(value)`
  returns the previous handle. It is non-null and has no default constructor.
- `atomic_nullptr<T>` lowers initially to
  `RwLock<Option<Arc<T>>>`. Its corresponding operations use
  `shared_nullptr<T>` and preserve null. Its default constructor creates a null
  slot and is const-evaluable.

Both atomic slot types are non-copyable; copying a lock would create an
independent slot. Existing loaded snapshots continue to refer to the old
allocation after a store or swap. The `__` operation names are reserved
compiler API, and a more specialized lowering may replace `RwLock` later
without changing Stainless semantics.

#### Mutexes, reader/writer locks, and conditions

Stainless calls the C++ `std::condition_variable` concept simply `condition`.
It is a compiler-known synchronization type, not a native Rust name that must
be imported. The initial synchronization vocabulary maps directly to safe Rust:

| Stainless | Rust lowering | Semantics |
| --- | --- | --- |
| `mutex<T>` | `std::sync::Mutex<T>` | Exclusive mutable access to one `T` |
| inferred lock guard | `std::sync::MutexGuard<'_, T>` | Scoped, move-only lock ownership |
| `shared_mutex<T>` | `std::sync::RwLock<T>` | Concurrent readers or one exclusive writer |
| inferred read guard | `std::sync::RwLockReadGuard<'_, T>` | Scoped, shared access to one `T` |
| inferred write guard | `std::sync::RwLockWriteGuard<'_, T>` | Scoped, mutable access to one `T` |
| `condition` | `std::sync::Condvar` | Wait and one/all waiter notification |

Construction and the core API use C++-like syntax with Rust-like method names:

```cpp
mutex<State> local = mutex<State>(State{false, 0});
condition changed;

auto guard = local.lock();
guard.ready = true;
changed.notify_one();

shared_mutex<State> indexed = shared_mutex<State>(State{true, 7});
auto view = indexed.read();
i32 value = view.value;
```

`lock()` produces an internal guard type that source code cannot spell. The
guard must be held in mutable `auto`, cannot be copied, explicitly moved,
returned, used as a field, or passed as an ordinary function argument, and
releases the mutex when its scope ends. It automatically dereferences to the
protected `T`, so its fields and methods are accessed directly with `.`, just
like Stainless pointers and references. A mutable guard binding provides
mutable access, while a const guard binding provides only const access.

`condition.wait(guard)` is the sole special guard transfer. It atomically
releases the mutex while waiting, reacquires it before returning, and
transparently rebinds the same named guard. The argument must therefore be a
mutable named guard. Conditions may wake spuriously, so the state test must be
in a loop:

```cpp
for (; !guard.ready;) {
    changed.wait(guard);
}
```

`notify_one()` wakes one waiter and `notify_all()` wakes every waiter. Every
wait performed on a particular `condition` instance must use a guard from the
same mutex instance; this is also an invariant of the Rust `Condvar` being
lowered to. The current compiler verifies the guard type and lifetime but does
not yet prove that instance identity, so violating this rule can produce the
underlying Rust panic.

`shared_mutex<T>.read()` allows any number of concurrent readers and exposes
only const access to `T`. `shared_mutex<T>.write()` excludes both readers and other
writers and exposes mutable access. Both guard types are inferred and obey the
same lexical, non-copyable, non-returnable restrictions as a mutex guard.
Condition waits accept only mutex guards; Rust condition variables do not wait
on reader/writer locks. Generated code recovers poisoned Rust locks by taking
their inner value, consistently with `mutex<T>`.

`mutex<T>`, `shared_mutex<T>`, and `condition` are non-copyable. Because Stainless
structs are implicitly copyable data, none may be stored directly in a struct
in the current language subset. Shared synchronized state instead uses
`shared_ptr<mutex<T>>` / `shared_ptr<shared_mutex<T>>`, and a condition can likewise
use `shared_ptr<condition>`:

```cpp
shared_ptr<mutex<State>> state =
    make_shared<mutex<State>>{false, 0};
shared_ptr<condition> changed = make_shared<condition>();
shared_ptr<shared_mutex<State>> index =
    make_shared<shared_mutex<State>>{false, 0};
```

These synchronized pointee types are deliberate exceptions to the otherwise
immutable `shared_ptr<T>` rule. A shared handle still cannot mutate the
protected `T` directly; mutation is possible only through a live lock guard.
Rust's type system remains the final thread-boundary check: `Arc<Mutex<T>>` is
shareable when `T: Send`, `Arc<RwLock<T>>` when `T: Send + Sync`, and
`Arc<Condvar>` is shareable. Lock poisoning caused by a Rust panic is recovered
with `PoisonError::into_inner()` by generated code, rather than adding an
undeclared Stainless checked exception.

Thread entry uses Rust's `std::thread::spawn`, reached in Stainless through
`rust::std::thread::spawn` or an import such as
`use rust::std::thread;`. The explicit C++ lambda syntax shown in the reference
examples lowers to a Rust closure with explicit capture initialization.

An unscoped spawn requires an owned `FnOnce` callback. Every capture and return
value must be `Send`, and all captures must be `'static`: a borrowed capture is
rejected, a unique value uses an initializer capture with `move`, and a
`shared_ptr<T>` copy capture clones its `Arc` handle. Dropping an unjoined
handle detaches the Rust thread. `join()` consumes the move-only handle and
throws checked `stainless::ThreadError` if the worker panicked:

```cpp
auto worker = thread::spawn([state]() {
    run_worker(state);
});
worker.join(); // must be caught or declared in throws
```

With `auto`, the initial `spawn` form contextualizes the callback as
`void()`. A result-producing callback uses an explicit handle target so its
return type remains deterministic:

```cpp
thread::JoinHandle<i32> worker = thread::spawn([]() {
    return 42;
});
i32 result = worker.join();
```

Rust's lexical `thread::scope` permits ordinary borrowed values, including
plain `mutex<T>` and `condition`, without `shared_ptr`. The outer scope callback
borrows them normally. Its `Scope.spawn` callbacks may copy-capture those
shared references because their lifetimes cannot escape the scope:

```cpp
const mutex<State> state = mutex<State>(State{false, 0});
const condition changed;

thread::scope([&state, &changed](const thread::Scope& scope) {
    scope.spawn([state, changed]() {
        auto guard = state.lock();
        guard.ready = true;
        changed.notify_one();
    });
});
```

All scoped workers are joined before `thread::scope` returns. A scoped handle
may be joined explicitly, but it cannot be returned, stored in a struct, or
otherwise escape its lexical callback. Any worker panic not already joined is
caught at the scope boundary and converted to `stainless::ThreadError`.
Thread and scoped-thread bodies are currently non-throwing Stainless callbacks;
they may catch checked exceptions internally. Generated Rust retains the
corresponding `Send`, `'static`, and lifetime constraints so `rustc` checks the
compiler's decision again. External binding manifests may likewise use
`escape = "thread"`; their generated callback bounds include `Send + 'static`.

#### Namespace-scope variables

Stainless follows C++ syntax: every variable declared at namespace scope has
static storage duration. There is no separate `global` keyword. A namespace
qualifier controls where the name is found, while `static` at namespace scope
controls linkage/visibility rather than whether the variable is global.

Namespace-scope declarations must use one of the following safe forms:

- `const T value = ...;` declares an immutable global when its initializer is
  const-evaluable. It may be accessed from multiple threads only when `T` is
  `Sync`.
- A null `const shared_nullptr<T>` is allowed because `None` requires no
  allocation. A non-null `shared_ptr<T>` or `shared_nullptr<T>` initializer is
  rejected because constructing its `Arc` requires runtime allocation.
- An `atomic_ptr<T>` or `atomic_nullptr<T>` may be changed through its
  `__load`, `__store`, and `__swap` operations, but its initial state must
  itself be const-evaluable. Initially that admits a null `atomic_nullptr<T>`;
  a non-null `atomic_ptr<T>` must be local because it needs an allocated owner.
- `thread_local T value = ...;` gives each thread an independent instance and
  likewise requires a const-evaluable initializer.

An ordinary mutable namespace-scope variable is rejected because it would
require Rust `static mut`-like unsynchronized access. It must instead be made
immutable, thread-local, or represented by an explicitly synchronized type.
Threads may refer to safe namespace-scope variables directly; the values do not
need to be passed as thread arguments.

Every namespace-scope initializer must lower directly to a Rust const/static
initializer. The accepted subset consists of literals, primitive const
arithmetic, static struct constants, null nullable owners, and aggregate construction whose
data base and fields are recursively const-evaluable. Ordinary Stainless
function calls, user constructor bodies, heap allocation, I/O, native calls not
verified as `const`, and any expression with a checked exception effect are
rejected. There is no `OnceLock`, lazy initialization, source-order dynamic
initialization, or global initialization failure in the initial language.

The Stainless type checker must enforce the thread-boundary rules itself so it
can issue source-level diagnostics. Generated Rust retains the corresponding
`Send`/`Sync` bounds, allowing `rustc` to verify them again. Combined with the
immutable shared-pointee rule, the synchronized slot types, and the absence of
unsafe/raw-pointer escape hatches, this provides the pointer layer's data-race
guarantee.

For classes, `unique_ptr<Class>`, `unique_nullptr<Class>`,
`shared_ptr<Class>`, and `shared_nullptr<Class>` may coerce to the
corresponding owner of an implemented interface. These lower to
`Box<dyn Interface>`, `Option<Box<dyn Interface>>`, `Arc<dyn Interface>`, or
`Option<Arc<dyn Interface>>`; null remains null. Like data-base reference
coercion, they apply only after a target type or function has been selected and
do not make an overload candidate match.

A `shared_ptr<Derived>` or `shared_nullptr<Derived>` may also convert to the
corresponding owner of any public class base. The conversion retains the
compiler-owned base subobject and may traverse the complete single-inheritance
chain. Derived-to-base class references use the same projected chain. These
specific class-base conversions may make an otherwise unique function or
method candidate viable after C++ name hiding has selected the declaring
overload set; exact candidates still win. Unique-owner class-base conversion is
deferred because it needs a separate consuming representation rule.

For example:

```cpp
shared_ptr<Config> current = make_shared<Config>(/* ... */);
shared_ptr<Config> observer = current;

current = make_shared<Config>(/* replacement */);
```

The assignment changes `current` to refer to a new immutable `Config`.
`observer` continues to refer to the original value until its own handle is
dropped or reassigned.

### Rust APIs and Cargo crate interoperability

Except for the ownership layer, Stainless follows Rust's standard vocabulary
instead of recreating either the C++ or Rust standard library. The Stainless
prelude includes the compiler-defined ownership types `unique_ptr`,
`unique_nullptr`, `shared_ptr`, `shared_nullptr`, and `weak_ptr`. Native Rust
types are not added to that prelude; they are selected through the reserved
`rust` namespace and may then be imported under their real short names:

```cpp
use rust::{Option, Result, String, Vec};
use rust::std::collections::HashMap;
```

Safe, representable APIs from `core`, `alloc`, and `std` keep their Rust module
paths below the virtual root, method names, signatures, and trait constraints.
Thus Rust `std::collections::HashMap` is
`rust::std::collections::HashMap` before import. Construction keeps C++ syntax:
an associated Rust `Type::new(arguments...) -> Self` is written
`Type(arguments...)`, `From<U>` may provide an exact one-argument constructor,
and a default construction may be written simply as `Type value;`. After
`use rust::Vec;`, examples therefore include `Vec()`, `.push`, and `.len`.
After `use rust::String;`, they include `String()` and `.push_str`. Explicit
generic construction needs no contextual target, so
`Map<u32, u64>()` and `Map<tuple<Vec<u8>, u32>, IndexEntry>()` are valid
expressions. No general Stainless facade renames `.len()` to `.size()`. The
deliberate exception is ownership: source code uses the restricted Stainless
pointer operations specified above instead of naming `Box`, `Arc`, or `Weak`
directly.
`rustc` remains the final check that every emitted standard-library call is
valid.

The compiler requires semantic signatures before it can produce useful
Stainless diagnostics. Each compiler release will therefore carry exact
metadata for the supported Rust toolchain's representable `core`, `alloc`, and
`std` APIs. The first checked-in subset is deliberately hand-authored and
validated; it can become machine-generated when the metadata format and
generator are mature. This is type/trait/borrow metadata, not wrapper
implementations or a second API. Its paths and names remain Rust's except for
the explicit ownership mapping and the
`List`/`Queue`/`Map`/`MultiMap`/`Set` collection aliases, its version is tied to
the supported Rust toolchain, and generated calls are revalidated by that
toolchain.

#### Implemented `Vec`, `String`, and collection subset

The compiler crate currently registers the following source-visible APIs:

- `rust::Vec<T>`: `Vec()`, `Vec::with_capacity`, `len`, `is_empty`,
  `capacity`, `reserve`, `reserve_exact`, `shrink_to`, `shrink_to_fit`,
  `push`, `pop`, `clear`, `truncate`, `insert`, `remove`, `swap_remove`,
  `append`, `extend_from_slice`, `copy_range`, `reverse`, `clone`, `contains`,
  `sort`, `dedup`, and the non-escaping `with_range(begin, end, callback)`
  slice adapter. `copy_range(begin, end)` clones a checked half-open range into
  a new owned `Vec<T>` and requires `T: Clone`. `with_range` visits the range
  without allocating or exposing a storable Rust slice and returns `false` for
  invalid bounds. `values[index]` provides shared or mutable indexed-place
  access, accepts `u8`, `u16`, `u32`, `u64`, or `usize`, and uses checked
  conversion to `usize` followed by Rust's bounds-checked indexing behavior.
- `rust::String`: `String()`, the explicit copy constructor
  `String(const String&)`, `String::with_capacity`, `clone`, `into_bytes`,
  `len`, `is_empty`, `capacity`, `reserve`, `reserve_exact`, `shrink_to`,
  `shrink_to_fit`, `truncate`, `clear`, `push`, `push_str`, `pop`, `insert`,
  `insert_str`, `remove`, `make_ascii_lowercase`, `make_ascii_uppercase`,
  `is_ascii`, `contains`, `starts_with`, `ends_with`,
  `eq_ignore_ascii_case`, `replace`, `repeat`, `to_lowercase`, and
  `to_uppercase`.
- `rust::List<T>`: a doubly linked list backed by Rust `LinkedList<T>`, with
  `List()`, `len`, `is_empty`, `clear`, `push_front`, `push_back`, `pop_front`,
  `pop_back`, `append`, `contains`, and `clone`.
- `rust::Queue<T>`: a double-ended queue backed by Rust `VecDeque<T>`, with
  `Queue()`, `Queue::with_capacity`, capacity/reservation methods, `len`,
  `is_empty`, `clear`, `truncate`, front/back push and pop, indexed `insert`
  and `remove`, front/back swap removal, `append`, rotation, `contains`, and
  `clone`.
- `rust::Map<K, V>`: an ordered map backed by Rust `BTreeMap<K, V>`, with
  `Map()`, `len`, `is_empty`, `clear`, `insert`, `remove`, `contains_key`,
  `with`, `with_mut`, `with_range`, `with_first_in_range`, `with_first_after`,
  `with_last_in_range`, `with_last_before`, `retain`, `append`, `clone`, and
  key/value structured-binding range iteration. `with(key, callback)` and
  `with_mut(key, callback)` perform an O(log n) lookup and confine the borrowed
  value to one non-escaping callback, avoiding a storable `Option<&V>`.
  `with_first_in_range(lower, upper, callback)` and
  `with_last_in_range(lower, upper, callback)` similarly borrow the least or
  greatest entry in an inclusive range without exposing a Rust iterator.
  `with_range(lower, upper, callback)` visits the inclusive interval in
  ascending order and returns its entry count. `with_last_before(lower, upper,
  callback)` selects the greatest entry in the half-open `[lower, upper)`
  interval; `with_first_after(lower, upper, callback)` symmetrically selects
  the least entry in `(lower, upper]`. `retain` has exact predicate overloads for
  `(const K&, const V&)` and for `const K&` alone; both still scan the complete
  map and are not used by kvstore revert.
- `rust::MultiMap<K, V>`: an ordered multimap backed by the compact runtime's
  B-tree with private `List<V>` buckets. `MultiMap()`, `insert`, `len`,
  `key_len`, `is_empty`, `clear`, `contains_key`, `remove`, `remove_all`,
  `with`, `with_mut`, `retain`, `clone`, and flat key/value range iteration are
  available. `remove(key, value)` removes the first matching association.
  `len()` counts associations, while `key_len()` counts distinct keys.
  `with(key, callback)` invokes the callback once for every matching value and
  returns the number of matches; no nested collection escapes.
- `rust::Set<T>`: an ordered set backed by Rust `BTreeSet<T>`, with `Set()`,
  `len`, `is_empty`, `clear`, `insert`, `replace`, `remove`, `take`,
  `contains`, `append`, and `clone`.

The metadata records `&self`, `&mut self`, and consuming `self` separately.
Generic `Vec` methods also retain their Rust trait requirements: `clone`
requires `T: Clone`, `contains` and `dedup` require `T: PartialEq`, and `sort`
requires `T: Ord`. Stainless `const String&` arguments to Rust string-slice
parameters are resolved as exact Stainless parameters first and adapted to
Rust `&str` only during lowering.

Ordered map keys and set elements retain Rust's `Ord` requirement. Collection
`clone` methods retain the corresponding element/key/value `Clone` bounds, and
list/queue membership retains `PartialEq`. These are compile-checked again by
the selected Rust toolchain.

The binding model supports direct reference returns and records whether their
borrow originates from the receiver or the callable's one reference parameter.
Map lookup deliberately uses non-escaping callbacks because the language does
not yet admit reference-bearing values such as `Option<const V&>`.
Reference-bearing values and iterator-producing APIs are not exposed in this
first subset: `Vec::get` returns `Option<&T>`, `Vec::iter` returns a borrowing
iterator, and `String::as_str`, `String::split`, and `String::chars` also require
either `str` or a borrowing iterator. Direct `Vec` indexing is compiler-known
syntax rather than an exposed reference-bearing Rust return value. Adding the
remaining APIs requires the deferred reference-bearing-value model rather than
merely placing their names in the registry. The semantic resolver now
instantiates this metadata for constructors, associated functions, and methods,
including receiver mode, argument adaptations, generic substitutions, and
retained trait obligations. The Rust emitter uses this metadata for direct
standard-library calls.

#### Native JSON and `var`

`var` is a compiler-native JSON value type and needs no `use` declaration.
It can contain only JSON `null`, a boolean, a JSON number, a string, an array,
or an object. `null` is reserved language syntax. Arrays and objects use
JavaScript-like literal syntax; object keys may be identifiers or string
literals:

```cpp
var response = {
    status: 200,
    "content-type": "application/json",
    values: [1, 2, null, {}],
};
```

The `stainless-runtime` representation stores arrays and objects behind
thread-safe Rust `Arc<RwLock<...>>` handles. Copying, assigning, passing, or
returning a `var` implicitly clones the handle, so aggregate copies are cheap,
preserve identity, and observe the same mutations. Scalar alternatives remain
ordinary values. Equality compares scalars by value and arrays/objects by
shared identity, matching JavaScript's object-reference behavior. Runtime read
and write locks make access through independently owned aliases data-race-free,
including when those aliases are transferred to different threads.

Dot access and array indexing return an owned `var` handle. A missing member,
an out-of-bounds index, or access on the wrong JSON kind returns `null`:

```cpp
var name = response.user.name;
var first = response.values[0];
var absent = response.values[999];
if (absent.is_null()) {
    // handle absence
}
```

JSON array index syntax accepts the same `u8` through `u64` and `usize` index
types as fixed arrays and vectors. Conversion to Rust `usize` is checked.

An object member or array element may be assigned through a mutable `var`
place. Indexed assignment extends an array with `null` values when necessary,
matching JavaScript's sparse-index behavior:

```cpp
void update(var& response) throws stainless::JsonError {
    response.status = 201;
    response.values[0usize] = "updated";
    response.values[5usize] = true;
}
```

A `const var&` cannot be used for mutation, even though another mutable alias
may update the same aggregate. The initial method surface is:

- objects: `set(const String&, var)`, `remove(const String&)`, and
  `contains_key(const String&)`;
- arrays: `push(var)`, `pop()`, `insert(usize, var)`, and `remove(usize)`;
- arrays or objects: `len()`, `is_empty()`, and `clear()`.

`pop()` and object `remove()` return `null` when no value exists. Array
`remove()` rejects an out-of-bounds index, while `insert()` accepts indices up
to and including the current length. All of these methods retain Rust naming.
JSON-compatible scalar arguments convert to `var` only after a unique method
has been selected by name and arity; they never participate in overload
resolution.

Mutation is checked: using the wrong aggregate kind, passing an invalid array
index, or inserting a value that would create a reference cycle raises
`stainless::JsonError`. Callers must catch or declare it. Rejecting cycles is
necessary both to prevent `Arc` cycles from leaking and to preserve the
guarantee that every `var` remains serializable by non-throwing `to_json()`.

JSON-compatible statically typed values convert implicitly when a destination
is `var` or when they occur inside a JSON literal. Conversion in the other
direction is explicit constructor syntax. `bool(value)`, every Stainless
integer/floating type, and `String(value)` use JavaScript-compatible scalar
coercion; integer coercion is extended deterministically to all Stainless
integer widths. Non-finite floating values are represented as JSON `null`
because JSON has no NaN or infinity.

Data structs have automatic structural conversion through `var(value)` and
the same implicit `var` destinations:

```cpp
struct Position {
    i32 x;
    i32 y;
};

var point = var(Position{3, 4});

Position named = Position{5, 6};
var copied = named; // `named` remains usable under struct copy semantics
```

Every converted struct object contains a `__type` string whose value is the
fully qualified Stainless type path, such as `Position` or
`geometry::Position`. This member is emitted first by the conversion (JSON
serialization may still apply its deterministic object-key ordering). A
derived struct uses its most-derived type; flattened base subobjects do not add
a second `__type`. Nested structs are separate objects and therefore carry
their own types.

All declared data fields participate, including private fields; `private`
controls Stainless source access, while explicitly requesting structural JSON
conversion serializes the complete data value. The `__` source prefix is
reserved, so a declared field cannot collide with the compiler-provided
`__type` member. Data-base fields are flattened from base to derived. Reusing
an inherited field name is therefore rejected for JSON conversion rather than
silently replacing one value. Nested data structs become nested objects.
`Vec`, `List`, `Queue`, and `Set` become arrays; `Map<String, V>` becomes an
object; and `Option<T>` becomes either its converted value or `null`, provided
the contained type is also convertible. Classes, ownership pointers, callbacks,
mutexes, and native types without a declared JSON representation are rejected
during Stainless analysis.

The compiler emits conversion code only for struct types that actually reach a
JSON conversion. It does not require or generate `serde` derives, and it does
not expose arbitrary Rust functions to Stainless. A named struct is cloned
before the generated conversion unless the source explicitly writes
`move(value)`; a temporary such as `var(Position{3, 4})` is consumed directly.

Parsing and serialization use the following initial API:

```cpp
use rust::String;

var parse_body(const String& source) throws stainless::JsonError {
    var value = var::parse(source);
    return value;
}

var parse_path(const String& path) throws stainless::JsonError {
    var value = var::parse_file(path);
    return value;
}

String encode(const var& value) {
    return value.to_json();
}
```

`var::parse` and `var::parse_file` return native Rust results internally. The
normal target-typed result adaptation converts their errors to the dedicated
checked `stainless::JsonError`, preserving the error text in the inherited
`message` field. Compiler-described fallible `var` methods receive the same
automatic checked conversion. `to_json()` is infallible because `var` cannot
contain a non-JSON value or a reference cycle; it returns compact,
deterministic JSON directly. `parse_file` opens the file and parses it through
a buffered reader. The runtime uses `serde_json` only at the parse/serialize
boundary; its own `Var` shape supplies Stainless's shared, synchronized
aggregate semantics.

### File I/O

The initial filesystem API follows Rust's `std::fs` names through the reserved
Rust namespace:

```cpp
use rust::{String, Vec};
use rust::std::fs;

String load(const String& path) throws stainless::IoError {
    return fs::read_to_string(path);
}

void save_text(
    const String& path,
    const String& contents
) throws stainless::IoError {
    fs::write(path, contents);
}

void save_bytes(
    const String& path,
    const Vec<u8>& contents
) throws stainless::IoError {
    fs::write(path, contents);
}
```

`read_to_string(path)` returns `String`, rejecting invalid UTF-8 through
`stainless::IoError`; `read(path)` returns `Vec<u8>`. The two exact `write`
overloads accept `const String&` and `const Vec<u8>&`. Like Rust's
`std::fs::write`, both create a missing file or truncate an existing file.

The same checked API exposes `exists(path) -> bool`, `copy(from, to) -> u64`,
`rename(from, to)`, `remove_file(path)`, `create_dir(path)`,
`create_dir_all(path)`, `remove_dir(path)`, and `remove_dir_all(path)`. Every
operation maps `std::io::Error` to `stainless::IoError`, retaining its display
message, and therefore must be caught or declared in `throws`. Paths are UTF-8
`String` values in this first surface.

`rust::std::fs::File` is a move-only open file handle. `File::open(path)` opens
the path once, and `file.pread(offset, length)` reads at most `length` bytes at
the absolute `u64` byte offset without changing a shared cursor. A read that
reaches end of file returns a shorter `Vec<u8>`. Both operations raise checked
`stainless::IoError` on failure. The `pread()` receiver is shared, and `File`
is `Send + Sync`, so one handle can be stored in shared state and used by
multiple threads concurrently. The runtime maps this to the platform's native
positioned file operation (`read_at` on Unix and `seek_read` on Windows); it
does not reopen the file for each read.

The initial handle API also provides `File::create(path)`,
`file.pwrite(offset, contents)`, `pread_exact()`, `pwrite_all()`, `len()`,
`is_empty()`, `set_len()`, `sync_data()`, `sync_all()`, and explicit
`try_clone()`.
`pread()` and `pwrite()` retain low-level short-I/O semantics;
`pread_exact()` and `pwrite_all()` loop without introducing a cursor and are
the preferred operations for durable record formats. `OpenOptions`
supports Rust's `read`, `write`, `append`, `truncate`, `create`, and
`create_new` flags followed by `open(path)`.

`stainless::BigEndian` and `stainless::LittleEndian` expose `write_u32()`,
`write_u64()`, and checked `read_u8()`, `read_u32()`, and `read_u64()`
operations for binary formats. They lower to Rust's fixed-width byte-conversion
operations rather than source-level arithmetic loops. A read with the wrong
byte count raises `stainless::IoError` with `InvalidData`. The corresponding
`read_u8_at()`, `read_u32_at()`, and `read_u64_at()` forms decode directly at a
checked `usize` offset in an existing byte vector.
The write methods also accept `usize` values such as collection lengths,
allowing `BigEndian::write_u32(output, values.len())` without a redundant cast.
Conversion to `u64` is lossless on every supported Rust target; conversion to
`u32` is checked and raises `stainless::IoError` instead of truncating.

`BigEndian` additionally provides exact-type `write(output, value)` and
`read(bytes, offset, value)` overloads for `u8`, `u16`, `u32`, `u64`, and
`u128`. The latter decodes into `value` and advances the mutable `usize`
offset, allowing compound ordered keys to be encoded and decoded without
temporary slices or repeated casts. Overload resolution uses the exact integer
type; there is intentionally no platform-dependent `usize` overload.

Cursor-based and buffered streams, rich metadata, permissions, and symlink
operations remain for the next file-I/O layer.

### Random bytes

Operating-system entropy is the deliberately small native boundary for
Stainless-written cryptographic code. `stainless::Random::bytes(length)`
returns a `Vec<u8>` filled by the platform random source. It rejects requests
larger than one MiB and converts entropy failures to the checked
`stainless::RustError` exception:

```cpp
use rust::Vec;
use stainless::Random;

Vec<u8> nonce(usize length) throws stainless::RustError
{
    return Random::bytes(length);
}
```

Hashing, key handling, signing, encoding, and protocol policy do not belong in
this runtime facade.

### Versioned key/value store showcase

`crates/stainless-kvstore` is the first substantial library implemented in
Stainless itself. Its `.stl` implementation owns a data WAL at the requested
path and a compact index WAL at `<path>.index`, plus internally synchronized
version/index state. Inserts append the value record and a small index record
at explicitly managed offsets. `commit(next)` syncs the data WAL first, then
appends and syncs an index commit marker carrying the committed data length.
Recovery treats that marker as authoritative and truncates incomplete or
uncommitted tails in both files.
The compact records have kind-specific layouts: insert records contain only
their version, value location, and key; commit records contain only their
version and committed data-WAL length.
All fixed-width WAL integers use big-endian encoding. This includes record
sizes, versions, key/value lengths, and checksums; built-in ordered integer key
codecs use the same byte order so lexicographic bytes preserve numeric order.

The key-ordered index is held in RAM and rebuilt by replaying only the compact
index WAL at open. It is a Stainless
`Map<tuple<Vec<u8>, u32>, IndexEntry>`, lowering to Rust's ordered
`BTreeMap`. The compound key is the sole version metadata: it orders every
logical key by version, and inserting the same key again within one version
replaces that version's location. `find()` uses the map's non-escaping
`with_last_in_range()` callback to select the greatest entry between `(key, 0)`
and `(key, current_version)` in O(log n), then calls `pread_exact()` under the
same shared `shared_mutex<StoreState>` guard. It never copies or scans the index.
`find_range(lower, upper)` walks the inclusive compound-key interval once,
coalesces each logical key's versions, and returns its latest visible values in
ascending key order. `find_range_first(lower, upper, count)` and
`find_range_last(lower, upper, count)` return bounded prefixes and suffixes.
They use ordered successor or predecessor lookups per logical key, returning
the first results ascending and the last results descending without scanning
the rest of the interval. All three keep the same shared guard while reading
value bytes, so they remain consistent with concurrent commits and reverts.
Recovery also builds an in-memory `Map<u32, u64>` version index from each
committed version to its index-WAL marker offset. `revert(version)` uses an
exact marker or the least successor marker to seek directly to the selected
branch boundary, then reads the discarded index-WAL suffix forward. It gathers
those exact `(key, version)` entries, durably truncates the index, and removes
only the gathered entries from the RAM map. It then truncates the data WAL to
the data length recorded at that branch boundary. Revert therefore does not call
`Map::retain()`, scan the complete key-ordered index, or perform reverse WAL
reads. Commits, reverts, and recovery take an exclusive write guard.
Each `IndexEntry` stores a `u32` value length. Encoded values are therefore
limited to `u32::MAX` bytes; larger writes fail with checked
`kvstore::ValueTooLarge` before their length is narrowed or a WAL record is
written. Both WALs use a big-endian `u32` record-size header; file offsets and
committed file lengths remain `u64`.

A Stainless `Database` sits above the table classes. `Table<K, V>` publicly
inherits `RawTable`, and each JSON table publicly continues that single class
chain. A shared typed table therefore converts to `shared_ptr<RawTable>` when
explicitly registered with the coordinator. The database can register multiple
heterogeneous tables and coordinate `commit(version)` and `revert(version)`
across them.
Database commits are serialized but cannot be one atomic filesystem operation
across independent WAL files. After opening every table on startup,
`Database::recover()` selects the minimum durable table version and reverts any
table ahead of it, safely resolving a process stop between table commits.
Multi-table reads that must observe one logical version are externally
synchronized with database commit, revert, and recovery.

Within Stainless, the byte/WAL engine is explicitly named `RawTable`.
Application serialization is layered on top of it in Stainless rather than in
compiler-generated Rust. `Table<K, V>` stores four callbacks: key encode/key
decode and value encode/value decode. Each callback returns a checked
`Encoded` or `Decoded<T>` result, and a failed callback becomes checked
`kvstore::CodecError`.

`JsonTable<K>` supplies the value callbacks automatically: values are `var`
and are persisted as compact JSON. `JsonTable1<K>`, `JsonTable2<K1, K2>`, and
`JsonTable3<K1, K2, K3>` compose one to three ordered key components. A
`KeyCodec<T>` contains an encoder and an offset-aware decoder so compound keys
can be concatenated without allocating nested containers. The initial
`codecs::u8_key()`, `u16_key()`, `u32_key()`, `u64_key()`, and `u128_key()`
implementations use the `BigEndian::write()` / `BigEndian::read()` overloads.
Applications can supply their own order-preserving key codecs or use
`Table<K, V>` when values should have a non-JSON representation.

Every `KeyCodec<T>` also provides `vec()`, producing a `KeyCodec<Vec<T>>` from
the element codec. Vector elements and the vector terminator use an escaped,
self-delimiting byte framing that preserves Rust's lexicographic `Vec<T>`
ordering; a length prefix would incorrectly order vectors by length first.
The operation composes recursively, so `codecs::u32_key().vec().vec()` encodes
`Vec<Vec<u32>>` keys.

The kvstore's typed tables, codecs, and multi-table Database coordinator stay
in Stainless. The crate packages their generated Rust output and does not
maintain a second hand-written Rust storage API.

Each Stainless compiler release supports one stable Rust minor release. The
build helper compares `rustc -Vv` with the metadata version and rejects a
different minor version with an actionable diagnostic; patch releases are
accepted and Cargo still performs final validation. The repository pins that
minor in `rust-toolchain.toml`. Supporting multiple Rust minors in one compiler
is deferred until the metadata generator and compatibility costs are known.

Rust macros do not have ordinary callable signatures. The implemented
purpose-built set is `rust::println!`, `rust::eprintln!`, `rust::format!`,
`rust::write!`, and `rust::writeln!`; importing one from `rust` permits its
short spelling, including the required `!`. Format arguments must be string
literals. Formatting values currently accept Stainless numeric, boolean,
character, `String`, and `var` values, and Cargo validates the Rust format string.
`format!` returns `String`. The initial `write!`/`writeln!` destination is a
mutable `String`; both return `void` in Stainless and automatically convert a
Rust `std::fmt::Error` into checked `stainless::FormatError`, so the exception
must be caught or declared in `throws`. `writeln!(destination)` writes only a
newline. Arbitrary Rust token trees are not passed through. Other standard and
external macros remain rejected until each receives purpose-built parsing and
lowering rules.

Cargo dependencies are declared in the surrounding Rust project's
`Cargo.toml`, or in `[[cargo_dependency]]` entries for standalone Stainless
programs, and native paths begin with `rust::<dependency>::...`; for
example, `use rust::regex::Regex;`. Here `dependency` is the Cargo dependency
key, with `-` normalized to `_` as in Rust crate paths, so renamed dependencies
work as expected. Non-standard crates use generated wrappers because a Rust
public signature can contain features that Stainless cannot directly spell or
validate: inferred lifetimes, associated types, `impl Trait`, higher-ranked
bounds, closure traits, macros, and unsafe contracts.

The package root may contain one versioned `stainless-bindings.toml` manifest.
Its version-1 schema uses structured entries whose type strings are parsed with
the Stainless type grammar:

```toml
schema = 1

[[cargo_dependency]]
name = "regex"
version = "1.12.4"

[[type]]
dependency = "regex"
rust_path = "regex::Regex"
stainless_path = "rust::regex::Regex"
representation = "opaque"

[[type]]
dependency = "regex"
rust_path = "regex::Error"
stainless_path = "rust::regex::Error"
representation = "opaque"
error_format = "display"

[[function]]
dependency = "regex"
rust_path = "regex::Regex::new"
stainless_path = "rust::regex::Regex::new"
parameters = ["const rust::String&"]
return = "rust::Result<rust::regex::Regex, rust::regex::Error>"

[[method]]
receiver_type = "rust::regex::Regex"
rust_name = "is_match"
stainless_name = "is_match"
receiver = "const"
parameters = ["const rust::String&"]
return = "bool"
```

Generated wrappers normally return the Rust call result unchanged. A binding
whose Rust API returns a fixed array can declare `return_conversion = "into"`
and expose a compatible Stainless type such as `rust::Vec<u8>`; the generated
wrapper applies `Into::into` and rustc checks the conversion.

### Stored callables

Stainless has two built-in, non-null stored callable types with exact
signatures:

```cpp
function<i32(i32)> transform = [](i32 value) {
    return value + 1;
};

function_mut<i32()> next = [count = 0]() mutable {
    count += 1;
    return count;
};
```

`function<R(A...)>` is a shared immutable callable handle. It maps to
`Arc<dyn Fn(A...) -> R + 'static>`, is implicitly copied by cloning the `Arc`,
and may be invoked through either a const or mutable binding. The handle is
shared, but it is not automatically thread-transferable: Rust continues to
require `Send` and `Sync` at any native thread boundary.

`function_mut<R(A...)>` maps to
`Box<dyn FnMut(A...) -> R + 'static>`. It is uniquely owned and move-only;
passing or assigning an existing value therefore requires `move(...)` unless
the destination is a mutable reference. Invocation requires a mutable binding
or mutable reference because it may modify captured state.

Both types have no null state and no default constructor. Their parameter and
return types are exact, named Stainless free-function overloads must resolve
uniquely, and throwing targets are rejected. Stored lambdas may use copy or
initializer captures, but may not borrow captures. A `mutable` lambda requires
`function_mut`; an ordinary lambda can initialize either type. Stored callable
reference returns are deferred. A move-only `function_mut` cannot be a field of
an implicitly copyable struct. Nullable or thread-safe callable wrappers, if
needed, will be separate types rather than weakening these guarantees.

### Native callback parameters

Callback parameters use a structured entry because their Rust invocation and
retention contracts are not ordinary Stainless value types:

```toml
[[method]]
receiver_type = "rust::callback_fixture::Processor"
rust_name = "apply"
stainless_name = "apply"
receiver = "mut"
parameters = [
    "i32",
    { callback = {
        kind = "fn_mut",
        parameters = ["i32"],
        return = "i32",
        escape = "call",
    } },
]
return = "i32"
```

`kind` is `fn`, `fn_mut`, `fn_once`, or `fn_ptr`, corresponding to Rust
`Fn`, `FnMut`, `FnOnce`, or `fn(...) -> ...`. The `escape = "call"` contract
guarantees that the Rust target does not retain the callback after returning.
The `escape = "thread"` contract requires `fn_once` and generates
`Send + 'static` Rust bounds after Stainless verifies the owned captures.
General `escape = "static"` storage remains reserved and rejected.

An async Rust callable uses `async = true` on its function or method entry. Its
declared `return` is the future's output type, not a Stainless-visible future
type. An async callback independently uses `async = true` inside its callback
table:

```toml
[[method]]
receiver_type = "rust::callback_fixture::Processor"
rust_name = "inspect_async"
stainless_name = "inspect_async"
receiver = "const"
async = true
parameters = [
    "i32",
    { callback = {
        kind = "fn",
        async = true,
        parameters = ["i32"],
        return = "i32",
        escape = "call",
    } },
]
return = "i32"
```

The matching Stainless syntax keeps the C++ lambda shape while using Rust's
postfix await spelling:

```cpp
async i32 increment_async(i32 value) {
    return value + 1;
}

async i32 process_async(Processor& processor, i32 bias) {
    return processor.inspect_async(3, [bias](i32 value) async {
        return increment_async(value).await + bias;
    }).await;
}
```

`.await` is rejected outside an async function or async lambda, on a synchronous
call, and when an async call is used without `.await`. Initial async callback
metadata supports `kind = "fn"` and `kind = "fn_once"`; `fn_mut`, function
pointers, reference parameters, stored future values, general task spawning,
and async interfaces are deferred. A repeatable async `fn` callback must own
copyable captures because every invocation needs independent owned future
state. A one-shot async `fn_once` callback may move non-copy state. Generated
wrappers express callbacks as `Fn(...) -> Fut`/`FnOnce(...) -> Fut` with a
`Future<Output = R>` bound, so Cargo checks the real Rust API signature.

The callback expression is contextually typed by that exact manifest
signature. It may be a uniquely resolved exact, non-throwing Stainless
free-function overload or a C++-style lambda with typed parameters and an
explicit capture list:

```cpp
i32 total = 0;
processor.apply(4, [&total](i32 value) {
    total += value;
    return total;
});

i32 factor = 3;
processor.apply(2, [multiplier = move(factor)](i32 value) {
    return multiplier * value;
});

i32 initial = 0;
processor.apply(2, [count = initial](i32 value) mutable {
    count += value;
    return count;
});
```

`[]` captures nothing, `[value]` performs a Stainless copy, `[&value]` borrows
an ordinary local for the duration of the native call, and `[name = expression]`
evaluates `expression` once to initialize a new capture named `name`. Its type
is inferred. Named copyable inputs are copied and remain valid; non-copy inputs
must be explicitly moved, which invalidates their source. Temporaries are owned
directly. A reference-producing initializer is rejected. Without `mutable`,
by-value captures are const inside the lambda; reference captures retain the
mutability of their referent. Reference bindings cannot themselves be captured;
capture the owner instead. Outside a stored-function or native callback
context, a lambda has no inferred type. An ordinary Rust callback cannot throw
or return a reference. Callback kinds and escape policy are not overload
discriminators: two native bindings otherwise having the same signature may
not differ only in those contracts.

Generated wrappers use a deterministic generic type parameter and the declared
Rust closure bound; `fn_ptr` uses a Rust function-pointer parameter directly.
The generated lambda shadows each captured binding with an explicit
clone/borrow/lowered initializer and then emits a `move` closure. Mutable
by-value captures become mutable Rust shadow bindings. Rust therefore checks
the claimed `Fn` capability and exact parameter/return types again when Cargo
compiles the wrapper.

Unknown keys, duplicate Stainless signatures, paths outside the named Cargo
dependency, and unversioned manifests are errors. `receiver` is one of
`value`, `const`, or `mut`, corresponding to Rust `self`, `&self`, or
`&mut self`. Parameter passing is expressed by the Stainless value/reference
syntax; wrapper-specific adaptations such as `const rust::String&` to Rust
`&str` are compiler-known. `error_format` may be `display` or `debug`; generated
Rust proves the requested trait before Stainless uses it for a `RustError`
message.

The compiler now parses this manifest rather than hard-coding the regex
metadata. Calls produce deterministic functions in a private
`__stainless_bindings` Rust module. The generated `Regex::new` wrapper accepts
`const rust::String&`, converts it to `&str` inside the wrapper, and preserves
the real `Result<Regex, regex::Error>` return type; `is_match` similarly
preserves its shared receiver and string adaptation. A Cargo integration test
loads the checked-in manifest, compiles and executes the wrappers against the
actual crate, then changes the target to a nonexistent associated item and
verifies that Cargo rejects the stale binding.

The implemented version-1 loader is deliberately smaller than the eventual
schema. It accepts non-generic opaque external types, associated functions, and
inherent methods with concrete signatures composed of primitives, compiler
supported containers, declared opaque types, values, input borrows, and call-
or thread-scoped callback parameters.
`[[function]]` introduces a Stainless associated function on its declared
owner; its Rust target may be either a safe free function or a safe associated
function inside the named dependency.
`rust::Option` and `rust::Result` may be used with explicit supported type
arguments. A `const rust::String&` parameter receives the compiler-known
`&str` adaptation. Free functions, external generic type declarations,
reference returns requiring provenance, frozen adapters, and explicit trait
requirements are not accepted yet. A user-authored safe Rust associated
function or method adapter is the current escape hatch for a Rust signature
that this subset cannot express.

Custom bindings can already be selected through the compiler library API:

```rust
use stainless_compiler::interop::load_package_bindings;
use stainless_compiler::transpile_with_bindings;

let bindings = load_package_bindings(package_root)?;
let result = transpile_with_bindings(stainless_source, &bindings);
```

`load_package_bindings` starts with compiler-owned bindings such as
`rust::String`, `rust::Vec`, `rust::List`, `rust::Queue`, `rust::Map`,
`rust::MultiMap`, and `rust::Set`, then merges an optional
`stainless-bindings.toml` from the supplied package root. A missing file means
there are no package-specific bindings. `load_bindings_manifest` loads only
one manifest file, while `parse_bindings_manifest` accepts manifest text for
tools that already own the source. `analyze_with_bindings` is the corresponding
analysis-only entry point. TOML syntax errors retain byte spans and file-loaded
errors retain their manifest path.

The target stable toolchain flow is:

1. The versioned binding manifest selects each public item and states its
   Stainless-representable signature and ownership effects.
2. The compiler generates a deterministic Rust shim in a private
   `__stainless_bindings` module. It calls the real crate item and may
   specialize generics, introduce a safe newtype, or normalize an otherwise
   unrepresentable return type.
3. Cargo compiles the shim against the selected dependency. A wrong path,
   signature, trait bound, feature assumption, or ownership claim is therefore
   a compile error rather than trusted foreign metadata.
4. Cargo metadata identifies the exact package version, enabled features, and
   target configuration before accepting the generated artifact. This
   dependency-graph validation is the next implementation step.
5. The verified signature becomes available to Stainless name and type
   resolution, and wrapper/rustc diagnostics are source-mapped to the binding
   declaration and call site.

At an interop boundary, Rust `Box<T>`, `Option<Box<T>>`, `Arc<T>`,
`Option<Arc<T>>`, and `Weak<T>` map to the corresponding Stainless pointer
types. Binding metadata records whether an owner is consumed, produced, or
borrowed. A Rust borrow such as `&Arc<T>` may therefore become a
non-consuming wrapper call that accepts `shared_ptr<T>` without making
`shared_ptr<T>&` a source-level type. A signature is rejected when this
adaptation cannot preserve observable ownership behavior.

An opaque native `T` may be stored by value or behind a unique owner. It is
forbidden as the pointee of `shared_ptr`, `shared_nullptr`, `weak_ptr`,
`atomic_ptr`, or `atomic_nullptr` by default because Rust `Sync` does not prove
Stainless's stronger logical-immutability rule. The planned opt-in is a
user-authored safe Rust newtype listed with
`representation = "frozen_adapter"`. This is an explicit semantic promise that
the selected API exposes no observable mutation through shared access; the
generated shim additionally asserts `Send + Sync`. Native values crossing a
thread boundary by value receive an ordinary generated `Send` assertion.
Stainless-defined types continue to derive these properties structurally.

The wrapper should preserve the Rust item and method name whenever no
adaptation requires a distinct name. Its generated Rust symbol is still
deterministically mangled so two specializations cannot collide. Wrappers are
generated only for selected APIs, not for an entire dependency, and generated
files are rebuildable artifacts rather than source to edit by hand.

Stable `cargo metadata` supplies the resolved dependency graph but not complete
Rust item signatures. Rustdoc JSON can provide richer API metadata, but it is
still a nightly, unstable interface, so it may be an optional binding-generator
accelerator rather than the required foundation. A later stable compiler API
could reduce the amount of explicit binding metadata without changing the shim
boundary.

Only safe Rust calls are admitted directly. An external `unsafe fn` or an API
whose safety depends on raw-pointer invariants requires a user-authored safe
Rust adapter outside Stainless; the generated binding then targets that safe
function. This is a language safety boundary, not a sandbox against users
editing their own Rust project.

Rust `Result` remains an ordinary Rust type. `Option` also retains its Rust API
for permitted element types, with the one ownership-layer restriction that its
direct `T` cannot be a Stainless pointer type. Calling the compiler-adapted
`.unwrap()` method is an explicit request to enter Stainless's checked
exception model and has `stainless::RustError` as its checked effect.
The compiler formats the native `E` when possible and otherwise supplies the
generic fallback message; binding metadata does not need to define an error
conversion. Exact target-typed initialization or assignment from `Result<T, E>`
to `T` inserts the same operation implicitly; it is not used for arguments,
returns, overload resolution, or `auto` deduction. Other `Result` operations do
not make this semantic change implicitly.

The compiler resolves calls into explicit HIR forms:

```text
Stainless function(FunctionId)
direct Rust item(RustItemId)
generated external wrapper(BindingId)
compiler intrinsic(IntrinsicId, including UnwrapRustResult)
```

The compact support runtime is limited to genuine Stainless language features,
currently the reference-counted JSON `var` representation and its
parse/mutation/serialization boundary. Checked-exception erasure is still
emitted inline. The runtime is not a general standard-library facade. The
ownership types are a narrow compiler-defined exception and lower to ordinary
safe Rust ownership and synchronization primitives. Operations whose rules the
compiler must understand, such as explicit `move`, guarded `require`, nullable-owner
conversion, atomic pointer slots, data-base projection, and
ownership-preserving interface coercions, remain intrinsics.

This list is a starting policy, not a complete specification. Any feature
proposal should answer all of the following before implementation:

1. What source syntax does it use?
2. What are its type, ownership, and lifetime semantics?
3. What stable Rust is emitted?
4. Can the emitted Rust preserve the behavior without hidden unsafety?
5. What diagnostics are produced when its rules are violated?

## Proposed compiler pipeline

```text
source
  -> tokens (including comments and whitespace)
  -> lossless concrete syntax tree (CST)
  -> compiler-owned AST (syntax-normalized but unresolved)
  -> imported Rust API and generated-wrapper metadata
  -> name, type, and ownership analysis
  -> small Rust-shaped high-level IR (HIR)
  -> Rust syntax tree/tokens
  -> formatted .rs source
  -> Cargo/rustc validation
```

The CST and compiler AST remain separate. The CST retains spelling, comments,
and malformed input needed for diagnostics and future editor tools. The AST
normalizes syntax for semantic passes and may contain explicit recovery nodes.
The later HIR must contain only validated Stainless concepts with defined Rust
semantics. Transpilation must happen from that validated HIR, not directly from
parser nodes.

### Implemented compiler slice

The `stainless-syntax` crate now exposes `lex(source)` and `parse(source)`.
Lexing retains whitespace, line comments, block comments, invalid tokens, and
UTF-8 byte ranges. Parsing produces an immutable Rowan green tree plus ordered,
recoverable diagnostics; converting the root back to text reproduces the input
exactly even when errors are present. `Parse::tree()` exposes zero-copy typed
wrappers over that CST, including precise range and classic `for` header
accessors.

The current recursive-descent grammar handles:

- namespace blocks and losslessly retained `use` declarations;
- function declarations and definitions, qualified names, parameters,
  reference types, type and compile-time `usize` generic arguments, `const`,
  and `throws` clauses;
- struct, class, interface, and explicitly represented scoped enum definitions;
  direct fields; typed `static const` struct members; ordered type/`usize` const
  generic parameters; data and interface base lists; `public:`/`private:`
  labels; member declarations;
  qualified out-of-type definitions; and struct/array aggregate initialization;
- constructor declarations, `= delete`, qualified out-of-type definitions,
  and C++-style data-base/member initializer lists;
- blocks, initialized or default-constructed local declarations, `return`,
  `throw`, `try` with ordered typed or catch-all handlers, `if`/`else`,
  `while`, `break`, `continue`, and empty statements;
- classic `for (init; condition; update)` and range
  `for (binding : expression)` statements;
- names and paths, literals, parenthesized expressions, calls with narrowly
  parsed explicit generic targets such as `make_unique<T>` and `Array<T, N>`,
  member access, indexing, ordinary and explicitly base-qualified struct fields,
  prefix/postfix operators, assignment, precedence-aware binary expressions,
  and typed lambdas with explicit capture lists, arbitrary owned initializer
  captures, C++-positioned `mutable`, and exact
  `function<R(A...)>`/`function_mut<R(A...)>` signature types.

`stainless_compiler::lowering::lower` converts the typed CST into a
compiler-owned AST with source spans and explicit recovery forms.
`stainless_compiler::analyze` combines parsing, lowering, and the currently
implemented structural checks:

- duplicate parameter names;
- missing initializers on `auto` and reference locals;
- forbidden ordinary-local `auto&` (it remains valid as a range binding);
- `break` and `continue` outside a loop;
- value returns from `void` functions and empty returns from non-`void`
  functions;
- duplicate constructor parameter names and constructor bodies placed inside a
  type declaration;
- access labels or constructors in interfaces, redundant `class sealed`, and
  member-function bodies placed inside a type declaration;
- bare rethrow outside a catch, non-const/non-reference typed catches, and a
  catch-all handler followed by another handler.

These are deliberately pre-resolution checks, not full type checking.
Additional macros and the remaining reference samples still need grammar
productions.

The initial `stainless_compiler::resolution` pass now provides:

- namespace-scoped direct, grouped, aliased, and glob import lookup for the
  implemented single-file subset;
- primitive and native type resolution, local/parameter scopes, contextual
  integer literal types, and expression typing for the current operators;
- user-defined struct/class/interface/enum names and layouts, direct and inherited
  field lookup, C++-style public/private access checks, static member lookup,
  exact declaration/definition matching, struct aggregate construction,
  fluent `void` member returns, and cycle/duplicate/reference-field
  diagnostics;
- exact enum-member typing, fixed-width unsigned representation validation,
  implicit same-signed non-narrowing integer conversion, and enum-member switch
  patterns; checked integer/String construction through `stainless::EnumError`;
- single data inheritance for structs; interface inheritance and exact
  implementation-contract validation; move-only, non-assignable class values;
  static interface implementations for structs and classes; dynamic interface
  references and unique/shared class-owner erasure, including nullable owners;
- exact user-constructor overload selection, deleted/missing-definition
  diagnostics, ordered base/member initialization, and synthesized or
  implicitly deleted struct default constructors, including checked
  constructor effects;
- exact canonical-type overload selection with value/reference-only conflict
  diagnostics and deterministic versioned Rust names; the documented
  derived-struct-to-base-reference projection is the sole struct conversion;
- classification of Stainless calls, compiler intrinsics such as `move` and
  primitive casts, and registered native Rust calls;
- all seven compiler-defined ownership pointer types; parenthesized and braced
  `make_unique<T>`/`make_shared<T>` allocation; nullable refinement and checked
  non-null recovery;
  immutable shared pointee access; `shared.__downgrade()`/`observer.lock()`;
  synchronized atomic
  `__load`/`__store`/`__swap`; and diagnostics for invalid copying, default
  construction, pointer-reference declarations, and move-only struct storage;
- compiler-known `mutex<T>`/`condition` synchronization, inferred scoped lock
  guards, owned and lexical Rust thread spawning, move-only join handles,
  structural `Send`/`Sync` capture checks, and checked `ThreadError` joins;
- exception-struct hierarchy validation, normalized checked `throws` sets,
  mandatory catch-or-declare checking, ordered base/derived handlers, and
  propagation through calls and constructor initialization;
- consuming native `rust::Result<T, E>.unwrap()` resolution plus exact
  target-typed Result-to-success adaptation for initialization and assignment,
  both with checked `stainless::RustError` effects;
- concrete `rust::Vec<T>`/`rust::String` and standard collection constructor,
  associated-function, and method resolution, including generic substitution,
  receiver mutability, consuming-receiver checks, Rust argument adaptations,
  trait requirements, and default construction;
- retained Rust representations and generated-wrapper resolution for
  user-manifest-defined opaque external types, exercised with
  `rust::regex::Regex` and `rust::regex::Error`;
- contextual native callbacks with exact signatures, non-throwing named
  function targets, inferred initializer-capture types, mutable by-value
  captures, `Fn`/`FnMut`/`FnOnce`/`fn` invocation contracts, and
  `escape = "thread"` ownership bounds;
- boundary-only async functions and callbacks, direct postfix `.await`, async
  manifest callables, and generated `Future<Output = R>` callback bounds;
- owning stored callables with exact signature checking, shared `function`
  copies, move-only `function_mut`, owned captures, mutable invocation, and
  non-throwing named-function conversion;
- `Vec<T>`, `List<T>`, and `Queue<T>` range-element resolution for shared,
  mutable, copied, and explicitly consumed loops, plus order-preserving
  non-mutable `Set<T>` iteration and key/value structured bindings for
  `Map<K, V>`.

The initial `stainless_compiler::ownership` pass runs after successful
resolution and before HIR construction. For the implemented subset it:

- tracks each non-copy binding as available, moved, or possibly moved after a
  control-flow join;
- recognizes assignment as explicit reinitialization and preserves availability
  when every continuing path restores the binding;
- validates shared and mutable local-reference loans, mutable reborrowing, and
  conflicting owner access;
- ends local loans after their final source use, while retaining outer loans
  across a loop that may repeat;
- treats consuming ranges as definite moves and checks moves inside potentially
  repeated `while`, classic, and range loop bodies;
- treats an explicit native `Result.unwrap()` and an inserted target-typed
  Result conversion as consuming operations and preserves their exceptional
  ownership paths;
- applies explicit callback captures at the call boundary: shorthand copies
  read their source, arbitrary initializers follow normal expression
  copy/move rules, and borrow captures create a temporary loan lasting through
  the native call;
- applies owned lambda captures when constructing a stored callable, treats
  shared `function` handles as implicit copies, and tracks `function_mut` as a
  move-only value;
- tracks unique and atomic pointers as move-only bindings, treats shared and
  weak handles as implicit handle copies, applies ordinary loan checks to
  pointee borrows, and diagnoses use after an explicit owner move;
- verifies that a direct reference return ultimately originates from the
  function's single reference parameter.

Struct values participate in the implemented copy analysis as implicit
memberwise copies. The backend realizes each copy with Rust `Clone`, so structs
containing `String`, `Vec`, or another cloneable struct retain Stainless value
semantics without pretending to implement Rust `Copy`.

`stainless_compiler::transpile` is the first fail-closed backend API, and
`transpile_with_bindings` applies the same pipeline with a caller-selected
binding registry. If the front end reports a diagnostic, or HIR lowering
encounters a construct without defined Rust semantics, it returns no Rust. For
the accepted subset it now:

- lowers resolved namespaces, free functions, structs/classes, associated
  integer constants, interface traits, static trait implementations,
  dynamically dispatched interface calls, member functions, and constructors
  into a public, typed HIR;
- makes reference borrows/dereferences, exact overload targets, primitive
  casts, explicit moves, struct copies, aggregate construction, inherited-field
  paths, base-reference projections, fluent member receivers, constructor
  field initialization, and implicit native/user default construction explicit;
- lowers unique owners to `Box`/`Option<Box>`, shared owners to
  `Arc`/`Option<Arc>`, weak observers to `Weak`, and atomic slots to
  `RwLock<Arc>`/`RwLock<Option<Arc>>`; allocation, nullable recovery, handle
  cloning, weak promotion, and poison-tolerant atomic replacement are explicit
  HIR operations;
- lowers interfaces to Rust traits with supertraits and `Send + Sync` trait
  objects, and erases class owners using safe `Box`/`Arc` unsizing (or an
  explicit nullable-owner map);
- lowers `while` loops directly, lowers classic loops without breaking C++
  `continue`/update ordering, and lowers shared, mutable, copied, and consuming
  native-collection range loops plus checked JSON-array snapshot loops to the
  corresponding Rust iterator form;
- lowers checked functions and constructors to Rust `Result`, throws to a
  boxed compiler-private error carrier, and typed catches to safe base
  projection without `unsafe`;
- lowers native `Result` conversion and fallible `var` mutation to inline
  non-panicking `match` expressions that construct and propagate a checked
  `stainless::RustError`, or the more specific `stainless::IoError` and
  `stainless::JsonError` for filesystem and native JSON operations;
- emits deterministic private wrappers for manifest-selected external
  associated functions and methods, with argument adaptations and generic
  callback trait bounds inside the Cargo-checked boundary;
- emits Stainless async functions as Rust `async fn`, awaits only
  compiler-proven async calls, and lowers repeatable async lambda captures to
  per-invocation owned copies;
- lowers stored `function` to `Arc<dyn Fn + 'static>` and `function_mut` to
  `Box<dyn FnMut + 'static>`, with explicit trait-object coercions and direct
  invocation;
- emits deterministic Rust with `proc-macro2` and `quote`, validates the
  generated token tree by parsing it with `syn`, and formats it with
  `prettyplease`;
- compiles the dependency-free supported reference files as Rust
  libraries, compiles and executes the JSON and external `regex` references
  through Cargo, and executes behavior fixtures covering functions, borrows, loops,
  structs, memberwise copying, data inheritance, `Vec`, `String`, moves,
  checked exceptions, throwing constructors, native `Result` conversion,
  external-wrapper validation, all four initial Rust callback kinds, and
  shared/mutable stored callable behavior, and synchronized reference-counted
  JSON mutation, plus non-null unique allocation, borrowing, member access,
  moves, and throwing pointee construction.

This is still not full semantic validation. Cross-file modules,
namespace-scope storage, `require(...)`, dynamic interface erasure through weak
and atomic pointer forms, ownership through fields, full path-sensitive
loop-exit precision, general borrow
lifetimes, member/native returned-reference provenance, and generalized native
trait satisfaction remain unresolved. General retained async callbacks,
async-`FnMut`, throwing/reference-returning async callbacks, source-level
future values, task executors, and FFI callback forms are also deferred.
Cargo metadata validation and source-mapped wrapper diagnostics are not
implemented.
Struct member functions and constructors currently lower to deterministically
named Rust free functions. Member functions receive an explicit hidden
receiver; constructors create a hidden mutable borrow only after assembling
every field. This preserves static dispatch while overloaded Rust `impl`
emission remains future work. Accepting an AST shape therefore still does not
imply that all ownership or type semantics are valid.

## Cargo integration and compiler packaging

The Rust workspace contains the compiler, tooling, runtime, and native backing
crates:

- `stainless-syntax` owns tokens, the lossless CST, and typed syntax wrappers.
- `stainless-compiler` owns AST/HIR lowering, name/type/ownership analysis,
  Rust metadata and binding handling, source maps, and Rust emission.
- `stainless-runtime` contains only generated-code support that cannot lower
  directly to the standard library, initially reference-counted JSON `Var`.
- `stainless-build` provides the Cargo build-script API.
- `stainlessc` is a thin CLI over `stainless-compiler` for diagnostics,
  fixtures, and standalone generation.
- `stainless-http` provides the native Rust transport behind the Stainless HTTP
  package.

Pure Stainless libraries such as `stainless-kvstore` use
`stainless-package.toml` and remain outside the Cargo workspace. They need no
Rust facade, `Cargo.toml`, `build.rs`, or generated source checked into the
repository.

Keeping semantics, interop, and codegen as modules inside `stainless-compiler`
avoids premature crate boundaries; they may be split after their APIs stabilize.
A procedural macro is not used because whole-file parsing, external manifests,
generated modules, dependency shims, and source-mapped diagnostics fit a build
step better.

`stainless-runtime` owns the `Var`/native JSON representation, native JSON
errors, and exact-signature file facades that cannot map directly to one
portable inherent Rust method; generated checked-exception trait/object support
remains inline until that ABI stabilizes.

The `stainless-build` crate is an optional bridge for the separate case where
hand-written Rust code embeds Stainless functions. Normal standalone Stainless
programs do not use it. Until the crates are published, an embedding package in
this workspace uses a path build dependency:

```toml
[dependencies]
# Required when generated Stainless code uses `var`/native JSON.
stainless-runtime = { path = "../../crates/stainless-runtime" }

[build-dependencies]
stainless-build = { path = "../../crates/stainless-build" }
```

```rust
// build.rs
fn main() {
    stainless_build::Builder::new("src/lib.stl")
        .add_source("test/lib_test.stl")
        .output_name("stainless.rs")
        .export("app::run", "stainless_run")
        .compile()
        .unwrap_or_else(|error| panic!("{error}"));
}
```

The builder resolves each source relative to `CARGO_MANIFEST_DIR`, concatenates
sources added with `add_source()` into one translation unit in call order,
writes the generated file beneath Cargo's `OUT_DIR`, emits
`cargo:rerun-if-changed`, and can re-export an exact non-overloaded free
function under a stable Rust name. This lets a package keep its Stainless tests
in a sibling `test` directory while sharing private implementation details.
The Rust crate includes the requested output at crate root:

```rust
// src/lib.rs
include!(concat!(env!("OUT_DIR"), "/stainless.rs"));
```

Including at crate root preserves Stainless `crate::` paths and permits direct
calls between generated and hand-written Rust modules. A duplicate Rust and
Stainless item name is a normal compile error unless one side is placed in a
module. Generated files are rebuildable artifacts and are never written into
`src`. Module-aware multi-file compilation, overload-signature export
selectors, and runtime ABI versioning are deferred.

Standalone Stainless programs need one or more `.stl` source files and one
root, non-overloaded `i32 main()` function. They do not need a Rust package,
`build.rs`, or `main.rs`:

```sh
stainlessc --check src/lib.stl
stainlessc src/lib.stl -o generated.rs
stainlessc src/lib.stl > generated.rs
stainlessc --build -o hello main.stl
stainlessc --run main.stl
```

Larger programs can use `stainless-package.toml`:

```toml
schema = 1
name = "poker"
sources = ["src/poker.stl", "src/hand.stl"]
main = "src/main.stl"

[dependencies]
stainless-crypto = "../../crates/stainless-crypto"
stainless-http = "../../crates/stainless-http"
```

Build the package and all transitive source dependencies with:

```sh
stainlessc --build --package apps/poker -o poker-dealer
```

Dependency paths are relative to the declaring package. A dependency's
`sources` are compiled before the dependent package, while its `main` is not
included. `[native_dependencies]` maps Cargo dependency names to local Cargo
package paths; registry dependencies continue to live in that package's
`stainless-bindings.toml`.

Multiple source arguments are concatenated into one translation unit in the
order supplied. Repeatable `--package-root DIR` options compose each package's
`DIR/stainless-bindings.toml`. Registry dependencies declared there are added
to the compiler's temporary Cargo build. Local backing crates are supplied as
`--dependency NAME=PATH` and are also linked only in that temporary directory.

`--check` validates without emitting Rust. Without `-o`, generated Rust is
written to stdout. `--build` writes an executable to the required `-o` path.
`--run` generates a private Rust entry point, compiles it in a temporary
directory, runs it with inherited standard streams, and removes the temporary
files. Runtime-free programs still use `rustc` directly. Programs that use
native JSON receive a hidden temporary Cargo manifest linking
`stainless-runtime`; users still do not create `main.rs` or `build.rs`.
Diagnostics go to stderr and use byte spans until richer source rendering is
added.

### Stainless test packages

A Stainless test is an ordinary executable package whose `main` returns zero on success. Run the checked-in test
packages directly with:

```sh
stainlessc --run --package crates/stainless-crypto/test
stainlessc --run --package crates/stainless-http/test
stainlessc --run --package crates/stainless-kvstore/test
stainlessc --run --package apps/mmx-wallet/test
stainlessc --run --package apps/poker/test
```

Each test directory has its own `stainless-package.toml` and declares the package under test as a source dependency.
When working from an uninstalled workspace build, replace `stainlessc` with
`cargo run -q -p stainlessc --`.

## Rust library survey

Research snapshot: 2026-07-31. The first compiler slice pins Logos 0.16.1,
Rowan 0.16.1, `proc-macro2` 1.0.107, `quote` 1.0.47, `syn` 2.0.119, and
`prettyplease` 0.2.37 through the workspace lockfile. Native JSON additionally
pins `serde_json` 1.0.151.

| Area | Candidate | Assessment |
| --- | --- | --- |
| Lexing | [`logos`](https://docs.rs/logos/latest/logos/) | **In use.** Rust-native, derive-based, fast, and provides byte spans. Trivia is tokenized rather than skipped so the CST stays lossless. Context-dependent token combinations are handled by the parser. |
| Parsing | Hand-written recursive descent plus a Pratt expression parser | **Selected.** This gives precise control over C++-like ambiguities, recovery, contextual keywords, and CST events while keeping the grammar and compiler fully in Rust. |
| Parsing alternative | [`chumsky`](https://docs.rs/chumsky/latest/chumsky/) | **Not selected initially.** It remains a possible later replacement if the hand-written parser's recovery or maintenance cost proves poor, but the first compiler will not carry two parser implementations. |
| Lossless CST | [`rowan`](https://docs.rs/rowan/latest/rowan/) | **In use.** It provides immutable green trees, syntax nodes/tokens, text ranges, and typed-AST support while leaving the language-specific node model to us. It is a tree library, not a parser. |
| CST schema/code generation | [`ungrammar`](https://docs.rs/ungrammar/latest/ungrammar/) | **Optional later.** Useful for declaring CST shapes and generating typed wrappers. It explicitly does not generate a parser, so it is not needed for the first slice. |
| Diagnostics | [`miette`](https://docs.rs/miette/latest/miette/) | **Recommended at the public API/CLI boundary.** It supports structured diagnostics, source spans, labels, error codes, rich terminal output, and a narratable renderer. Internal compiler diagnostics should remain our own data types. |
| Rust emission | [`proc-macro2`](https://docs.rs/proc-macro2/latest/proc_macro2/), [`quote`](https://docs.rs/quote/latest/quote/), [`syn`](https://docs.rs/syn/latest/syn/), and [`prettyplease`](https://docs.rs/prettyplease/latest/prettyplease/) | **In use.** HIR is emitted as structured tokens, reparsed with `syn` as a validity boundary, and formatted with `prettyplease` without requiring an installed `rustfmt`. String construction is limited to compiler-controlled identifiers and paths rather than semantic source generation. |
| JSON | [`serde_json`](https://docs.rs/serde_json/latest/serde_json/) | **In use.** Its well-tested parser, reader support, exact JSON `Value` boundary, Serde ecosystem compatibility, portability, and safe API fit the first runtime. `Var` converts at that boundary so Stainless can retain reference-counted array/object identity. |
| JSON performance alternative | [`simd-json`](https://docs.rs/simd-json/latest/simd_json/) | **Defer.** It is a promising optional parser backend after benchmarks justify it, but its mutable-input and borrowed/owned parsing model add complexity that is unnecessary for the initial portable runtime. |
| Incremental compilation | [`salsa`](https://docs.rs/salsa/latest/salsa/) | **Defer.** Potentially useful for an IDE or a mature multi-file compiler, but unnecessary complexity for the first end-to-end transpilation slice. |

### Alternatives not selected for the core compiler

- [LALRPOP](https://lalrpop.github.io/lalrpop/) is a capable LR(1) parser
  generator, but C++-like contextual syntax and compiler-quality recovery are
  likely to require fighting or restructuring its grammar.
- [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) is excellent for
  fast, incremental, error-tolerant editor parsing. A custom grammar is authored
  in JavaScript and generates a C parser, however, and its CST does not replace
  the compiler's typed AST and semantic passes. It may be added later for editor
  support after the Stainless grammar stabilizes.
- Parsing the existing Tree-sitter C++ grammar or a Clang AST would accept the
  wrong language first and reject it later, tie Stainless to full C++ grammar
  decisions, and work against the goal of a small Rust-embeddable compiler.

## Recommended first implementation slice

The first milestone should prove the whole architecture with a deliberately
tiny language rather than attempt broad C++ compatibility. Current progress is
recorded inline so implemented syntax is not confused with planned work:

1. **Implemented for the initial slice:** `stainless-compiler`,
   `stainless-syntax`, `stainless-runtime`, `stainless-build`, and `stainlessc`
   exist. The CLI supplies hidden Cargo integration when generated code needs
   the JSON runtime.
2. **Implemented:** tokenize identifiers, literals, comments, punctuation, and
   keywords with byte spans and lossless trivia.
3. **Implemented for the listed subset:** parse namespaces, imports, function
   definitions, structs/classes/interfaces, fields, data and interface bases,
   access labels, aggregates, typed
   local bindings, struct constructors and initializer lists, blocks, calls,
   arithmetic, `return`, `throw`, `try`/`catch`, `if`/`else`, and classic/range
   `for` loops plus typed lambdas with explicit, initializer, mutable, and
   async forms, direct postfix `.await`, and stored callable signatures into a
   Rowan CST with recoverable error nodes, plus compiler-native JSON arrays and
   objects.
4. **In progress:** typed CST views, AST lowering, structural validation, the
   initial single-file name/type/call resolver, move/borrow analysis, and
   resolved HIR construction are implemented for the function/control-flow and
   struct/class/interface subsets, including checked exceptions and throwing
   constructors, shared `function` and move-only `function_mut` values, and the
   local/function ownership-pointer family with nullable flow tracking, shared
   handle copies, weak promotion, atomic slots, exact interface conformance,
   Rust-trait lowering, and dynamic class-owner erasure. Namespace-scope pointer
   storage, `require(...)`, weak/atomic interface erasure, and general ownership
   through fields remain.
5. **In progress:** the initial `Vec`, `String`, linked-list, queue, ordered
   map/set, and JSON metadata is connected through resolution and code
   generation. The versioned package binding manifest is
   parsed and merged with compiler built-ins; deterministic wrappers for its
   `regex::Regex::new` and `Regex::is_match` entries and call/thread callback
   entries are generated. Cargo rejects a deliberately stale target and checks
   the emitted `Fn`, `FnMut`, `FnOnce`, function-pointer, and
   `Send + 'static` signatures, plus an async `Fn(...) -> Future` callback and
   awaited Rust method. Next, validate the selected dependency and
   feature set through Cargo metadata.
6. **In progress:** structured Rust emission, formatting, and representative
   generated-file compile/behavior tests are implemented, including a Cargo
   execution test against the real JSON runtime. Source-mapping Rust/Cargo
   diagnostics back to Stainless remains.
7. Compile every reference sample as it enters the supported milestone subset,
   and keep unsupported later-stage samples as explicit expected diagnostics
   rather than silently accepting partial semantics.

Every accepted construct should have three kinds of tests: valid source and its
CST, invalid source and its diagnostics, and source-to-generated-Rust behavior.

## Non-goals

- Being a drop-in C++ compiler or transpiling arbitrary existing C++.
- Preserving C++ ABI, undefined behavior, or platform-specific implementation
  details.
- Pretending every Rust signature is directly expressible in Stainless without
  preserving its lifetime, trait, unsafe, and ownership constraints or using a
  checked wrapper where necessary.
- Using generated Rust as a substitute for defining Stainless semantics.
- Adding syntax before its ownership, type-checking, and lowering behavior is
  specified.
