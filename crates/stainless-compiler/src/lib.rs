//! Compiler infrastructure for the Stainless language.
//!
//! It contains the source front end and the native Rust binding registry used
//! to resolve APIs that Stainless can lower without introducing wrapper
//! newtypes.

pub mod ast;
mod codegen;
mod diagnostic;
pub mod hir;
mod hir_lowering;
pub mod interop;
pub mod lowering;
pub mod resolution;
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
    let semantic_model = match interop::standard_bindings() {
        Ok(bindings) => {
            let resolved = resolution::resolve(&ast, &bindings);
            diagnostics.extend(resolved.diagnostics);
            resolved.model
        }
        Err(error) => {
            diagnostics.push(Diagnostic::semantic(
                "INT001",
                format!("invalid compiler-provided native bindings: {error}"),
                ast.span,
            ));
            resolution::SemanticModel::default()
        }
    };
    diagnostics.sort_by_key(|diagnostic| diagnostic.span);

    Analysis {
        parse,
        ast,
        semantics: semantic_model,
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
    /// Resolved functions, expression types, and call targets.
    pub semantics: resolution::SemanticModel,
    /// Syntax, structural, name, and type diagnostics in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Complete result of one attempted Stainless-to-Rust translation.
#[derive(Clone, Debug)]
pub struct TranspileResult {
    /// Analysis products plus any diagnostics added by backend stages.
    pub analysis: Analysis,
    /// Typed backend IR, absent when analysis or HIR lowering failed.
    pub hir: Option<hir::Program>,
    /// Formatted generated Rust, absent when any required stage failed.
    pub rust: Option<String>,
}

/// Parses, resolves, lowers, and emits one Stainless source file.
///
/// Generation is fail-closed: a source diagnostic or unsupported backend form
/// prevents Rust output instead of producing a partially trustworthy file.
#[must_use]
pub fn transpile(source: &str) -> TranspileResult {
    let mut analysis = analyze(source);
    if !analysis.diagnostics.is_empty() {
        return TranspileResult {
            analysis,
            hir: None,
            rust: None,
        };
    }

    let hir = match hir_lowering::lower(&analysis.ast, &analysis.semantics) {
        Ok(hir) => hir,
        Err(mut diagnostics) => {
            analysis.diagnostics.append(&mut diagnostics);
            analysis
                .diagnostics
                .sort_by_key(|diagnostic| diagnostic.span);
            return TranspileResult {
                analysis,
                hir: None,
                rust: None,
            };
        }
    };
    match codegen::emit(&hir) {
        Ok(rust) => TranspileResult {
            analysis,
            hir: Some(hir),
            rust: Some(rust),
        },
        Err(message) => {
            analysis
                .diagnostics
                .push(Diagnostic::codegen("GEN001", message, analysis.ast.span));
            TranspileResult {
                analysis,
                hir: Some(hir),
                rust: None,
            }
        }
    }
}
