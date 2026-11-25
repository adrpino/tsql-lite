use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

fn decode_hex_literal(raw: &str) -> Vec<u8> {
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    let mut out = Vec::with_capacity(hex.len().div_ceil(2));
    let bytes = hex.as_bytes();
    let mut i = 0;
    if bytes.len() % 2 == 1 {
        if let Ok(b) = u8::from_str_radix(&hex[0..1], 16) {
            out.push(b);
        }
        i = 1;
    }
    while i + 1 < bytes.len() {
        if let Ok(b) = u8::from_str_radix(&hex[i..i + 2], 16) {
            out.push(b);
        }
        i += 2;
    }
    out
}

fn expect_ident_or_kw(parser: &mut Parser, desc: &'static str) -> ParseResult<String> {
    match parser.next() {
        Some(Token::Identifier(id)) => Ok(id.clone()),
        Some(Token::Keyword(kw)) => Ok(kw.as_sql().to_string()),
        _ => parser.backtrack(Expected::Description(desc)),
    }
}

fn expect_eq(parser: &mut Parser) -> ParseResult<()> {
    if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
        let _ = parser.next();
        Ok(())
    } else {
        parser.backtrack(Expected::Description("'='"))
    }
}

fn expect_string_lit(parser: &mut Parser, desc: &'static str) -> ParseResult<String> {
    match parser.next() {
        Some(Token::String(s)) | Some(Token::NString(s)) => Ok(s.clone()),
        _ => parser.backtrack(Expected::Description(desc)),
    }
}

/// Parses `COLUMN MASTER KEY <name> WITH (KEY_STORE_PROVIDER_NAME = ..., KEY_PATH = ... [, ENCLAVE_COMPUTATIONS (SIGNATURE = 0x...)])`
/// or `COLUMN ENCRYPTION KEY <name> WITH VALUES ((COLUMN_MASTER_KEY = ..., ALGORITHM = ..., ENCRYPTED_VALUE = 0x...), ...)`
/// after `CREATE COLUMN` has already been consumed.
pub fn parse_create_column_key(parser: &mut Parser) -> ParseResult<Statement> {
    if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MASTER")) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Key)?;
        let name = expect_ident_or_kw(parser, "column master key name")?;
        parser.expect_keyword(Keyword::With)?;
        parser.expect_lparen()?;

        let mut provider = String::new();
        let mut key_path = String::new();
        let mut allow_enclave = false;
        let mut signature: Option<Vec<u8>> = None;

        while !matches!(parser.peek(), Some(Token::RParen) | None) {
            let opt = expect_ident_or_kw(parser, "CMK option")?;
            if opt.eq_ignore_ascii_case("KEY_STORE_PROVIDER_NAME") {
                expect_eq(parser)?;
                provider = expect_string_lit(parser, "provider name")?;
            } else if opt.eq_ignore_ascii_case("KEY_PATH") {
                expect_eq(parser)?;
                key_path = expect_string_lit(parser, "key path")?;
            } else if opt.eq_ignore_ascii_case("ENCLAVE_COMPUTATIONS") {
                allow_enclave = true;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    while !matches!(parser.peek(), Some(Token::RParen) | None) {
                        if matches!(parser.peek(), Some(Token::Identifier(s)) if s.eq_ignore_ascii_case("SIGNATURE"))
                        {
                            let _ = parser.next();
                            expect_eq(parser)?;
                            if let Some(Token::BinaryLiteral(raw)) = parser.next() {
                                signature = Some(decode_hex_literal(raw));
                            }
                        } else {
                            let _ = parser.next();
                        }
                    }
                    let _ = parser.expect_rparen();
                }
            }
            if matches!(parser.peek(), Some(Token::Comma)) {
                let _ = parser.next();
            } else {
                break;
            }
        }
        parser.expect_rparen()?;

        Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
            Box::new(crate::ast::CatalogMetadataAction::CreateColumnMasterKey {
                name,
                key_store_provider_name: provider,
                key_path,
                allow_enclave_computations: allow_enclave,
                signature,
            }),
        ))))
    } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ENCRYPTION"))
    {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Key)?;
        let name = expect_ident_or_kw(parser, "column encryption key name")?;
        parser.expect_keyword(Keyword::With)?;
        if !parser.at_keyword(Keyword::Values)
            && !matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("VALUES"))
        {
            return parser.backtrack(Expected::Description("VALUES"));
        }
        let _ = parser.next();

        let mut values = Vec::new();
        loop {
            values.push(parse_cek_value_block(parser)?);
            if matches!(parser.peek(), Some(Token::Comma)) {
                let _ = parser.next();
            } else {
                break;
            }
        }

        Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
            Box::new(crate::ast::CatalogMetadataAction::CreateColumnEncryptionKey { name, values }),
        ))))
    } else {
        parser.backtrack(Expected::Description("MASTER or ENCRYPTION"))
    }
}

