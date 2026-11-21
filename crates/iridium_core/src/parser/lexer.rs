use crate::parser::ast::Token;
use crate::parser::token::Keyword;
use winnow::ascii::float;
use winnow::combinator::{alt, opt, repeat};
use winnow::prelude::*;
use winnow::token::{any, take_while};

pub fn lex(input: &mut &str, quoted_identifier: bool) -> ModalResult<Vec<Token>> {
    repeat(
        0..,
        alt((
            parse_whitespace.map(|_| None),
            parse_comment.map(|_| None),
            parse_binary_literal.map(|hex| Some(Token::BinaryLiteral(hex.to_string()))),
            |input: &mut _| {
                let start = *input;
                parse_number(input).map(|n| {
                    let consumed = &start[..start.len() - input.len()];
                    let is_float =
                        consumed.contains('.') || consumed.contains('e') || consumed.contains('E');
                    Some(Token::Number {
                        value: n,
                        is_float,
                        raw: consumed.to_string(),
                    })
                })
            },
            |input: &mut _| {
                parse_string(input).map(|s| {
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
                    parse_quoted_identifier(i)
                        .map(|s| Some(Token::String(unescape_quoted_identifier(s))))
                } else {
                    Err(winnow::error::ErrMode::Backtrack(
                        winnow::error::ContextError::new(),
                    ))
                }
            },
            parse_variable.map(|v| Some(Token::Variable(v.to_string()))),
            |i: &mut _| {
                if quoted_identifier {
                    parse_quoted_identifier(i)
                        .map(|id| Some(Token::Identifier(unescape_quoted_identifier(id))))
                } else {
                    Err(winnow::error::ErrMode::Backtrack(
                        winnow::error::ContextError::new(),
                    ))
                }
            },
            parse_identifier
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
            parse_bracketed_identifier
                .map(|id| Some(Token::Identifier(unescape_bracketed_identifier(id)))),
            parse_operator_token.map(Some),
            parse_punctuation,
        )),
    )
    .map(|v: Vec<Option<Token>>| v.into_iter().flatten().collect())
    .parse_next(input)
}

fn parse_whitespace<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    take_while(1.., |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '\u{0001}'..='\u{001F}'
                    | '\u{FEFF}'
                    | '\u{200B}'
                    | '\u{200C}'
                    | '\u{200D}'
                    | '\u{2060}'
            )
    })
    .parse_next(input)
}

fn parse_comment(input: &mut &str) -> ModalResult<()> {
    if input.starts_with("--") {
        *input = &input[2..];
        take_while(0.., |c| c != '\n').parse_next(input)?;
        opt('\n').parse_next(input)?;
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

fn parse_number(input: &mut &str) -> ModalResult<f64> {
    let start = *input;
    if start.starts_with('+') || start.starts_with('-') {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    }
    let result: ModalResult<f64> = float.parse_next(input);
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

fn parse_string<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
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
        let _ = take_while(0.., |c| c != '\'').parse_next(input)?;
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
    let len = start.len() - input.len();
    Ok(&start[..len])
}

fn parse_identifier<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    let start = *input;
    let _: char = any
        .verify(|c: &char| c.is_ascii_alphabetic() || *c == '_' || *c == '#' || *c == '@' || *c == '$')
        .parse_next(input)?;
    let _: &str = take_while(0.., |c: char| {
        c.is_ascii_alphanumeric() || c == '_' || c == '#' || c == '$'
    })
    .parse_next(input)?;
    let len = start.len() - input.len();
    Ok(&start[..len])
}

fn parse_bracketed_identifier<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    let start = *input;
    if !input.starts_with('[') {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    }
    *input = &input[1..];
    loop {
        let _ = take_while(0.., |c| c != ']').parse_next(input)?;
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
    let len = start.len() - input.len();
    Ok(&start[..len])
}

fn parse_variable<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    let start = *input;
    let _: &str = alt(("@@", "@")).parse_next(input)?;
    let _: &str =
        take_while(1.., |c: char| c.is_ascii_alphanumeric() || c == '_').parse_next(input)?;
    let len = start.len() - input.len();
    Ok(&start[..len])
}

fn parse_binary_literal<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    let start = *input;
    let _: &str = alt(("0x", "0X")).parse_next(input)?;
    let _: &str = take_while(0.., |c: char| c.is_ascii_hexdigit()).parse_next(input)?;
    let len = start.len() - input.len();
    Ok(&start[..len])
}

fn parse_quoted_identifier<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    let start = *input;
    if !input.starts_with('"') {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    }
    *input = &input[1..];
    loop {
        let _ = take_while(0.., |c| c != '"').parse_next(input)?;
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
    let len = start.len() - input.len();
    Ok(&start[..len])
}

fn parse_operator_token(input: &mut &str) -> ModalResult<Token> {
    alt((
        "~".map(|_| Token::Tilde),
        alt((
            alt(("<=", ">=", "<>", "!=", "::")),
            alt(("=", "<", ">", "+", "-", "*")),
            alt(("/", "%", "&", "|", "^")),
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

fn parse_punctuation(input: &mut &str) -> ModalResult<Option<Token>> {
    alt((
        "(".map(|_| Some(Token::LParen)),
        ")".map(|_| Some(Token::RParen)),
        ",".map(|_| Some(Token::Comma)),
        ";".map(|_| Some(Token::Semicolon)),
        ".".map(|_| Some(Token::Dot)),
    ))
    .parse_next(input)
}

pub fn unescape_string(s: &str) -> String {
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

pub fn unescape_bracketed_identifier(s: &str) -> String {
    let mut s_slice = s;
    if s_slice.starts_with('[') {
        s_slice = &s_slice[1..];
    }
    if s_slice.ends_with(']') {
        s_slice = &s_slice[..s_slice.len() - 1];
    }
    s_slice.replace("]]", "]")
}

pub fn unescape_quoted_identifier(s: &str) -> String {
    let mut s_slice = s;
    if s_slice.starts_with('"') {
        s_slice = &s_slice[1..];
    }
    if s_slice.ends_with('"') {
        s_slice = &s_slice[..s_slice.len() - 1];
    }
    s_slice.replace("\"\"", "\"")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
