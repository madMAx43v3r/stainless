use stainless_syntax::{SyntaxKind, lex};

#[test]
fn lexer_preserves_every_source_byte_including_trivia() {
    let source = "for (const auto& value : values) { /* keep */ value += 1; }\n";
    let lexed = lex(source);

    assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
    let reconstructed = lexed
        .tokens
        .iter()
        .map(|token| &source[usize::from(token.range.start())..usize::from(token.range.end())])
        .collect::<String>();
    assert_eq!(reconstructed, source);
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::ForKw)
    );
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::BlockComment)
    );
}

#[test]
fn lexer_reports_invalid_and_unterminated_input_without_dropping_it() {
    let source = "i32 value = @;\nString text = \"unfinished";
    let lexed = lex(source);

    assert_eq!(lexed.errors.len(), 2);
    assert!(
        lexed
            .errors
            .iter()
            .any(|error| error.message.contains("unexpected character"))
    );
    assert!(
        lexed
            .errors
            .iter()
            .any(|error| error.message.contains("unterminated string"))
    );
    let covered_bytes = lexed
        .tokens
        .iter()
        .map(|token| usize::from(token.range.len()))
        .sum::<usize>();
    assert_eq!(covered_bytes, source.len());
}

#[test]
fn lexer_recognizes_stainless_numeric_and_character_spellings() {
    let source = "3.0 3.0f 42u64 0xffu8 'x' '🦀'";
    let lexed = lex(source);
    let significant = lexed
        .tokens
        .iter()
        .filter(|token| !token.kind.is_trivia())
        .map(|token| token.kind)
        .collect::<Vec<_>>();

    assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
    assert_eq!(
        significant,
        [
            SyntaxKind::Float,
            SyntaxKind::Float,
            SyntaxKind::Integer,
            SyntaxKind::Integer,
            SyntaxKind::Character,
            SyntaxKind::Character,
        ]
    );
}

#[test]
fn lexer_recognizes_the_lambda_mutable_keyword() {
    let lexed = lex("[count = initial]() mutable { count += 1; }");

    assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::MutableKw)
    );
}

#[test]
fn lexer_reserves_json_null() {
    let lexed = lex("var value = null;");

    assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
    assert!(
        lexed
            .tokens
            .iter()
            .any(|token| token.kind == SyntaxKind::NullKw)
    );
}

#[test]
fn lexer_recognizes_switch_arms() {
    let lexed = lex("switch (value) { \"open\" | \"close\" => 10, else => 20 }");

    assert!(lexed.errors.is_empty(), "{:?}", lexed.errors);
    let significant = lexed
        .tokens
        .iter()
        .filter(|token| !token.kind.is_trivia())
        .map(|token| token.kind)
        .collect::<Vec<_>>();
    assert_eq!(significant[0], SyntaxKind::SwitchKw);
    assert!(significant.contains(&SyntaxKind::FatArrow));
    assert!(significant.contains(&SyntaxKind::Pipe));
    assert!(significant.contains(&SyntaxKind::String));
    assert!(significant.contains(&SyntaxKind::ElseKw));
}
