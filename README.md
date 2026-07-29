# Stainless

Stainless is a new C++-like language that transpiles to Rust.

> **Status:** design and library-research stage. No language syntax is stable yet.

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

## Language boundaries

The exact syntax still needs to be specified. The initial language should focus
on constructs with direct Rust equivalents:

- functions, blocks, local bindings, control flow, structs, classes, enums, and
  methods;
- namespaces/modules and explicit imports rather than textual inclusion;
- pure-data structs with data-only inheritance implemented as composition;
- sealed classes that can implement interfaces but cannot inherit other
  classes;
- interfaces and interface inheritance implemented as Rust traits,
  supertraits, and trait objects where dynamic dispatch is required;
- safe dynamic allocation with ownership represented by `unique_ptr<T>`,
  `shared_ptr<T>`, and `weak_ptr<T>`;
- value semantics, explicit moves, borrowing, and references governed by
  Rust-like ownership rules;
- type inference where C++ would commonly use `auto`;
- function and method overloads resolved by exact parameter types;
- a deliberately constrained form of parametric generics, once its lowering is
  specified.

The following C++ features should not be accepted in the initial language
because they have no direct, general safe-Rust translation:

- the preprocessor, textual `#include`, and C/C++ macros;
- behavioral/class implementation inheritance, virtual concrete-class methods
  outside interface contracts, method overriding between classes, virtual
  inheritance, and C++ RTTI;
- default arguments and implicit conversions other than the explicitly defined
  data-base reference coercion;
- unrestricted templates, specialization, SFINAE, and template metaprogramming;
- raw-pointer arithmetic and manual `new`/`delete` memory management;
- exceptions (`throw`, `try`, and `catch`);
- C-style variadics, `goto`, and unrestricted unions.

Destructors/`Drop`, operator traits, FFI, async code, and other less-direct
mappings are deferred. Each needs a written source-level semantic model and
lowering rule before it becomes part of the language.

### Inheritance model

Stainless gives `struct`, `class`, and `interface` distinct roles instead of
using them as nearly interchangeable spellings.

An `interface` is behavior-only:

- It contains method signatures but no instance data, constructors, or
  destructors.
- Interface inheritance lowers directly to Rust supertraits.
- An interface may inherit only from other interfaces.
- A class may implement one or more interfaces.
- Interface calls may use static dispatch when the concrete class is known or
  Rust trait-object dispatch when a dynamic interface value is required.

A `struct` is data-only:

- Its declaration contains fields, data-base declarations, and data-related
  metadata, but no methods.
- A struct may inherit data only from another struct. This is not subtype
  inheritance: it lowers to an embedded Rust field containing the base value.
- A struct cannot inherit or implement an interface and cannot inherit from a
  class.
- Inherited fields may use convenient source-level lookup, but the compiler
  lowers that access to the corresponding embedded-field path. Ambiguous field
  names must be diagnosed rather than selected by an implicit precedence rule.
- A reference to a derived struct implicitly coerces to a reference to its data
  base. This lowers to a safe reference projection such as `&derived.base` or
  `&mut derived.base`, follows the normal mutability/reborrowing rules, and may
  traverse multiple unambiguous levels of data inheritance.
- This reference coercion never converts or slices an owned derived value,
  permits a base-to-derived downcast, or introduces runtime type information.
  If the same base is reachable through more than one data-inheritance path,
  the source must select the intended base path explicitly.
- There are no virtual data bases, compiler-inserted vtable pointers, C++ object
  slicing, or C++ layout/ABI guarantees. An explicitly declared field may still
  contain an interface value whose Rust representation carries trait-object
  metadata.

A `class` combines data with behavior but is sealed against class inheritance:

- A class may declare its own fields, methods, and implementations of interface
  methods.
- A class cannot inherit from another class or from a struct. Data reuse must
  use fields and composition.
- A class may implement interfaces, but there is no class method inheritance or
  overriding. Only calls made through an interface participate in virtual
  dispatch.
- An ordinary class method is statically dispatched and cannot be marked
  `virtual`. An interface method implementation supplies behavior for its
  interface slot; it does not override a method inherited from another class.
- Classes may therefore require vtable-based dispatch, while structs are
  guaranteed never to do so.

