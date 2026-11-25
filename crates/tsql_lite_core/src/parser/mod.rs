pub mod ast;
pub(crate) mod error;
pub(crate) mod lexer;
pub(crate) mod lower;
pub(crate) mod parse;
pub(crate) mod state;
pub(crate) mod token;

use crate::ast::Statement;
use crate::error::DbError;
use crate::parser::parse as inner_parse;
use crate::parser::state::Parser;

pub mod utils {
    use super::{ast, lexer};

    pub fn split_csv_top_level(input: &str) -> Vec<String> {
        let mut sql_ref = input;
        let tokens = match lexer::lex(&mut sql_ref, true) {
            Ok(t) => t,
            Err(_) => return vec![input.to_string()],
        };

        let mut result = Vec::new();
        let mut depth: i32 = 0;

        let mut current_part_tokens = Vec::new();
        for tok in tokens {
            match tok {
                ast::Token::LParen => {
                    depth += 1;
                    current_part_tokens.push(tok);
                }
                ast::Token::RParen => {
                    depth = depth.saturating_sub(1);
                    current_part_tokens.push(tok);
                }
                ast::Token::Comma if depth == 0 => {
                    result.push(format_tokens(&current_part_tokens));
                    current_part_tokens.clear();
                }
                _ => {
                    current_part_tokens.push(tok);
                }
            }
        }
        if !current_part_tokens.is_empty() {
            result.push(format_tokens(&current_part_tokens));
        }
        result
    }