/// Parses `ALTER COLUMN ENCRYPTION KEY <name> ADD|DROP VALUE (...)` after `ALTER COLUMN` is consumed.
pub fn parse_alter_column_encryption_key(parser: &mut Parser) -> ParseResult<Statement> {
    if !matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ENCRYPTION"))
    {
        return parser.backtrack(Expected::Description("ENCRYPTION"));
    }
    let _ = parser.next();
    parser.expect_keyword(Keyword::Key)?;
    let name = expect_ident_or_kw(parser, "column encryption key name")?;

    let add = if parser.at_keyword(Keyword::Add) {
        let _ = parser.next();
        true
    } else if parser.at_keyword(Keyword::Drop) {
        let _ = parser.next();
        false
    } else {
        return parser.backtrack(Expected::Description("ADD or DROP"));
    };

    if parser.at_keyword(Keyword::Value)
        || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("VALUE"))
    {
        let _ = parser.next();
    } else {
        return parser.backtrack(Expected::Description("VALUE"));
    }

    let val = parse_cek_value_block(parser)?;
    Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
        Box::new(
            crate::ast::CatalogMetadataAction::AlterColumnEncryptionKey {
                name,
                add,
                cmk_name: val.0,
                algorithm: val.1,
                encrypted_value: val.2,
            },
        ),
    ))))
}

/// Parses `DROP COLUMN MASTER|ENCRYPTION KEY <name>` after `DROP COLUMN` is consumed.
pub fn parse_drop_column_key(parser: &mut Parser) -> ParseResult<Statement> {
    if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MASTER")) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Key)?;
        let name = expect_ident_or_kw(parser, "column master key name")?;
        Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
            Box::new(crate::ast::CatalogMetadataAction::DropColumnMasterKey { name }),
        ))))
    } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ENCRYPTION"))
    {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Key)?;
        let name = expect_ident_or_kw(parser, "column encryption key name")?;
        Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
            Box::new(crate::ast::CatalogMetadataAction::DropColumnEncryptionKey { name }),
        ))))
    } else {
        parser.backtrack(Expected::Description("MASTER or ENCRYPTION"))
    }
}

fn parse_cek_value_block(parser: &mut Parser) -> ParseResult<(String, String, Vec<u8>)> {
    parser.expect_lparen()?;
    let mut cmk_name = String::new();
    let mut algorithm = String::new();
    let mut encrypted_value = Vec::new();

    while !matches!(parser.peek(), Some(Token::RParen) | None) {
        let opt = expect_ident_or_kw(parser, "CEK value option")?;
        expect_eq(parser)?;
        if opt.eq_ignore_ascii_case("COLUMN_MASTER_KEY") {
            cmk_name = expect_ident_or_kw(parser, "column master key name")?;
        } else if opt.eq_ignore_ascii_case("ALGORITHM") {
            algorithm = expect_string_lit(parser, "encryption algorithm")?;
        } else if opt.eq_ignore_ascii_case("ENCRYPTED_VALUE") {
            match parser.next() {
                Some(Token::BinaryLiteral(raw)) => encrypted_value = decode_hex_literal(raw),
                Some(Token::String(s)) | Some(Token::NString(s)) => {
                    encrypted_value = s.as_bytes().to_vec();
                }
                _ => return parser.backtrack(Expected::Description("hex literal")),
            }
        }
        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
        } else {
            break;
        }
    }
    parser.expect_rparen()?;
    Ok((cmk_name, algorithm, encrypted_value))
}