“Vtable pointer” describes the semantic distinction using C++ terminology, not
an object-layout promise. In generated Rust, the vtable metadata will normally
be carried by a `dyn Interface` fat pointer rather than stored inside the
concrete class value. Stainless must not expose or depend on the physical vtable
layout.

Functions and associated functions that operate on a struct are declared in a
separate implementation construct. They are not members of its data declaration,
are not inherited by data-derived structs, and cannot be virtual or overridden.
Reuse of behavior must be explicit: call a base-value function or extract a
helper function.

For example, data inheritance lowers conceptually as follows:

```cpp
struct Point2 {
    float x;
    float y;
}

struct Point3 : Point2 {
    float z;
}
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

Consequently, a Stainless reference conversion conceptually lowers as follows:

```cpp
Point3& derived = /* ... */;
Point2& base = derived;
```

```rust
let derived: &mut Point3 = /* ... */;
let base: &mut Point2 = &mut derived.base;
```

The final Stainless syntax may give the embedded base a stable explicit name;
that decision must be made before field lookup and multiple data bases are
implemented.

### Function overloading

Stainless supports function and method overloads with deliberately simpler
rules than C++:

- An overload is selected using the exact canonical types of its arguments.
  Type aliases may be normalized, but the compiler must not insert numeric
  widening, borrowing, dereferencing, user-defined conversions, or other
  implicit conversions to make a candidate match.
- Data-base reference coercion is likewise not used to make an overload
  candidate match. It may be applied after a function has already been selected
  unambiguously, such as for a call to a non-overloaded function.
- The return type does not participate in overload selection.
- A call that has no exact match, or more than one exact match, is a compile
  error with the candidate signatures shown in its diagnostic.
- Default arguments and variadic overloads are not supported.
- Every overload lowers to a unique, deterministic Rust name derived from its
  fully qualified Stainless name and canonical parameter types. The mangling
  format will be versioned and specified before it becomes a compatibility
  promise; it must not depend on randomized or implementation-defined hashing.

For example, these Stainless declarations:

```cpp
int parse(int value);
int parse(string value);
```

may lower conceptually to Rust functions named `parse__i32` and
`parse__string`. The final mangling scheme must also handle modules, methods,
references, generic arguments, and collisions without relying on this
illustrative spelling.

### Dynamic allocation and owning pointers

Dynamic allocation is supported only through ownership-bearing types. Stainless
does not have an owning raw-pointer form:

| Stainless | Rust lowering | Semantics |
| --- | --- | --- |
| `unique_ptr<T>` | `Box<T>` | One non-null owner; movable but not copyable |
| `shared_ptr<T>` | `Arc<T>` | Non-null, atomically reference-counted, immutable shared ownership |
| `weak_ptr<T>` | `Weak<T>` | Non-owning observation that does not keep the allocation alive |
| `optional<unique_ptr<T>>` | `Option<Box<T>>` | Nullable unique ownership |
| `optional<shared_ptr<T>>` | `Option<Arc<T>>` | Nullable shared ownership |
| `atomic_shared_ptr<T>` | Initially `RwLock<Arc<T>>` | Synchronized, replaceable shared handle |
| `vector<T>` | `Vec<T>` | Owned dynamically sized sequence |

Allocation uses `make_unique<T>(...)` or `make_shared<T>(...)`. There is no
owning `new`, `delete`, placement allocation, or dynamically allocated C-style
array. `drop(value)` may release an owning handle before the end of its scope;
otherwise destruction happens automatically. The initial allocation-failure
behavior follows ordinary Rust allocation and aborts instead of throwing an
exception.

`unique_ptr<T>` permits mutation of its pointee when the pointer binding is
mutable and no active borrow prevents it. Moving it transfers ownership and
invalidates the previous binding.

`shared_ptr<T>` deliberately provides immutable shared data:

- Dereferencing it yields only a shared/const reference.
- Fields cannot be assigned through it, a mutable reference cannot be extracted
  from it, and mutating methods cannot be called through it.
- Stainless does not expose `Arc::get_mut`, copy-on-write operations such as
  `Arc::make_mut`, or an interior-mutability escape hatch.
- Reassigning a mutable `shared_ptr<T>` binding to a new pointer is allowed.
  Reassignment replaces only that handle and decrements the old allocation's
  strong count; it neither changes the old pointee nor redirects other handles.
- Duplicating shared ownership is explicit, using `clone(pointer)`, and lowers
  to `Arc::clone`.
- `weak_ptr<T>::lock()` returns `optional<shared_ptr<T>>`.

#### Thread safety

Atomic reference counting makes the ownership control block thread-safe; it
does not make concurrent mutation of one pointer binding safe. Stainless
therefore applies all of the following rules:

- A `shared_ptr<T>` may cross a thread boundary only when `T` satisfies the
  equivalents of Rust's `Send` and `Sync` traits. `Send` permits ownership and
  destruction on another thread; `Sync` permits concurrent shared references.
- These properties are structural for generated types: a struct or class is
  sendable and shareable only when all of its fields are. Stainless code cannot
  provide an unchecked manual implementation of either property.
- A shared interface pointer crossing a thread boundary lowers to
  `Arc<dyn Interface + Send + Sync>`. Every concrete class stored in it must
  satisfy both bounds.
- Each thread receives its own handle through `clone(pointer)` or an ownership
  move. Atomic reference counts make concurrent cloning and dropping of
  separate handles safe.
- A thread may reassign its own local handle. The same mutable pointer binding
  cannot be concurrently accessed or reassigned by multiple threads.
- An ordinary global `shared_ptr<T>` binding is immutable after initialization,
  so threads may only load or clone it. An ordinary mutable global shared
  pointer is rejected.

Globally replaceable shared state uses `atomic_shared_ptr<T>`, not an ordinary
`shared_ptr<T>`. It changes which immutable allocation a synchronized slot
points to; it never mutates a pointee:

- `load()` returns a cloned `shared_ptr<T>` snapshot.
- `store(new_value)` replaces the slot's handle.
- `swap(new_value)` replaces the handle and returns the previous one.
- Existing snapshots continue to refer to the old allocation.

The initial Rust lowering may use `RwLock<Arc<T>>`; a more specialized atomic
implementation can replace it later without changing these semantics.

#### Namespace-scope variables

Stainless follows C++ syntax: every variable declared at namespace scope has
static storage duration. There is no separate `global` keyword. A namespace
qualifier controls where the name is found, while `static` at namespace scope
controls linkage/visibility rather than whether the variable is global.

Namespace-scope declarations must use one of the following safe forms:

- `const T value = ...;` declares an immutable global. It may be accessed from
  multiple threads only when `T` is `Sync`.
- `const shared_ptr<T> value = ...;` declares an immutable global handle.
  Threads may access it directly or clone their own handles when `T` is
  `Send + Sync`.
- A synchronization-aware type such as `atomic_shared_ptr<T>` may be changed
  through its `load`, `store`, and `swap` operations.
- `thread_local T value = ...;` gives each thread an independent instance.

An ordinary mutable namespace-scope variable is rejected because it would
require Rust `static mut`-like unsynchronized access. It must instead be made
immutable, thread-local, or represented by an explicitly synchronized type.
Threads may refer to safe namespace-scope variables directly; the values do not
need to be passed as thread arguments.

Constant global initializers may lower directly to Rust statics. Runtime global
initialization requires a thread-safe one-time initialization mechanism such as
`OnceLock`; its precise source-order and failure semantics must be specified
before runtime-initialized globals are implemented.

The Stainless type checker must enforce the thread-boundary rules itself so it
can issue source-level diagnostics. Generated Rust retains the corresponding
`Send`/`Sync` bounds, allowing `rustc` to verify them again. Combined with the
ban on mutable pointee access, unsafe code, owning raw pointers, and
interior-mutability escape hatches, this provides the language's data-race
guarantee.

For classes, a `unique_ptr<Class>` or `shared_ptr<Class>` may coerce to the
corresponding pointer to an implemented interface. These ownership-preserving
interface coercions lower to `Box<dyn Interface>` or `Arc<dyn Interface>`.
Like data-base reference coercion, they apply only after a target type or
function has been selected and do not make an overload candidate match.

For example:

```cpp
shared_ptr<Config> current = make_shared<Config>(/* ... */);
shared_ptr<Config> observer = clone(current);

