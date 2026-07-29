//! Compiler infrastructure for the Stainless language.
//!
//! It contains the source front end and the native Rust binding registry used
//! to resolve APIs that Stainless can lower without introducing wrapper
//! newtypes.

pub mod ast;
mod diagnostic;
pub mod interop;
pub mod lowering;
pub mod semantics;

pub use diagnostic::{Diagnostic, DiagnosticPhase};

/// Parses, lowers, and performs the structural semantic checks currently
/// implemented by the front end.
#[must_use]
pub fn analyze(source: &str) -> Analysis {
    let parse = stainless_syntax::parse(source);
    let ast = lowering::lower(&parse.tree());
    let mut diagnostics = parse
        .errors()
        .iter()
        .map(|error| {
            Diagnostic::syntax(
                "SYN000",
                error.message.clone(),
                ast::Span::from_text_range(error.range),
            )
        })
        .collect::<Vec<_>>();
    diagnostics.extend(semantics::validate(&ast));
    diagnostics.sort_by_key(|diagnostic| diagnostic.span);

    Analysis {
        parse,
        ast,
        diagnostics,
    }
}

/// All front-end products for one source file.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// Lossless, error-recovering concrete syntax.
    pub parse: stainless_syntax::Parse,
    /// Compiler-owned semantic syntax tree.
    pub ast: ast::SourceFile,
    /// Syntax and structural semantic diagnostics, in source order.
    pub diagnostics: Vec<Diagnostic>,
}
