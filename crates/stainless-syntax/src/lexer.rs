use logos::Logos;
use rowan::{TextRange, TextSize};

use crate::SyntaxKind;

/// One losslessly retained source token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    /// Concrete token kind.
    pub kind: SyntaxKind,
    /// Byte range in the original UTF-8 source.
    pub range: TextRange,
}

/// A lexical diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexError {
    /// Human-readable diagnostic.
    pub message: String,
    /// Offending byte range.
    pub range: TextRange,
}

/// Tokens and recoverable diagnostics produced by [`lex`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lexed {
    /// Every token, including whitespace, comments, and invalid bytes.
    pub tokens: Vec<Token>,
    /// Lexical diagnostics in source order.
    pub errors: Vec<LexError>,
}

/// Tokenizes Stainless source without dropping trivia or invalid input.
#[must_use]
pub fn lex(source: &str) -> Lexed {
    let mut lexer = RawToken::lexer(source);
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

    while let Some(result) = lexer.next() {
        let span = lexer.span();
        let range = text_range(span.clone());
        let kind = if let Ok(raw) = result {
            raw.into()
        } else {
            errors.push(LexError {
                message: format!("unexpected character `{}`", &source[span]),
                range,
            });
            SyntaxKind::ErrorToken
        };
        tokens.push(Token { kind, range });
    }

    errors.append(&mut lexer.extras);
    errors.sort_by_key(|error| error.range.start());
    Lexed { tokens, errors }
}

#[derive(Logos, Clone, Copy, Debug, Eq, PartialEq)]
#[logos(extras = Vec<LexError>)]
enum RawToken {
    #[regex(r"[ \t\r\n]+")]
    Whitespace,
    #[token("//", lex_line_comment)]
    LineComment,
    #[token("/*", lex_block_comment)]
    BlockComment,

    #[token("namespace")]
    NamespaceKw,
    #[token("use")]
    UseKw,
    #[token("return")]
    ReturnKw,
    #[token("if")]
    IfKw,
    #[token("else")]
    ElseKw,
    #[token("for")]
    ForKw,
    #[token("const")]
    ConstKw,
    #[token("auto")]
    AutoKw,
    #[token("true")]
    TrueKw,
    #[token("false")]
    FalseKw,
    #[token("move")]
    MoveKw,
    #[token("struct")]
    StructKw,
    #[token("class")]
    ClassKw,
    #[token("interface")]
    InterfaceKw,
    #[token("public")]
    PublicKw,
    #[token("private")]
    PrivateKw,
    #[token("throws")]
    ThrowsKw,
    #[token("throw")]
    ThrowKw,
    #[token("try")]
    TryKw,
    #[token("catch")]
    CatchKw,
    #[token("while")]
    WhileKw,
    #[token("break")]
    BreakKw,
    #[token("continue")]
    ContinueKw,
    #[token("mod")]
    ModKw,
    #[token("as")]
    AsKw,

    #[token("::")]
    ColonColon,
    #[token("<=")]
    LessEq,
    #[token(">=")]
    GreaterEq,
    #[token("==")]
    EqEq,
    #[token("!=")]
    NotEq,
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("++")]
    PlusPlus,
    #[token("--")]
    MinusMinus,

    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token("<")]
    Less,
    #[token(">")]
    Greater,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("!")]
    Bang,
    #[token("&")]
    Amp,
    #[token("|")]
    Pipe,
    #[token("^")]
    Caret,
    #[token("~")]
    Tilde,
    #[token("=")]
    Eq,
    #[token("...")]
    Ellipsis,
    #[token(".")]
    Dot,
    #[token(",")]
    Comma,
    #[token(";")]
    Semicolon,
    #[token(":")]
    Colon,

    #[token("\"", lex_string)]
    String,
    #[token("'", lex_character)]
    Character,
    #[regex(r"0[xX][0-9a-fA-F][0-9a-fA-F_]*(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize)?")]
    #[regex(r"0[bB][01][01_]*(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize)?")]
    #[regex(r"0[oO][0-7][0-7_]*(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize)?")]
    #[regex(r"[0-9][0-9_]*(i8|i16|i32|i64|i128|isize|u8|u16|u32|u64|u128|usize)?")]
    Integer,
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+-]?[0-9][0-9_]*)?f?")]
    #[regex(r"[0-9][0-9_]*[eE][+-]?[0-9][0-9_]*f?")]
    Float,

    #[regex(r"[A-Za-z_][A-Za-z0-9_]*")]
    Identifier,
}

