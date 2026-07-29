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

## Provisional syntax examples

The `.stl` files in [`docs/ref`](docs/ref/) show the intended source style
before the parser exists. They are design references rather than a stable
language specification or executable test suite:

- [`01_basics.stl`](docs/ref/01_basics.stl) — functions, namespaces, local
  variables, and control flow.
- [`02_structs_and_data_inheritance.stl`](docs/ref/02_structs_and_data_inheritance.stl)
  — out-of-type function definitions, data inheritance, and base-reference
  coercion.
- [`03_interfaces.stl`](docs/ref/03_interfaces.stl) — static struct interfaces
  and dynamically dispatched class interfaces.
- [`04_exact_overloads.stl`](docs/ref/04_exact_overloads.stl) — free-function
  and member-function overloads.
- [`05_ownership_and_containers.stl`](docs/ref/05_ownership_and_containers.stl)
  — `unique_ptr`, immutable `shared_ptr`, `vector`, moves, and borrows.
- [`06_threads_and_globals.stl`](docs/ref/06_threads_and_globals.stl) —
  namespace-scope storage, `atomic_shared_ptr`, thread-local state, and thread
  handles.
- [`07_native_runtime_api.stl`](docs/ref/07_native_runtime_api.stl) — trusted
  `native`/`sealed` declarations used to define the runtime API whitelist.
- [`08_numeric_types.stl`](docs/ref/08_numeric_types.stl) — fixed-width
  integers, `usize`, `f32`/`f64`, inference defaults, and literal suffixes.
- [`09_value_semantics.stl`](docs/ref/09_value_semantics.stl) — default
  construction, implicit struct copies, explicit moves, and explicit class
  cloning.

As syntax and semantics become executable, these examples should be converted
into parser, diagnostic, and transpilation fixtures rather than allowed to
drift from the implementation.

## Language boundaries

The exact syntax still needs to be specified. The initial language should focus
on constructs with direct Rust equivalents:

- functions, blocks, local bindings, control flow, structs, classes, enums, and
  methods;
- namespaces/modules and explicit imports rather than textual inclusion;
- vtable-free structs with data-only inheritance implemented as composition;
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
- default arguments and C++'s general implicit-conversion sequences; only the
  narrow reference, pointer, and interface bindings explicitly specified below
  are permitted;
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
  destructors. Function bodies cannot appear inside the interface.
- Interface inheritance lowers directly to Rust supertraits.
- An interface may inherit only from other interfaces.
- A struct or class may implement one or more interfaces.
- Interface calls on a concrete struct, or through a generic constrained by an
  interface, always use static dispatch. A struct cannot be converted to an
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
- A struct may implement interfaces using static dispatch, but cannot inherit
  from a class.
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
- Implementing an interface does not change a struct's representation. The
  generated Rust uses an ordinary trait implementation with static dispatch,
  and Stainless prohibits creating a `dyn Interface` from the struct.

A `class` combines data with behavior but is sealed against class inheritance:

- A class may declare its own fields, member functions, and implementations of
  interface methods. As with a struct, only declarations appear inside the
  class; function bodies are defined outside it.
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

Member functions are declared inside their struct or class but defined outside
it using a qualified C++-style name. Stainless has no Rust-style `impl` syntax:

```cpp
struct Buffer : Sequence<i32> {
    vector<i32> values;
    usize size() const;
};

usize Buffer::size() const {
    return values.size();
}
```

The declaration and definition signatures must match exactly. Struct member
functions are not inherited by data-derived structs and cannot be virtual or
overridden. Reuse of behavior must be explicit: call a function on the embedded
base value or extract a helper function. Listing an interface creates an
implementation obligation: matching member declarations and their out-of-type
definitions must satisfy every required interface function. A `native`
declaration is the exception because its definition is supplied by the trusted
runtime.

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

The final Stainless syntax may give the embedded base a stable explicit name;
that decision must be made before field lookup and multiple data bases are
implemented.

### Declaration syntax and contextual modifiers

Declaration modifiers follow the declaration kind:

```cpp
interface native sealed vector_api<T> {
    usize size() const;
    bool empty() const;
    void push_back(T value);
};

struct native vector<T> : vector_api<T> {
    usize size() const;
    bool empty() const;
    void push_back(T value);
};
```

The order is the declaration kind, its modifiers, and then the declared name.
`native` and `sealed` are contextual declaration modifiers, not globally
reserved keywords; outside the modifier position they remain ordinary
identifiers.

