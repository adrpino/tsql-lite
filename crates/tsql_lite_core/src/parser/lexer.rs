use crate::parser::ast::Token;
use crate::parser::token::Keyword;

/// Direct byte-dispatch T-SQL lexer (zero combinator backtracking).
pub fn lex(input: &mut &str, quoted_identifier: bool) -> Result<Vec<Token>, &'static str> {
    let s = *input;
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut pos = 0;
    let mut tokens = Vec::with_capacity((len / 5).max(8));

    while pos < len {
        let b = bytes[pos];
        match b {
            // ASCII whitespace and control chars 0x01..=0x1F (excluding NUL 0x00)
            0x01..=b' ' => {
                pos += 1;
                while pos < len {
                    let next = bytes[pos];
                    if next != 0 && next <= b' ' {
                        pos += 1;
                    } else {
                        break;
                    }
                }
            }
            // Single-byte punctuation & unconditional operators
            b'(' => {
                tokens.push(Token::LParen);
                pos += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                pos += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                pos += 1;
            }
            b';' => {
                tokens.push(Token::Semicolon);
                pos += 1;
            }
            b'~' => {
                tokens.push(Token::Tilde);
                pos += 1;
            }
            b'*' => {
                tokens.push(Token::Star);
                pos += 1;
            }
            b'=' => {
                tokens.push(Token::Operator("=".to_string()));
                pos += 1;
            }
            b'+' => {
                tokens.push(Token::Operator("+".to_string()));
                pos += 1;
            }
            b'%' => {
                tokens.push(Token::Operator("%".to_string()));
                pos += 1;
            }
            b'&' => {
                tokens.push(Token::Operator("&".to_string()));
                pos += 1;
            }
            b'|' => {
                tokens.push(Token::Operator("|".to_string()));
                pos += 1;
            }
            b'^' => {
                tokens.push(Token::Operator("^".to_string()));
                pos += 1;
            }
            // '-' operator or '--' line comment
            b'-' => {
                if pos + 1 < len && bytes[pos + 1] == b'-' {
                    pos += 2;
                    while pos < len && bytes[pos] != b'\n' {
                        pos += 1;
                    }
                    if pos < len {
                        pos += 1;
                    }
                } else {
                    tokens.push(Token::Operator("-".to_string()));
                    pos += 1;
                }
            }
            // '/' operator or '/* ... */' nested block comment
            b'/' => {
                if pos + 1 < len && bytes[pos + 1] == b'*' {
                    let mut i = pos + 2;
                    let mut depth: usize = 1;
                    while i + 1 < len && depth > 0 {
                        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                            depth += 1;
                            i += 2;
                        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            depth -= 1;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                    if depth == 0 {
                        pos = i;
                    } else {
                        // Unterminated block comment falls back to '/' operator
                        tokens.push(Token::Operator("/".to_string()));
                        pos += 1;
                    }
                } else {
                    tokens.push(Token::Operator("/".to_string()));
                    pos += 1;
                }
            }
            // '<', '<=', '<>'
            b'<' => {
                if pos + 1 < len && bytes[pos + 1] == b'=' {
                    tokens.push(Token::Operator("<=".to_string()));
                    pos += 2;
                } else if pos + 1 < len && bytes[pos + 1] == b'>' {
                    tokens.push(Token::Operator("<>".to_string()));
                    pos += 2;
                } else {
                    tokens.push(Token::Operator("<".to_string()));
                    pos += 1;
                }
            }
            // '>', '>='
            b'>' => {
                if pos + 1 < len && bytes[pos + 1] == b'=' {
                    tokens.push(Token::Operator(">=".to_string()));
                    pos += 2;
                } else {
                    tokens.push(Token::Operator(">".to_string()));
                    pos += 1;
                }
            }
            // '!='
            b'!' => {
                if pos + 1 < len && bytes[pos + 1] == b'=' {
                    tokens.push(Token::Operator("!=".to_string()));
                    pos += 2;
                } else {
                    break;
                }
            }
            // '::' or ':'
            b':' => {
                if pos + 1 < len && bytes[pos + 1] == b':' {
                    tokens.push(Token::Operator("::".to_string()));
                    pos += 2;
                } else {
                    tokens.push(Token::Colon);
                    pos += 1;
                }
            }
            // '.' or leading-dot numeric literal like '.5'
            b'.' => {
                if pos + 1 < len && bytes[pos + 1].is_ascii_digit() {
                    if let Some((tok, next_pos)) = scan_number(s, bytes, pos) {
                        tokens.push(tok);
                        pos = next_pos;
                    } else {
                        // Overflowing float (e.g. .5e999) falls back to Dot
                        tokens.push(Token::Dot);
                        pos += 1;
                    }
                } else {
                    tokens.push(Token::Dot);
                    pos += 1;
                }
            }
            // Binary literal (0x/0X) or numeric literal
            b'0'..=b'9' => {
                if b == b'0' && pos + 1 < len && (bytes[pos + 1] == b'x' || bytes[pos + 1] == b'X')
                {
                    let start = pos;
                    pos += 2;
                    while pos < len && bytes[pos].is_ascii_hexdigit() {
                        pos += 1;
                    }
                    tokens.push(Token::BinaryLiteral(s[start..pos].to_string()));
                } else if let Some((tok, next_pos)) = scan_number(s, bytes, pos) {
                    tokens.push(tok);
                    pos = next_pos;
                } else {
                    break;
                }
            }
            // Single-quoted string literal
            b'\'' => {
                if let Some((content, next_pos)) = scan_delimited(s, bytes, pos, b'\'') {
                    tokens.push(Token::String(content));
                    pos = next_pos;
                } else {
                    break;
                }
            }
            // Double-quoted identifier or string
            b'"' => {
                if let Some((content, next_pos)) = scan_delimited(s, bytes, pos, b'"') {
                    if quoted_identifier {
                        tokens.push(Token::Identifier(content));
                    } else {
                        tokens.push(Token::String(content));
                    }
                    pos = next_pos;
                } else {
                    break;
                }
            }
            // Bracketed identifier [...]
            b'[' => {
                if let Some((content, next_pos)) = scan_delimited(s, bytes, pos, b']') {
                    tokens.push(Token::Identifier(content));
                    pos = next_pos;
                } else {
                    break;
                }
            }
            // Variable (@var / @@sysvar) or fallback identifier starting with '@'
            b'@' => {
                let prefix_len = if pos + 1 < len && bytes[pos + 1] == b'@' {
                    2
                } else {
                    1
                };
                if pos + prefix_len < len && is_var_char(bytes[pos + prefix_len]) {
                    let start = pos;
                    pos += prefix_len + 1;
                    while pos < len && is_var_char(bytes[pos]) {
                        pos += 1;
                    }
                    tokens.push(Token::Variable(s[start..pos].to_string()));
                } else {
                    let start = pos;
                    pos += 1;
                    while pos < len && is_ident_cont(bytes[pos]) {
                        pos += 1;
                    }
                    tokens.push(Token::Identifier(s[start..pos].to_string()));
                }
            }
            // Identifiers, keywords, GO, and N'...' Unicode string literals
            b'A'..=b'Z' | b'a'..=b'z' | b'_' | b'#' | b'$' => {
                if b == b'N' && pos + 1 < len && bytes[pos + 1] == b'\'' {
                    if let Some((content, next_pos)) = scan_delimited(s, bytes, pos + 1, b'\'') {
                        tokens.push(Token::NString(content));
                        pos = next_pos;
                        continue;
                    }
                }
                let start = pos;
                pos += 1;
                while pos < len && is_ident_cont(bytes[pos]) {
                    pos += 1;
                }
                let id = &s[start..pos];
                if id.eq_ignore_ascii_case("GO") {
                    tokens.push(Token::Go);
                } else if let Some(kw) = Keyword::parse(id) {
                    tokens.push(Token::Keyword(kw));
                } else {
                    tokens.push(Token::Identifier(id.to_string()));
                }
            }
            // Non-ASCII bytes: Unicode whitespace and zero-width formatting characters
            0x80..=0xFF => {
                let ch = s[pos..].chars().next().unwrap();
                if is_unicode_ws(ch) {
                    pos += ch.len_utf8();
                } else {
                    break;
                }
            }
            // Unrecognized byte stops lexing (leaving *input pointing at it)
            _ => break,
        }
    }

    *input = &s[pos..];
    Ok(tokens)
}