impl From<RawToken> for SyntaxKind {
    fn from(raw: RawToken) -> Self {
        match raw {
            RawToken::Whitespace => Self::Whitespace,
            RawToken::LineComment => Self::LineComment,
            RawToken::BlockComment => Self::BlockComment,
            RawToken::Identifier => Self::Identifier,
            RawToken::Integer => Self::Integer,
            RawToken::Float => Self::Float,
            RawToken::String => Self::String,
            RawToken::Character => Self::Character,
            RawToken::NamespaceKw => Self::NamespaceKw,
            RawToken::UseKw => Self::UseKw,
            RawToken::ReturnKw => Self::ReturnKw,
            RawToken::IfKw => Self::IfKw,
            RawToken::ElseKw => Self::ElseKw,
            RawToken::ForKw => Self::ForKw,
            RawToken::ConstKw => Self::ConstKw,
            RawToken::AutoKw => Self::AutoKw,
            RawToken::TrueKw => Self::TrueKw,
            RawToken::FalseKw => Self::FalseKw,
            RawToken::MoveKw => Self::MoveKw,
            RawToken::StructKw => Self::StructKw,
            RawToken::ClassKw => Self::ClassKw,
            RawToken::InterfaceKw => Self::InterfaceKw,
            RawToken::PublicKw => Self::PublicKw,
            RawToken::PrivateKw => Self::PrivateKw,
            RawToken::ThrowsKw => Self::ThrowsKw,
            RawToken::ThrowKw => Self::ThrowKw,
            RawToken::TryKw => Self::TryKw,
            RawToken::CatchKw => Self::CatchKw,
            RawToken::WhileKw => Self::WhileKw,
            RawToken::BreakKw => Self::BreakKw,
            RawToken::ContinueKw => Self::ContinueKw,
            RawToken::ModKw => Self::ModKw,
            RawToken::AsKw => Self::AsKw,
            RawToken::LParen => Self::LParen,
            RawToken::RParen => Self::RParen,
            RawToken::LBrace => Self::LBrace,
            RawToken::RBrace => Self::RBrace,
            RawToken::LBracket => Self::LBracket,
            RawToken::RBracket => Self::RBracket,
            RawToken::Less => Self::Less,
            RawToken::Greater => Self::Greater,
            RawToken::LessEq => Self::LessEq,
            RawToken::GreaterEq => Self::GreaterEq,
            RawToken::EqEq => Self::EqEq,
            RawToken::NotEq => Self::NotEq,
            RawToken::Plus => Self::Plus,
            RawToken::Minus => Self::Minus,
            RawToken::Star => Self::Star,
            RawToken::Slash => Self::Slash,
            RawToken::Percent => Self::Percent,
            RawToken::Bang => Self::Bang,
            RawToken::Amp => Self::Amp,
            RawToken::Pipe => Self::Pipe,
            RawToken::Caret => Self::Caret,
            RawToken::Tilde => Self::Tilde,
            RawToken::AndAnd => Self::AndAnd,
            RawToken::OrOr => Self::OrOr,
            RawToken::Eq => Self::Eq,
            RawToken::PlusEq => Self::PlusEq,
            RawToken::MinusEq => Self::MinusEq,
            RawToken::StarEq => Self::StarEq,
            RawToken::SlashEq => Self::SlashEq,
            RawToken::PercentEq => Self::PercentEq,
            RawToken::PlusPlus => Self::PlusPlus,
            RawToken::MinusMinus => Self::MinusMinus,
            RawToken::Dot => Self::Dot,
            RawToken::Ellipsis => Self::Ellipsis,
            RawToken::Comma => Self::Comma,
            RawToken::Semicolon => Self::Semicolon,
            RawToken::Colon => Self::Colon,
            RawToken::ColonColon => Self::ColonColon,
        }
    }
}

fn lex_block_comment(lexer: &mut logos::Lexer<'_, RawToken>) {
    let start = lexer.span().start;
    let remainder = lexer.remainder();
    if let Some(offset) = remainder.find("*/") {
        lexer.bump(offset + 2);
    } else {
        lexer.bump(remainder.len());
        lexer.extras.push(LexError {
            message: "unterminated block comment".to_owned(),
            range: text_range(start..lexer.source().len()),
        });
    }
}

fn lex_line_comment(lexer: &mut logos::Lexer<'_, RawToken>) {
    let remainder = lexer.remainder();
    let length = remainder.find(['\r', '\n']).unwrap_or(remainder.len());
    lexer.bump(length);
}

fn lex_string(lexer: &mut logos::Lexer<'_, RawToken>) {
    lex_quoted(lexer, '"', "string");
}

fn lex_character(lexer: &mut logos::Lexer<'_, RawToken>) {
    lex_quoted(lexer, '\'', "character");
}

fn lex_quoted(lexer: &mut logos::Lexer<'_, RawToken>, delimiter: char, description: &str) {
    let start = lexer.span().start;
    let remainder = lexer.remainder();
    let mut escaped = false;

    for (offset, character) in remainder.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == delimiter {
            lexer.bump(offset + character.len_utf8());
            return;
        }
        if matches!(character, '\r' | '\n') {
            lexer.bump(offset);
            lexer.extras.push(LexError {
                message: format!("unterminated {description} literal"),
                range: text_range(start..start + 1 + offset),
            });
            return;
        }
    }

    lexer.bump(remainder.len());
    lexer.extras.push(LexError {
        message: format!("unterminated {description} literal"),
        range: text_range(start..lexer.source().len()),
    });
}

fn text_range(range: std::ops::Range<usize>) -> TextRange {
    TextRange::new(text_size(range.start), text_size(range.end))
}

fn text_size(offset: usize) -> TextSize {
    TextSize::try_from(offset).expect("Stainless source files must be smaller than 4 GiB")
}