- `native` means that storage, behavior, or both are supplied by the trusted
  Rust runtime/backend rather than a Stainless definition. Normal project
  source cannot use `native` to bind arbitrary Rust code.
- `sealed` prevents code outside the defining standard-library module from
  inheriting or implementing the declaration.

The set of modifiers valid for each declaration kind will be explicit in the
grammar. They must not be parsed as general prefix modifiers.

### Reserved implementation identifiers

Every identifier beginning with `__` is reserved for the Stainless compiler,
bundled standard library, and Rust runtime. Project source cannot declare a
function, type, namespace, field, parameter, local variable, or other symbol
whose name begins with this prefix. A leading single underscore remains
available to project code.

User code may call documented compiler-provided operations with reserved names,
such as `atomic_shared_ptr<T>::__load()`, but cannot define, overload, shadow,
or replace them. Generated Rust symbols may also use a `__stainless` prefix,
giving the backend a namespace that cannot collide with source declarations.
The lexer still treats these spellings as identifiers; name validation enforces
the reservation and reports their declaration as an error.

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

Numeric literals follow Rust-style inference and suffixes. An integer literal
uses its expected type and otherwise defaults to `i32`; a floating literal uses
its expected type and otherwise defaults to `f64`. Suffixes such as `42u64`,
`3.0f32`, and `3.0f64` select an exact type.

`f32` and `f64` lower directly to Rust's IEEE-754 binary32 and binary64 types,
including their infinities, signed zero, and NaN behavior. There is no implicit
promotion between numeric types: mixed-width arithmetic and overload arguments
must use an explicit conversion. The conversion syntax, float-to-integer
behavior, and integer-overflow policy must be specified before implementation;
overflow behavior must not accidentally vary with the generated Cargo build
profile.

### Initialization, copying, and moving

Stainless never permits uninitialized storage. A declaration without an
explicit initializer, such as `vector<i32> values;`, requests default
construction; it does not create an indeterminate value. It is valid only when
the type has an accessible, non-deleted zero-argument constructor that fully
initializes the value. Primitive numeric types have no implicit default
constructor and therefore require an initializer.

When no constructor prevents it, a default constructor may be synthesized only
if every field can itself be default-constructed. Otherwise it is implicitly
deleted and a default-construction attempt is a compile error. Stainless may
use C++-style `= delete` syntax to make a constructor unavailable explicitly.
Aggregate initialization remains valid only when it initializes every required
field.

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
- Copying a `shared_ptr<Class>` duplicates only its ownership handle and
  continues to refer to the same immutable class object; it does not clone that
  object.

A named value never moves implicitly. Passing, returning, initializing, or
assigning from a named non-copy value requires `move(value)`. A fresh temporary
can initialize or pass directly because it has no binding that could be used
afterward. Using a moved binding is a compile error until it is explicitly
reinitialized in a context where assignment is allowed. An active borrow also
prevents moving its owner.

`shared_ptr<T>` is an implicitly copyable, nullable value type. Copying a
non-null handle increments its strong reference count; copying a null handle
simply copies the null state. Copy construction, assignment, and pass-by-value
leave the source handle valid. `move(shared_pointer)` transfers either state
without a reference-count increment and invalidates the source binding.

The Stainless semantic pass must track moves, borrows, conditional control
flow, and reinitialization so it can diagnose errors against `.stl` source.
Lowering `move(value)` to an actual Rust move lets `rustc` independently verify
the result, but generated-Rust errors are a backstop rather than the primary
Stainless diagnostic mechanism.

### Reference parameters and value returns

References exist only as function parameters, including the implicit receiver
of a member function:

```cpp
void inspect(const Config& value); // lowers to &Config
void update(Config& value);        // lowers to &mut Config
```

A reference cannot be declared as a field, local variable, namespace-scope
variable, container element, type alias target, or return type. Functions always
return values. Returning `move(value)` explicitly transfers a named local; the
compiler may elide or optimize the resulting Rust move when doing so preserves
observable behavior.

Reference binding follows these rules:

- `T&` requires an exclusive mutable argument and prevents all conflicting
  access for the duration of the call.
- `const T&` creates a shared borrow. A `T&` may implicitly reborrow as
  `const T&`.
- References are non-null and cannot escape the call.
- A derived-struct argument may project to a data-base reference after the
  function has been selected.
