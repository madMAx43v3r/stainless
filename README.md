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

- functions, blocks, local bindings, control flow, structs, enums, and methods;
- namespaces/modules and explicit imports rather than textual inclusion;
- value semantics, explicit moves, borrowing, and references governed by
  Rust-like ownership rules;
- type inference where C++ would commonly use `auto`;
- a deliberately constrained form of parametric generics, once its lowering is
  specified.

The following C++ features should not be accepted in the initial language
because they have no direct, general safe-Rust translation:

- the preprocessor, textual `#include`, and C/C++ macros;
- class inheritance, virtual inheritance, and C++ RTTI;
- function overloading, default arguments, and implicit conversions;
- unrestricted templates, specialization, SFINAE, and template metaprogramming;
- raw-pointer arithmetic and manual `new`/`delete` memory management;
- exceptions (`throw`, `try`, and `catch`);
- C-style variadics, `goto`, and unrestricted unions.

Destructors/`Drop`, operator traits, dynamic dispatch, FFI, async code, and
other less-direct mappings are deferred. Each needs a written source-level
semantic model and lowering rule before it becomes part of the language.

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