#[inline]
fn is_var_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[inline]
fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'#' || b == b'$'
}

#[inline]
fn is_unicode_ws(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '\u{FEFF}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}'
        )
}

#[inline]
fn scan_number(s: &str, bytes: &[u8], start: usize) -> Option<(Token, usize)> {
    let len = bytes.len();
    let mut i = start;
    let mut is_float = false;

    if bytes[i] == b'.' {
        is_float = true;
        i += 2; // '.' and first digit already verified
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        i += 1;
        while i < len && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i < len && bytes[i] == b'.' {
            is_float = true;
            i += 1;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }

    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
        let mut j = i + 1;
        if j < len && (bytes[j] == b'+' || bytes[j] == b'-') {
            j += 1;
        }
        if j < len && bytes[j].is_ascii_digit() {
            is_float = true;
            j += 1;
            while j < len && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        } else {
            return None;
        }
    }

    let raw = &s[start..i];
    let value = if !is_float && raw.len() <= 15 {
        let mut acc: u64 = 0;
        for &d in &bytes[start..i] {
            acc = acc * 10 + (d - b'0') as u64;
        }
        acc as f64
    } else {
        let parsed = raw.parse::<f64>().ok()?;
        if !parsed.is_finite() {
            return None;
        }
        parsed
    };

    Some((
        Token::Number {
            value,
            is_float,
            raw: raw.to_string(),
        },
        i,
    ))
}