- Passing a `unique_ptr<T>` to a `T&` or `const T&` parameter borrows its
  pointee; passing a `shared_ptr<T>` may bind only to `const T&` and only where
  control-flow analysis proves that the shared pointer is non-null.

References to ownership handles are forbidden:

```cpp
void invalid(unique_ptr<T>& value);        // rejected
void invalid(const shared_ptr<T>& value);  // rejected
```

Owning handles must be passed by value: `unique_ptr<T>` requires an explicit
move, while `shared_ptr<T>` copies implicitly. The exception is a synchronized
pointer slot such as `atomic_shared_ptr<T>`, which may be passed by reference
because its API provides synchronized access to the binding.

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
  fully qualified Stainless name and canonical parameter types. The mangling
  format will be versioned and specified before it becomes a compatibility
  promise; it must not depend on randomized or implementation-defined hashing.

For example, these Stainless declarations:

```cpp
i32 parse(i32 value);
i32 parse(string value);
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
| `shared_ptr<T>` | `Option<Arc<T>>` | Nullable, implicitly copyable, atomically reference-counted, immutable shared ownership |
| `weak_ptr<T>` | `Weak<T>` | Non-owning observation that does not keep the allocation alive |
| `optional<unique_ptr<T>>` | `Option<Box<T>>` | Nullable unique ownership |
| `atomic_shared_ptr<T>` | Initially `RwLock<Option<Arc<T>>>` | Synchronized, replaceable nullable shared handle |
| `vector<T>` | `Vec<T>` | Owned dynamically sized sequence |

Allocation uses `make_unique<T>(...)` or `make_shared<T>(...)`. There is no
owning `new`, `delete`, placement allocation, or dynamically allocated C-style
array. `drop(move(value))` may consume a named owning handle before the end of
its scope; a fresh temporary may be dropped directly. This keeps the rule that
named values never move implicitly. Otherwise destruction happens
automatically. The initial allocation-failure behavior follows ordinary Rust
allocation and aborts instead of throwing an exception.

`unique_ptr<T>` permits mutation of its pointee when the pointer binding is
mutable and no active borrow prevents it. Moving it transfers ownership and
invalidates the previous binding.

`shared_ptr<T>` deliberately provides immutable shared data:

- Default construction and assignment from `nullptr` produce a null handle.
  `make_shared<T>(...)` produces a non-null handle.
- Dereferencing it yields only a shared/const reference.
- Fields cannot be assigned through it, a mutable reference cannot be extracted
  from it, and mutating methods cannot be called through it.
- Stainless does not expose `Arc::get_mut`, copy-on-write operations such as
  `Arc::make_mut`, or an interior-mutability escape hatch.
- Reassigning a mutable `shared_ptr<T>` binding to a new pointer is allowed.
  Reassignment replaces only that handle and decrements the old allocation's
  strong count; it neither changes the old pointee nor redirects other handles.
- Copy construction, assignment, and pass-by-value implicitly duplicate the
  handle and lower to an `Arc::clone` where a reference-count increment is
  required.
- `move(pointer)` remains available when the caller wants to transfer a handle
  without incrementing its reference count.
- `lock(pointer)` returns a non-null `shared_ptr<T>` when the weak allocation
  remains alive and a null `shared_ptr<T>` otherwise.

#### Nullable shared-pointer refinement

Stainless tracks whether each `shared_ptr<T>` binding is definitely null,
definitely non-null, or unknown. Member access and conversion to a `const T&`
parameter require a definitely non-null binding:

```cpp
shared_ptr<Config> config; // default-constructed null handle

if (!config) {
    config = make_shared<Config>(/* ... */);
}

i32 version = config.version; // accepted: non-null on every incoming path
```

The initial analysis is deliberately conservative:

- `make_shared<T>(...)` establishes non-null and `nullptr` establishes null.
- `if (pointer)`, `!pointer`, and comparisons with `nullptr` refine the
  corresponding control-flow branches.
- A guard that exits the null branch establishes non-null afterward.
- Initialization, copying, or assignment gives the destination the source
  expression's fact; assignment first invalidates the destination's old fact.
- `move(pointer)` transfers the fact to its destination and invalidates the
  source binding along with its fact.
- At a control-flow merge, a fact survives only when it agrees on every
  incoming path. Complex cases that the analysis cannot prove are rejected.

After a successful guard such as `if (!config) return;`, code generation
introduces a hidden borrowed refinement of type `&T` and directs pointee access
through it:

```rust
let config: Option<Arc<Config>> = /* ... */;

