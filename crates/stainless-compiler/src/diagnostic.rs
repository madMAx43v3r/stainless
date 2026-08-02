use crate::ast::Span;

/// The compiler stage that produced a diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticPhase {
    /// Lexing or parsing.
    Syntax,
    /// Structural semantic validation of the lowered AST.
    Semantic,
    /// Move, borrow, and reference-lifetime validation.
    Ownership,
    /// Lowering from resolved syntax into the compiler's backend IR.
    Hir,
    /// Structured Rust generation or validation.
    Codegen,
}

/// Whether a diagnostic blocks code generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    /// Compilation cannot continue.
    Error,
    /// Compilation continues, but the source should be simplified or reviewed.
    Warning,
}

/// A source diagnostic produced by the compiler.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// Stable diagnostic identifier.
    pub code: &'static str,
    /// Compiler phase.
    pub phase: DiagnosticPhase,
    /// Error or non-blocking warning.
    pub severity: DiagnosticSeverity,
    /// Human-readable explanation.
    pub message: String,
    /// Relevant source range.
    pub span: Span,
}

impl Diagnostic {
    pub(crate) fn syntax(code: &'static str, message: String, span: Span) -> Self {
        Self {
            code,
            phase: DiagnosticPhase::Syntax,
            severity: DiagnosticSeverity::Error,
            message,
            span,
        }
    }

    pub(crate) fn semantic(code: &'static str, message: String, span: Span) -> Self {
        Self {
            code,
            phase: DiagnosticPhase::Semantic,
            severity: DiagnosticSeverity::Error,
            message,
            span,
        }
    }

    pub(crate) fn semantic_warning(code: &'static str, message: String, span: Span) -> Self {
        Self {
            code,
            phase: DiagnosticPhase::Semantic,
            severity: DiagnosticSeverity::Warning,
            message,
            span,
        }
    }

    pub(crate) fn ownership(code: &'static str, message: String, span: Span) -> Self {
        Self {
            code,
            phase: DiagnosticPhase::Ownership,
            severity: DiagnosticSeverity::Error,
            message,
            span,
        }
    }

    pub(crate) fn hir(code: &'static str, message: String, span: Span) -> Self {
        Self {
            code,
            phase: DiagnosticPhase::Hir,
            severity: DiagnosticSeverity::Error,
            message,
            span,
        }
    }

    pub(crate) fn codegen(code: &'static str, message: String, span: Span) -> Self {
        Self {
            code,
            phase: DiagnosticPhase::Codegen,
            severity: DiagnosticSeverity::Error,
            message,
            span,
        }
    }

    /// Returns whether this diagnostic prevents code generation.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == DiagnosticSeverity::Error
    }
}
