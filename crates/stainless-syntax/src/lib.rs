//! Lossless lexing and concrete syntax trees for Stainless source.
//!
//! The parser intentionally accepts only the first executable language slice.
//! It preserves every source byte in a Rowan tree and reports recoverable
//! diagnostics instead of discarding malformed input.

pub mod ast;
mod kind;
mod lexer;
mod parser;

pub use kind::{StainlessLanguage, SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};
pub use lexer::{LexError, Lexed, Token, lex};
pub use parser::{Parse, ParseError, parse};
pub use rowan::TextRange;