let __stainless_config_nonnull: &Config = match config.as_deref() {
    Some(value) => value,
    None => return,
};

let version = __stainless_config_nonnull.version;
```

The hidden binding is valid only until assignment, move, or a control-flow
boundary invalidates the proof. A later non-null generation receives a new
hidden binding. Handle operations such as implicit copying continue to use the
original `Option<Arc<T>>`.

The compiler must not create an extra owned `Arc<T>` merely to represent a
non-null fact: cloning would observably increase the strong count and could
delay destruction, while moving the `Arc` out would change the source handle.
The hidden `&T` changes neither ownership nor lifetime.

#### Member access and owning-pointer operations

Stainless does not have C++'s `->` member-access operator. The `.` operator
automatically dereferences references, `unique_ptr<T>`, and `shared_ptr<T>` when
resolving a field or member function on `T`:

```cpp
unique_ptr<Config> unique = make_unique<Config>(/* ... */);
unique.version = 2;

shared_ptr<Config> shared = make_shared<Config>(/* ... */);
i32 version = shared.version;
```

Automatic receiver dereferencing is a member-access rule, not a general
implicit conversion used during overload selection. A mutable unique owner may
provide mutable member access when borrowing permits it; a shared owner always
provides const member access and must first be proven non-null. The same dot
syntax calls an interface function through a proven-non-null owning interface
pointer.

`unique_ptr<T>` and `shared_ptr<T>` do not expose their own member functions, so
their operations cannot collide with members of `T`. Ownership operations use
free functions:

- `move(pointer)` transfers an owning handle.
- `drop(move(pointer))` consumes and releases a named handle early.
- `downgrade(pointer)` creates a `weak_ptr<T>` from a proven-non-null
  `shared_ptr<T>`.
- `lock(pointer)` attempts to promote a weak handle.

No `get`, `release`, `reset`, pointer arithmetic, or raw-pointer escape is
provided. `atomic_shared_ptr<T>` is a synchronized slot rather than a
dereferenceable owning pointer, so its own `__load`, `__store`, and `__swap`
operations continue to use dot syntax.

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
  `Option<Arc<dyn Interface + Send + Sync>>`. Every concrete class stored in a
  non-null handle must satisfy both bounds.
- Passing a `shared_ptr<T>` to a thread copies the handle by value unless the
  caller explicitly uses `move(pointer)`. Atomic reference counts make
  concurrent copying and dropping of separate handles safe.
- A thread may reassign its own local handle. The same mutable pointer binding
  cannot be concurrently accessed or reassigned by multiple threads.
- An ordinary global `shared_ptr<T>` binding is immutable after initialization,
  so threads may only read or copy its handle. An ordinary mutable global shared
  pointer is rejected.

Globally replaceable shared state uses `atomic_shared_ptr<T>`, not an ordinary
`shared_ptr<T>`. It changes which immutable allocation a synchronized slot
points to; it never mutates a pointee:

- `__load()` returns a copied, potentially null `shared_ptr<T>` snapshot.
- `__store(new_value)` replaces the slot's handle, including with null.
- `__swap(new_value)` replaces the handle and returns the previous one.
- Existing snapshots continue to refer to the old allocation.

The initial Rust lowering may use `RwLock<Option<Arc<T>>>`; a more specialized
atomic implementation can replace it later without changing these semantics.

#### Namespace-scope variables

Stainless follows C++ syntax: every variable declared at namespace scope has
static storage duration. There is no separate `global` keyword. A namespace
qualifier controls where the name is found, while `static` at namespace scope
controls linkage/visibility rather than whether the variable is global.

Namespace-scope declarations must use one of the following safe forms:

- `const T value = ...;` declares an immutable global. It may be accessed from
  multiple threads only when `T` is `Sync`.
- `const shared_ptr<T> value = ...;` declares an immutable global handle.
  The handle may be null; threads may inspect it or copy their own handles when
  `T` is `Send + Sync`.
- A synchronization-aware type such as `atomic_shared_ptr<T>` may be changed
  through its `__load`, `__store`, and `__swap` operations.
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
interface coercions lower to `Box<dyn Interface>` or
`Option<Arc<dyn Interface>>`; a null shared pointer remains null. Like
data-base reference coercion, they apply only after a target type or function
has been selected and do not make an overload candidate match.

For example:

```cpp
shared_ptr<Config> current = make_shared<Config>(/* ... */);
shared_ptr<Config> observer = current;