current = make_shared<Config>(/* replacement */);
```

The assignment changes `current` to refer to a new immutable `Config`.
`observer` continues to refer to the original value until its own handle is
dropped or reassigned.

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
  -> typed language AST
  -> name, type, and ownership analysis
  -> small Rust-shaped high-level IR (HIR)
  -> Rust syntax tree/tokens
  -> formatted .rs source
```

The CST and semantic AST should remain separate. The CST retains the spelling,
comments, and malformed input needed for diagnostics and future editor tools.
The AST/HIR should contain only validated Stainless concepts with defined Rust
semantics. Transpilation must happen from the validated HIR, not directly from
parser nodes.

## Rust library survey

Research snapshot: 2026-07-29. Versions should be selected and locked when the
first compiler spike is created rather than copied from this document.

| Area | Candidate | Assessment |
| --- | --- | --- |
| Lexing | [`logos`](https://docs.rs/logos/latest/logos/) | **Recommended.** Rust-native, derive-based, fast, and provides byte spans. Trivia should be tokenized rather than skipped so the CST stays lossless. Context-dependent token combinations can be handled by the parser. |
| Parsing | Hand-written recursive descent plus a Pratt expression parser | **Recommended starting point.** This gives precise control over C++-like ambiguities, recovery, contextual keywords, and CST events. It also keeps the grammar and compiler fully in Rust. |
| Parsing alternative | [`chumsky`](https://docs.rs/chumsky/latest/chumsky/) | **Worth a small comparison spike.** It supports token-stream parsing, context-sensitive parsers, spans, and error recovery. It is attractive if it remains readable and can emit the lossless CST/error nodes we need; combinator complexity should be measured before adopting it. |
| Lossless CST | [`rowan`](https://docs.rs/rowan/latest/rowan/) | **Recommended.** It provides immutable green trees, syntax nodes/tokens, text ranges, and typed-AST support while leaving the language-specific node model to us. It is a tree library, not a parser. |
| CST schema/code generation | [`ungrammar`](https://docs.rs/ungrammar/latest/ungrammar/) | **Optional later.** Useful for declaring CST shapes and generating typed wrappers. It explicitly does not generate a parser, so it is not needed for the first slice. |
| Diagnostics | [`miette`](https://docs.rs/miette/latest/miette/) | **Recommended at the public API/CLI boundary.** It supports structured diagnostics, source spans, labels, error codes, rich terminal output, and a narratable renderer. Internal compiler diagnostics should remain our own data types. |
| Rust emission | [`proc-macro2`](https://docs.rs/proc-macro2/latest/proc_macro2/), [`quote`](https://docs.rs/quote/latest/quote/), [`syn`](https://docs.rs/syn/latest/syn/), and [`prettyplease`](https://docs.rs/prettyplease/latest/prettyplease/) | **Recommended when code generation starts.** Generate structured Rust tokens/AST, validate them with `syn`, and format with `prettyplease` without requiring an installed `rustfmt`. Avoid using string concatenation as the semantic code generator. |
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

The first milestone should prove the whole architecture with a deliberately tiny
language rather than attempt broad C++ compatibility:

1. Create a Cargo workspace with separate syntax, semantic/HIR, Rust-codegen,
   and CLI/public-facade crates.
2. Tokenize identifiers, literals, comments, punctuation, and a few keywords
   with byte spans and lossless trivia.
3. Parse function definitions, typed local bindings, blocks, calls, arithmetic,
   `return`, and `if`/`else` into a Rowan CST, including recoverable error nodes.
4. Lower the CST to a small typed AST/HIR and reject unresolved names and type
   mismatches before code generation.
5. Emit and format Rust, then compile representative generated files in
   integration tests.
6. Compare the hand-written parser with a narrowly scoped Chumsky prototype
   before the grammar grows. Keep the option with clearer recovery behavior and
   more maintainable tests.

Every accepted construct should have three kinds of tests: valid source and its
CST, invalid source and its diagnostics, and source-to-generated-Rust behavior.

## Non-goals

- Being a drop-in C++ compiler or transpiling arbitrary existing C++.
- Preserving C++ ABI, undefined behavior, or platform-specific implementation
  details.
- Using generated Rust as a substitute for defining Stainless semantics.
- Adding syntax before its ownership, type-checking, and lowering behavior is
  specified.