    fn format_tokens(tokens: &[ast::Token]) -> String {
        tokens
            .iter()
            .map(|t| match t {
                ast::Token::String(s) => format!("'{}'", s.replace("'", "''")),
                ast::Token::Identifier(id) => format!("[{}]", id),
                ast::Token::Variable(v) => v.to_string(),
                ast::Token::Keyword(k) => k.to_string(),
                ast::Token::Operator(op) => op.to_string(),
                ast::Token::Number { value: n, .. } => n.to_string(),
                ast::Token::LParen => "(".to_string(),
                ast::Token::RParen => ")".to_string(),
                ast::Token::Comma => ",".to_string(),
                ast::Token::Dot => ".".to_string(),
                ast::Token::Semicolon => ";".to_string(),
                ast::Token::Star => "*".to_string(),
                _ => "".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub(crate) fn validate_identifier_lengths(tokens: &[ast::Token]) -> Result<(), DbError> {
    for tok in tokens {
        let (ident, max_len) = match tok {
            ast::Token::Identifier(id) => {
                // SQL Server limits local temp table identifiers (#...) to 116 chars;
                // regular, bracketed, double-quoted, and global temp (##...) identifiers allow 128.
                if id.starts_with('#') && !id.starts_with("##") {
                    (id.as_str(), 116usize)
                } else {
                    (id.as_str(), 128usize)
                }
            }
            ast::Token::Variable(v) => (v.as_str(), 128usize),
            _ => continue,
        };
        if ident.chars().count() > max_len {
            let prefix: String = ident.chars().take(max_len).collect();
            return Err(DbError::Custom {
                class: 15,
                number: 103,
                message: format!(
                    "The identifier that starts with '{}' is too long. Maximum length is {}.",
                    prefix, max_len
                ),
            });
        }
    }
    Ok(())
}

pub mod statements {
    pub mod procedural {
        use crate::error::DbError;
        use crate::parser::{lexer, lower, parse as inner_parse, state::Parser};

        pub fn parse_routine_params(input: &str) -> Result<Vec<crate::ast::RoutineParam>, DbError> {
            let mut sql_ref = input;
            let tokens = lexer::lex(&mut sql_ref, true)
                .map_err(|e| DbError::Parse(format!("Lexer error: {:?}", e)))?;
            crate::parser::validate_identifier_lengths(&tokens)?;
            let mut parser = Parser::new(tokens);
            let params =
                inner_parse::parse_comma_list(&mut parser, inner_parse::parse_routine_param)
                    .map_err(|e| DbError::Parse(e.to_string()))?;
            params.into_iter().map(lower::lower_routine_param).collect()
        }
    }
}

pub fn parse_expr(input: &str) -> Result<crate::ast::Expr, DbError> {
    parse_expr_with_quoted_ident(input, true)
}

pub fn parse_expr_with_quoted_ident(
    input: &str,
    quoted_identifier: bool,
) -> Result<crate::ast::Expr, DbError> {
    let mut sql_ref = input;
    let tokens = lexer::lex(&mut sql_ref, quoted_identifier)
        .map_err(|e| DbError::Parse(format!("Lexer error: {:?}", e)))?;
    validate_identifier_lengths(&tokens)?;
    let mut parser = Parser::new(tokens);
    let expr = inner_parse::expressions::parse_expr(&mut parser)
        .map_err(|e| DbError::Parse(e.to_string()))?;
    lower::lower_expr(expr)
}

pub fn parse_expr_subquery_aware(input: &str) -> Result<crate::ast::Expr, DbError> {
    parse_expr_subquery_aware_with_quoted_ident(input, true)
}

pub fn parse_expr_subquery_aware_with_quoted_ident(
    input: &str,
    quoted_identifier: bool,
) -> Result<crate::ast::Expr, DbError> {
    parse_expr_with_quoted_ident(input, quoted_identifier)
}

pub fn parse_batch(sql: &str) -> Result<Vec<Statement>, DbError> {
    parse_batch_with_quoted_ident(sql, true)
}

/// Preserves verbatim SQL module text (including leading/inline/trailing comments and whitespace),
/// normalizing only a leading `ALTER` keyword to `CREATE` to match SQL Server's `sys.sql_modules`
/// / `OBJECT_DEFINITION()` storage behavior.
fn normalize_module_definition_sql(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        // Skip ASCII whitespace & BOM/zero-width
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Skip -- line comments
        if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip /* ... */ nested block comments
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth = 1usize;
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
            continue;
        }
        break;
    }
    if i + 5 <= len
        && sql.is_char_boundary(i)
        && sql.is_char_boundary(i + 5)
        && sql[i..i + 5].eq_ignore_ascii_case("ALTER")
        && (i + 5 == len || !bytes[i + 5].is_ascii_alphanumeric() && bytes[i + 5] != b'_')
    {
        let mut out = String::with_capacity(sql.len() + 1);
        out.push_str(&sql[..i]);
        out.push_str("CREATE");
        out.push_str(&sql[i + 5..]);
        return out;
    }
    sql.to_string()
}

pub fn parse_batch_with_quoted_ident(
    sql: &str,
    quoted_identifier: bool,
) -> Result<Vec<Statement>, DbError> {
    let mut sql_ref = sql;
    let tokens = lexer::lex(&mut sql_ref, quoted_identifier)
        .map_err(|e| DbError::Parse(format!("Lexer error: {:?}", e)))?;
    validate_identifier_lengths(&tokens)?;
    let mut parser = Parser::new(tokens);
    let stmts = inner_parse::parse_batch(&mut parser).map_err(|e| DbError::Parse(e.to_string()))?;
    let mut lowered = lower::lower_batch(stmts)?;
    if lowered.len() == 1 {
        if let Some(Statement::Procedural(proc_stmt)) = lowered.first_mut() {
            match proc_stmt {
                crate::ast::ProceduralStatement::CreateView(v) => {
                    v.definition_sql = Some(normalize_module_definition_sql(sql));
                }
                crate::ast::ProceduralStatement::CreateProcedure(p) => {
                    p.definition_sql = Some(normalize_module_definition_sql(sql));
                }
                crate::ast::ProceduralStatement::CreateFunction(f) => {
                    f.definition_sql = Some(normalize_module_definition_sql(sql));
                }
                crate::ast::ProceduralStatement::CreateTrigger(t) => {
                    t.definition_sql = Some(normalize_module_definition_sql(sql));
                }
                _ => {}
            }
        }
    }
    Ok(lowered)
}

pub fn parse_sql(sql: &str) -> Result<Statement, DbError> {
    parse_sql_with_quoted_ident(sql, true)
}

pub fn parse_sql_with_quoted_ident(
    sql: &str,
    quoted_identifier: bool,
) -> Result<Statement, DbError> {
    let stmts = parse_batch_with_quoted_ident(sql, quoted_identifier)?;
    if stmts.is_empty() {
        return Err(DbError::Parse("Expected at least one statement".into()));
    }
    // We take the first one, or should we error if more than one?
    // Old parser seems to have error if stmts.len() != 1 in some places.
    if stmts.len() != 1 {
        return Err(DbError::Parse("Expected exactly one statement".into()));
    }
    if let Some(stmt) = stmts.into_iter().next() {
        Ok(stmt)
    } else {
        Err(DbError::Parse("Expected exactly one statement".into()))
    }
}