current = make_shared<Config>(/* replacement */);
```

The assignment changes `current` to refer to a new immutable `Config`.
`observer` continues to refer to the original value until its own handle is
dropped or reassigned.

### Standard-library prelude and Rust runtime

Standard ownership and container types have canonical declarations in the
Stainless `std` namespace and are also re-exported by the built-in prelude.
Both `std::vector<i32>` and unqualified `vector<i32>` name the same type without
a user-written `using` statement. This initially includes `unique_ptr`,
`shared_ptr`, `weak_ptr`, `optional`, `vector`, and `string`.

A compact `stainless-runtime` Rust crate supplies the native implementations.
The bundled Stainless declarations are the source-level API whitelist:

```cpp
vector<i32> values;
values.push_back(10); // accepted Stainless API
values.size();        // accepted Stainless API
values.len();         // error: vector<i32> has no member named len
```

`Vec::len`, `Arc::clone`, and other Rust APIs may be used inside the runtime,
but they are never added to Stainless name resolution. Target-Rust paths such
as `std::vec::Vec` are likewise not Stainless declarations and cannot be called
or named by source programs.

Native standard types implement their bundled Stainless interfaces statically.
For example, the runtime may implement its generated `VectorApi<T>` Rust trait
for `Vec<T>` and translate `vector<T>::size()` to that trait's `size` method.
The generated call uses fully qualified static dispatch, so this does not create
a trait object or give the Stainless struct a vtable.

The compiler resolves every call to one of three typed HIR forms:

```text
user function(FunctionId)
runtime item(RuntimeItemId)
compiler intrinsic(IntrinsicId)
```

There is no arbitrary “call this Rust path” form. A runtime item is registered
by a bundled `native` declaration and has a stable ID, Stainless signature,
receiver constness/mutability, ownership effects, generic constraints, and
runtime entry point. Free functions such as `make_unique` are registered the
same way; only operations that the type or borrow checker must understand,
such as `move` and ownership/interface coercions, remain compiler intrinsics.

Normal project source cannot declare a `native` binding. This prevents the
runtime mechanism from becoming an accidental Rust FFI escape hatch. It is a
language-semantic boundary rather than a security sandbox: a user can still
edit generated Rust or their Cargo project outside Stainless.

The runtime should use `#![forbid(unsafe_code)]` for the supported safe standard
library. Its API version must match the compiler, and conformance tests must
verify that every bundled runtime declaration has exactly one implementation
with the expected generated Rust signature. C++ standard-library functions are
included only when Stainless can give them safe, documented semantics; unsafe
escape APIs such as `unique_ptr::release`, `vector::data`, and `string::c_str`
remain absent.

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

1. Create a Cargo workspace with separate syntax, semantic/HIR, bundled
   standard-library declarations, compact Rust runtime, Rust-codegen, and
   CLI/public-facade crates.
2. Tokenize identifiers, literals, comments, punctuation, and a few keywords
   with byte spans and lossless trivia.
3. Parse function definitions, typed local bindings, blocks, calls, arithmetic,
   `return`, and `if`/`else` into a Rowan CST, including recoverable error nodes.
4. Lower the CST to a small typed AST/HIR, classify every call as user-defined,
   registered runtime, or intrinsic, and reject unresolved names and type
   mismatches before code generation.
5. Implement one native standard type and its interface in the safe Rust
   runtime, proving that unregistered Rust methods remain invisible to
   Stainless.
6. Emit and format Rust, then compile representative generated files in
   integration tests.
7. Compare the hand-written parser with a narrowly scoped Chumsky prototype
   before the grammar grows. Keep the option with clearer recovery behavior and
   more maintainable tests.

Every accepted construct should have three kinds of tests: valid source and its
CST, invalid source and its diagnostics, and source-to-generated-Rust behavior.

## Non-goals

- Being a drop-in C++ compiler or transpiling arbitrary existing C++.
- Preserving C++ ABI, undefined behavior, or platform-specific implementation
  details.
- Calling arbitrary Rust functions or treating Rust crates as implicit
  Stainless namespaces.
- Using generated Rust as a substitute for defining Stainless semantics.
- Adding syntax before its ownership, type-checking, and lowering behavior is
  specified.
