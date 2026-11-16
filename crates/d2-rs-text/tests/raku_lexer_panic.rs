use d2_rs_text::code::{CodeWorkError, tokenize_checked};

const PANIC: &str = "lexer panic: runtime error: invalid memory address or nil pointer dereference";

#[test]
fn unterminated_raku_pod_reports_the_recovered_go_lexer_panic() {
    // chroma raku.go: an unmatched `=begin` pod block makes regexp2 return a
    // nil match, which the Go lexer dereferences; d2scenebuild recovers it.
    for (language, source) in [
        ("raku", "  =begin pod\n"),
        (
            "pl6",
            "#!/usr/bin/env raku\n=begin pod\n=head1 Some B<Chroma>\n",
        ),
        ("raku", "=begin pod\nfoo\n=end head1\n"),
        ("raku", "=begin pod\n=begin pod\nx\n=end pod\n"),
        // Markdown fences and HTTP bodies delegate to the same lexer.
        ("mkd", "```raku\n=begin pod\nx\n```\n"),
        (
            "http",
            "POST / HTTP/1.1\nContent-Type: text/x-raku\n\n=begin pod\nx\n",
        ),
    ] {
        let error = tokenize_checked(language, source).unwrap_err();
        assert!(matches!(error, CodeWorkError::LexerPanic(_)), "{source:?}");
        assert_eq!(error.to_string(), PANIC, "{source:?}");
    }
    for source in [
        "say 1;\n=begin pod\nfoo\n=end pod\nsay 2;\n",
        "=for pod\nx\n",
    ] {
        assert!(
            tokenize_checked("raku", source).unwrap().is_some(),
            "{source:?}"
        );
    }
}