#[inline]
fn scan_delimited(
    s: &str,
    bytes: &[u8],
    open_pos: usize,
    close_byte: u8,
) -> Option<(String, usize)> {
    let len = bytes.len();
    let inner_start = open_pos + 1;
    let mut i = inner_start;
    let mut has_escape = false;

    while i < len {
        if bytes[i] == close_byte {
            if i + 1 < len && bytes[i + 1] == close_byte {
                has_escape = true;
                i += 2;
                continue;
            }
            let content = if !has_escape {
                s[inner_start..i].to_string()
            } else {
                let mut out = String::with_capacity(i - inner_start);
                let mut seg_start = inner_start;
                let mut k = inner_start;
                while k < i {
                    if bytes[k] == close_byte {
                        // Include one copy of close_byte and skip the escaped second copy
                        out.push_str(&s[seg_start..=k]);
                        k += 2;
                        seg_start = k;
                    } else {
                        k += 1;
                    }
                }
                if seg_start < i {
                    out.push_str(&s[seg_start..i]);
                }
                out
            };
            return Some((content, i + 1));
        }
        i += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use winnow::ascii::float as ref_float;
    use winnow::combinator::{alt as ref_alt, opt as ref_opt, repeat as ref_repeat};
    use winnow::prelude::*;
    use winnow::token::{any as ref_any, take_while as ref_take_while};

    fn unescape_string(s: &str) -> String {
        let mut s_slice = s;
        if s_slice.starts_with('N') {
            s_slice = &s_slice[1..];
        }
        if s_slice.starts_with('\'') {
            s_slice = &s_slice[1..];
        }
        if s_slice.ends_with('\'') {
            s_slice = &s_slice[..s_slice.len() - 1];
        }
        s_slice.replace("''", "'")
    }

    fn unescape_bracketed_identifier(s: &str) -> String {
        let mut s_slice = s;
        if s_slice.starts_with('[') {
            s_slice = &s_slice[1..];
        }
        if s_slice.ends_with(']') {
            s_slice = &s_slice[..s_slice.len() - 1];
        }
        s_slice.replace("]]", "]")
    }

    fn unescape_quoted_identifier(s: &str) -> String {
        let mut s_slice = s;
        if s_slice.starts_with('"') {
            s_slice = &s_slice[1..];
        }
        if s_slice.ends_with('"') {
            s_slice = &s_slice[..s_slice.len() - 1];
        }
        s_slice.replace("\"\"", "\"")
    }

    /// Pre-optimization winnow combinator lexer preserved as a differential oracle.
    fn reference_lex(input: &mut &str, quoted_identifier: bool) -> Vec<Token> {
        fn ref_ws<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            ref_take_while(1.., |c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '\u{0001}'
                            ..='\u{001F}'
                                | '\u{FEFF}'
                                | '\u{200B}'
                                | '\u{200C}'
                                | '\u{200D}'
                                | '\u{2060}'
                    )
            })
            .parse_next(input)
        }
        fn ref_comment(input: &mut &str) -> ModalResult<()> {
            if input.starts_with("--") {
                *input = &input[2..];
                ref_take_while(0.., |c| c != '\n').parse_next(input)?;
                ref_opt('\n').parse_next(input)?;
                return Ok(());
            }
            if input.starts_with("/*") {
                let bytes = input.as_bytes();
                let mut i = 2;
                let mut depth: usize = 1;
                while i + 1 < bytes.len() && depth > 0 {
                    if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if depth == 0 {
                    *input = &input[i..];
                    return Ok(());
                }
            }
            Err(winnow::error::ErrMode::Backtrack(
                winnow::error::ContextError::new(),
            ))
        }
        fn ref_num(input: &mut &str) -> ModalResult<f64> {
            let start = *input;
            if start.starts_with('+') || start.starts_with('-') {
                return Err(winnow::error::ErrMode::Backtrack(
                    winnow::error::ContextError::new(),
                ));
            }
            let result: ModalResult<f64> = ref_float.parse_next(input);
            if let Ok(val) = result {
                if val.is_infinite() || val.is_nan() {
                    *input = start;
                    return Err(winnow::error::ErrMode::Backtrack(
                        winnow::error::ContextError::new(),
                    ));
                }
            }
            result
        }
        fn ref_str<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            let start = *input;
            if input.starts_with('N') {
                *input = &input[1..];
            }
            if !input.starts_with('\'') {
                return Err(winnow::error::ErrMode::Backtrack(
                    winnow::error::ContextError::new(),
                ));
            }
            *input = &input[1..];
            loop {
                let _ = ref_take_while(0.., |c| c != '\'').parse_next(input)?;
                if input.starts_with('\'') {
                    *input = &input[1..];
                    if input.starts_with('\'') {
                        *input = &input[1..];
                        continue;
                    } else {
                        break;
                    }
                } else {
                    return Err(winnow::error::ErrMode::Backtrack(
                        winnow::error::ContextError::new(),
                    ));
                }
            }
            Ok(&start[..start.len() - input.len()])
        }
        fn ref_ident<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            let start = *input;
            let _: char = ref_any
                .verify(|c: &char| {
                    c.is_ascii_alphabetic() || *c == '_' || *c == '#' || *c == '@' || *c == '$'
                })
                .parse_next(input)?;
            let _: &str = ref_take_while(0.., |c: char| {
                c.is_ascii_alphanumeric() || c == '_' || c == '#' || c == '$'
            })
            .parse_next(input)?;
            Ok(&start[..start.len() - input.len()])
        }
        fn ref_bracket<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            let start = *input;
            if !input.starts_with('[') {
                return Err(winnow::error::ErrMode::Backtrack(
                    winnow::error::ContextError::new(),
                ));
            }
            *input = &input[1..];
            loop {
                let _ = ref_take_while(0.., |c| c != ']').parse_next(input)?;
                if input.starts_with(']') {
                    *input = &input[1..];
                    if input.starts_with(']') {
                        *input = &input[1..];
                        continue;
                    } else {
                        break;
                    }
                } else {
                    return Err(winnow::error::ErrMode::Backtrack(
                        winnow::error::ContextError::new(),
                    ));
                }
            }
            Ok(&start[..start.len() - input.len()])
        }
        fn ref_var<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            let start = *input;
            let _: &str = ref_alt(("@@", "@")).parse_next(input)?;
            let _: &str = ref_take_while(1.., |c: char| c.is_ascii_alphanumeric() || c == '_')
                .parse_next(input)?;
            Ok(&start[..start.len() - input.len()])
        }
        fn ref_bin<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            let start = *input;
            let _: &str = ref_alt(("0x", "0X")).parse_next(input)?;
            let _: &str = ref_take_while(0.., |c: char| c.is_ascii_hexdigit()).parse_next(input)?;
            Ok(&start[..start.len() - input.len()])
        }
        fn ref_quoted<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
            let start = *input;
            if !input.starts_with('"') {
                return Err(winnow::error::ErrMode::Backtrack(
                    winnow::error::ContextError::new(),
                ));
            }
            *input = &input[1..];
            loop {
                let _ = ref_take_while(0.., |c| c != '"').parse_next(input)?;
                if input.starts_with('"') {
                    *input = &input[1..];
                    if input.starts_with('"') {
                        *input = &input[1..];
                        continue;
                    } else {
                        break;
                    }
                } else {
                    return Err(winnow::error::ErrMode::Backtrack(
                        winnow::error::ContextError::new(),
                    ));
                }
            }
            Ok(&start[..start.len() - input.len()])
        }
        fn ref_op(input: &mut &str) -> ModalResult<Token> {
            ref_alt((
                "~".map(|_| Token::Tilde),
                ref_alt((
                    ref_alt(("<=", ">=", "<>", "!=", "::")),
                    ref_alt(("=", "<", ">", "+", "-", "*")),
                    ref_alt(("/", "%", "&", "|", "^")),
                ))
                .map(|op: &str| {
                    if op == "*" {
                        Token::Star
                    } else {
                        Token::Operator(op.to_string())
                    }
                }),
            ))
            .parse_next(input)
        }
        fn ref_punct(input: &mut &str) -> ModalResult<Option<Token>> {
            ref_alt((
                "(".map(|_| Some(Token::LParen)),
                ")".map(|_| Some(Token::RParen)),
                ",".map(|_| Some(Token::Comma)),
                ";".map(|_| Some(Token::Semicolon)),
                ":".map(|_| Some(Token::Colon)),
                ".".map(|_| Some(Token::Dot)),
            ))
            .parse_next(input)
        }

        ref_repeat(
            0..,
            ref_alt((
                ref_ws.map(|_| None),
                ref_comment.map(|_| None),
                ref_bin.map(|hex| Some(Token::BinaryLiteral(hex.to_string()))),
                |input: &mut _| {
                    let start = *input;
                    ref_num(input).map(|n| {
                        let consumed = &start[..start.len() - input.len()];
                        let is_float = consumed.contains('.')
                            || consumed.contains('e')
                            || consumed.contains('E');
                        Some(Token::Number {
                            value: n,
                            is_float,
                            raw: consumed.to_string(),
                        })
                    })
                },
                |input: &mut _| {
                    ref_str(input).map(|s| {
                        let is_n = s.starts_with('N');
                        let unescaped = unescape_string(s);
                        Some(if is_n {
                            Token::NString(unescaped)
                        } else {
                            Token::String(unescaped)
                        })
                    })
                },
                |i: &mut _| {
                    if !quoted_identifier {
                        ref_quoted(i).map(|s| Some(Token::String(unescape_quoted_identifier(s))))
                    } else {
                        Err(winnow::error::ErrMode::Backtrack(
                            winnow::error::ContextError::new(),
                        ))
                    }
                },
                ref_var.map(|v| Some(Token::Variable(v.to_string()))),
                |i: &mut _| {
                    if quoted_identifier {
                        ref_quoted(i)
                            .map(|id| Some(Token::Identifier(unescape_quoted_identifier(id))))
                    } else {
                        Err(winnow::error::ErrMode::Backtrack(
                            winnow::error::ContextError::new(),
                        ))
                    }
                },
                ref_ident
                    .map(|id| {
                        if id.eq_ignore_ascii_case("GO") {
                            Token::Go
                        } else if let Some(kw) = Keyword::parse(id) {
                            Token::Keyword(kw)
                        } else {
                            Token::Identifier(id.to_string())
                        }
                    })
                    .map(Some),
                ref_bracket.map(|id| Some(Token::Identifier(unescape_bracketed_identifier(id)))),
                ref_op.map(Some),
                ref_punct,
            )),
        )
        .map(|v: Vec<Option<Token>>| v.into_iter().flatten().collect())
        .parse_next(input)
        .expect("reference_lex should not fail")
    }

    fn assert_matches_reference(sql: &str) {
        for quoted_identifier in [true, false] {
            let mut ref_in = sql;
            let expected = reference_lex(&mut ref_in, quoted_identifier);
            let mut got_in = sql;
            let actual = lex(&mut got_in, quoted_identifier).expect("lex should succeed");
            assert_eq!(
                actual, expected,
                "token mismatch for quoted_identifier={quoted_identifier}, input={sql:?}"
            );
            assert_eq!(
                got_in, ref_in,
                "remaining slice mismatch for quoted_identifier={quoted_identifier}, input={sql:?}"
            );
        }
    }

    #[test]
    fn test_tsql_line_comments_without_trailing_whitespace() {
        let mut input = "--no-space comment\n---separator\n--===banner===\nSELECT 1";
        let tokens = lex(&mut input, true).expect("lex should succeed");
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[0], Token::Keyword(Keyword::Select)));
    }

    #[test]
    fn test_tsql_nested_block_comments() {
        let mut input = "/* outer /* inner */ still outer */ SELECT 1";
        let tokens = lex(&mut input, true).expect("lex should succeed");
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[0], Token::Keyword(Keyword::Select)));
    }

    #[test]
    fn test_lexer_differential_crafted_edge_cases() {
        let cases = [
            "",
            "   \t\r\n\x01\x1F\u{FEFF}\u{200B}\u{200C}\u{200D}\u{2060}\u{00A0}",
            "\0SELECT 1",
            "SELECT\0 1",
            "-- line comment at EOF",
            "-- line comment\nSELECT 1",
            "/* nested /* block */ comment */ SELECT 1",
            "/* unterminated block comment SELECT 1",
            "/* nested /* unterminated */ SELECT 1",
            "0x 0X 0x01aBcDef99 0xZZ 0X_1",
            "0 123 123. 123.45 .5 .5e2 .5e-2 .5E+3 123e 123e+ 123e- 123e+5 123.foo 1e999 1e-999",
            "inf infinity NaN INF NAN -5 +5 -.5 +.5",
            "'hello' '' 'it''s' N'unicode''text' n'lower' N'unterminated 'unterminated",
            "\"quoted\" \"a\"\"b\" \"\" \"unterminated",
            "[dbo].[My]]Table] [] [unterminated",
            "@a @@ROWCOUNT @1 @@1 @ @@ @$a @#a @foo$bar @foo#bar #temp ##global $money _id",
            "GO go Go gO SELECT current_timestamp Current_Timestamp NonKeyword123",
            "<= >= <> != :: = < > + - * / % & | ^ ~ ! : ( ) , ; . .. .dbo",
            "EXEC dbo.usp_Run @x += 1, @y = N'café', @z = [a]]b];\nGO\nSELECT OBJECT::dbo.T;",
            "SELECT 'é' + N'日本語' FROM [t] WHERE a <> b AND c != d AND e <= f AND g >= h",
            "SELECT 1é2",
            "/*é*/ SELECT /*/*é*/*/ 1",
        ];
        for case in cases {
            assert_matches_reference(case);
        }
    }

    #[test]
    fn test_lexer_differential_random_fuzz() {
        const ALPHABET: &[char] = &[
            ' ', '\t', '\n', '\r', '\x01', '\0', '-', '/', '*', '0', '9', 'x', 'X', 'a', 'f', 'e',
            'E', '+', '.', 'N', 'n', '\'', '"', '[', ']', '@', '_', '#', '$', '<', '>', '=', '!',
            ':', '~', '%', '&', '|', '^', '(', ')', ',', ';', 'g', 'o', 'i', 's', '\u{200B}',
            '\u{00A0}', 'é',
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next_u64 = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut buf = String::with_capacity(64);
        for _ in 0..20_000 {
            buf.clear();
            let len = (next_u64() as usize) % 36;
            for _ in 0..len {
                let idx = (next_u64() as usize) % ALPHABET.len();
                buf.push(ALPHABET[idx]);
            }
            assert_matches_reference(&buf);
        }
    }
}
