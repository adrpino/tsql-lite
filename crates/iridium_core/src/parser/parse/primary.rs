use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

use super::common::parse_comma_list;
use super::data_types::parse_data_type;
use super::parse_expr;
use super::window::parse_window_over;

pub fn parse_primary(parser: &mut Parser) -> ParseResult<Expr> {
    match parser.peek() {
        Some(Token::Number {
            value: n,
            is_float,
            raw,
        }) => {
            let n = *n;
            let is_float = *is_float;
            let raw = raw.clone();
            let _ = parser.next();
            if !is_float && n.fract() == 0.0 {
                Ok(Expr::Integer(n as i64))
            } else {
                Ok(Expr::Float(raw))
            }
        }
        Some(Token::String(s)) => {
            let s = s.clone();
            let _ = parser.next();
            Ok(Expr::String(s))
        }
        Some(Token::NString(s)) => {
            let s = s.clone();
            let _ = parser.next();
            Ok(Expr::UnicodeString(s))
        }
        Some(Token::Variable(v)) => {
            let v = v.clone();
            let _ = parser.next();
            Ok(Expr::Variable(v))
        }
        Some(Token::BinaryLiteral(hex)) => {
            let hex = hex.clone();
            let _ = parser.next();
            let hex_str = if hex.starts_with("0x") || hex.starts_with("0X") {
                &hex[2..]
            } else {
                hex.as_ref()
            };
            let padded;
            let normalized = if hex_str.len() % 2 != 0 {
                padded = format!("0{}", hex_str);
                padded.as_str()
            } else {
                hex_str
            };
            let bytes = (0..normalized.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&normalized[i..i + 2], 16).unwrap_or(0))
                .collect();
            Ok(Expr::BinaryLiteral(bytes))
        }
        Some(Token::Tilde) => {
            let _ = parser.next();
            let expr = super::pratt::parse_expr(parser)?;
            Ok(Expr::Unary {
                op: UnaryOp::BitwiseNot,
                expr: Box::new(expr),
            })
        }
        Some(Token::Identifier(id)) => {
            let id = id.clone();
            let _ = parser.next();
            parse_identifier_or_function(parser, id)
        }
        Some(Token::Keyword(k)) if *k == Keyword::Null => {
            let _ = parser.next();
            Ok(Expr::Null)
        }
        Some(Token::Keyword(k)) if *k == Keyword::Coalesce || *k == Keyword::Nullif => {
            let name = k.as_ref().to_string();
            let _ = parser.next();
            parser.expect_lparen()?;
            let args = parse_comma_list(parser, parse_expr)?;
            parser.expect_rparen()?;
            Ok(Expr::FunctionCall {
                name,
                args,
                within_group: None,
            })
        }
        Some(Token::Keyword(k)) if *k == Keyword::Case => {
            let _ = parser.next();
            parse_case(parser)
        }
        Some(Token::Keyword(k)) if *k == Keyword::Exists => {
            let _ = parser.next();
            parser.expect_lparen()?;
            let subquery = Box::new(crate::parser::parse::statements::query::parse_select(
                parser,
            )?);
            parser.expect_rparen()?;
            Ok(Expr::Exists {
                subquery,
                negated: false,
            })
        }
        Some(Token::Keyword(k)) if *k == Keyword::Cast => {
            let _ = parser.next();
            parse_cast(parser)
        }
        Some(Token::Keyword(k)) if *k == Keyword::Convert => {
            let _ = parser.next();
            parse_convert(parser)
        }
        Some(Token::Keyword(k)) if *k == Keyword::TryCast => {
            let _ = parser.next();
            parse_try_cast(parser)
        }
        Some(Token::Keyword(k)) if *k == Keyword::TryConvert => {
            let _ = parser.next();
            parse_try_convert(parser)
        }
        Some(Token::Keyword(k)) if *k == Keyword::Not => {
            let _ = parser.next();
            if parser.at_keyword(Keyword::Exists) {
                let _ = parser.next();
                parser.expect_lparen()?;
                let subquery = Box::new(crate::parser::parse::statements::query::parse_select(
                    parser,
                )?);
                parser.expect_rparen()?;
                Ok(Expr::Exists {
                    subquery,
                    negated: true,
                })
            } else {
                // SQL precedence: NOT binds tighter than AND (3,4) and OR (1,2),
                // and looser than comparisons (=, LIKE, IN, BETWEEN at 5,6).
                let expr = super::pratt::parse_pratt_expr(parser, 5)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
        }
        Some(Token::Keyword(Keyword::Update))
            if matches!(parser.peek_at(1), Some(Token::LParen)) =>
        {
            let _ = parser.next(); // UPDATE
            let _ = parser.next(); // (
            let col_name = match parser.next() {
                Some(Token::Identifier(id)) => id.clone(),
                Some(Token::Keyword(k)) => k.as_sql().to_string(),
                _ => {
                    return parser.backtrack(Expected::Description("column name in UPDATE(column)"))
                }
            };
            parser.expect_rparen()?;
            Ok(Expr::FunctionCall {
                name: "__TRIGGER_UPDATE".to_string(),
                args: vec![Expr::String(col_name)],
                within_group: None,
            })
        }
        Some(Token::Keyword(Keyword::Next)) => {
            let _ = parser.next();
            parser.expect_keyword(Keyword::Value)?;
            parser.expect_keyword(Keyword::For)?;
            let sequence_name =
                crate::parser::parse::statements::query::parse_multipart_name(parser)?;
            Ok(Expr::NextValueFor { sequence_name })
        }
        Some(Token::Keyword(k))
            if matches!(
                k,
                Keyword::CurrentDate
                    | Keyword::CurrentTime
                    | Keyword::CurrentTimestamp
                    | Keyword::CurrentUser
                    | Keyword::SessionUser
                    | Keyword::SystemUser
            ) =>
        {
            let name = k.as_sql().to_string();
            let _ = parser.next();
            Ok(Expr::FunctionCall {
                name,
                args: vec![],
                within_group: None,
            })
        }
        Some(Token::Keyword(k)) if matches!(parser.peek_at(1), Some(Token::LParen)) => {
            let name = k.as_ref().to_string();
            let _ = parser.next();
            parse_identifier_or_function(parser, name)
        }
        Some(Token::Operator(op)) if *op == "-" => {
            let _ = parser.next();
            let expr = super::pratt::parse_pratt_expr(parser, 13)?;
            Ok(Expr::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(expr),
            })
        }
        Some(Token::Operator(op)) if *op == "+" => {
            let _ = parser.next();
            super::pratt::parse_pratt_expr(parser, 13)
        }
        Some(Token::Keyword(k))
            if matches!(parser.peek_at(1), Some(Token::LParen) | Some(Token::Dot)) =>
        {
            let name = k.as_ref().to_string().to_lowercase();
            let _ = parser.next();
            parse_identifier_or_function(parser, name)
        }
        Some(Token::Keyword(k)) => {
            let name = k.as_ref().to_string().to_lowercase();
            let _ = parser.next();
            Ok(Expr::Identifier(name))
        }
        Some(Token::LParen) => {
            let _ = parser.next();
            let expr = if parser.at_keyword(Keyword::Select) {
                Expr::Subquery(Box::new(
                    crate::parser::parse::statements::query::parse_select(parser)?,
                ))
            } else {
                parse_expr(parser)?
            };
            parser.expect_rparen()?;
            // Support SQL Server XML .value(xpath, sqltype) postfix on parenthesized subqueries/expressions,
            // e.g. (SELECT ',' + name FROM t FOR XML PATH(''), TYPE).value('.', 'NVARCHAR(MAX)')
            if matches!(parser.peek(), Some(Token::Dot))
                && matches!(
                    parser.peek_at(1),
                    Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("VALUE")
                )
                || (matches!(parser.peek(), Some(Token::Dot))
                    && matches!(
                        parser.peek_at(1),
                        Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("VALUE")
                    ))
            {
                if matches!(parser.peek_at(2), Some(Token::LParen)) {
                    let _ = parser.next(); // .
                    let _ = parser.next(); // value
                    let _ = parser.next(); // (
                    let _xpath = parse_expr(parser)?;
                    parser.expect_comma()?;
                    let _sql_type = parse_expr(parser)?;
                    parser.expect_rparen()?;
                }
            }
            Ok(expr)
        }
        Some(Token::Star) => {
            let _ = parser.next();
            Ok(Expr::Wildcard)
        }
        _ => parser.backtrack(Expected::Description("expression")),
    }
}

pub fn parse_case(parser: &mut Parser) -> ParseResult<Expr> {
    let mut operand = None;
    if !parser.at_keyword(Keyword::When) {
        operand = Some(Box::new(parse_expr(parser)?));
    }

    let mut when_clauses = Vec::new();
    while parser.at_keyword(Keyword::When) {
        let _ = parser.next();
        let condition = parse_expr(parser)?;
        parser.expect_keyword(Keyword::Then)?;
        let result = parse_expr(parser)?;
        when_clauses.push(WhenClause { condition, result });
    }

    let mut else_result = None;
    if parser.at_keyword(Keyword::Else) {
        let _ = parser.next();
        else_result = Some(Box::new(parse_expr(parser)?));
    }

    parser.expect_keyword(Keyword::End)?;
    Ok(Expr::Case {
        operand,
        when_clauses,
        else_result,
    })
}

pub fn parse_cast(parser: &mut Parser) -> ParseResult<Expr> {
    parser.expect_lparen()?;
    let expr = parse_expr(parser)?;
    parser.expect_keyword(Keyword::As)?;
    let target = parse_data_type(parser)?;
    parser.expect_rparen()?;
    Ok(Expr::Cast {
        expr: Box::new(expr),
        target,
    })
}

pub fn parse_convert(parser: &mut Parser) -> ParseResult<Expr> {
    parser.expect_lparen()?;
    let target = parse_data_type(parser)?;
    parser.expect_comma()?;
    let expr = parse_expr(parser)?;
    let mut style = None;
    if matches!(parser.peek(), Some(Token::Comma)) {
        let _ = parser.next();
        if let Some(Token::Number { value: s, .. }) = parser.next() {
            style = Some(*s as i32);
        } else {
            return parser.backtrack(Expected::Description("number"));
        }
    }
    parser.expect_rparen()?;
    Ok(Expr::Convert {
        target,
        expr: Box::new(expr),
        style,
    })
}

pub fn parse_try_cast(parser: &mut Parser) -> ParseResult<Expr> {
    parser.expect_lparen()?;
    let expr = parse_expr(parser)?;
    parser.expect_keyword(Keyword::As)?;
    let target = parse_data_type(parser)?;
    parser.expect_rparen()?;
    Ok(Expr::TryCast {
        expr: Box::new(expr),
        target,
    })
}

pub fn parse_try_convert(parser: &mut Parser) -> ParseResult<Expr> {
    parser.expect_lparen()?;
    let target = parse_data_type(parser)?;
    parser.expect_comma()?;
    let expr = parse_expr(parser)?;
    let mut style = None;
    if matches!(parser.peek(), Some(Token::Comma)) {
        let _ = parser.next();
        if let Some(Token::Number { value: s, .. }) = parser.next() {
            style = Some(*s as i32);
        } else {
            return parser.backtrack(Expected::Description("number"));
        }
    }
    parser.expect_rparen()?;
    Ok(Expr::TryConvert {
        target,
        expr: Box::new(expr),
        style,
    })
}

fn parse_identifier_or_function(parser: &mut Parser, name: String) -> ParseResult<Expr> {
    let mut parts = vec![name];
    loop {
        if matches!(parser.peek(), Some(Token::Dot)) {
            let _ = parser.next();
            if let Some(tok) = parser.next() {
                match tok {
                    Token::Identifier(next_id) => parts.push(next_id.clone()),
                    Token::Keyword(k) => parts.push(k.as_ref().to_string()),
                    Token::Star => {
                        return Ok(Expr::QualifiedWildcard(parts));
                    }
                    _ => return parser.backtrack(Expected::Description("identifier")),
                }
            } else {
                return parser.backtrack(Expected::Description("identifier"));
            }
        } else if matches!(parser.peek(), Some(Token::Operator(op)) if op == "::") {
            // T-SQL static CLR/spatial type method call: `geometry::STGeomFromText(...)`
            let _ = parser.next();
            let method = match parser.next() {
                Some(Token::Identifier(next_id)) => next_id.clone(),
                Some(Token::Keyword(k)) => k.as_ref().to_string(),
                _ => return parser.backtrack(Expected::Description("method name")),
            };
            if let Some(last) = parts.last_mut() {
                last.push_str("::");
                last.push_str(&method);
            }
        } else {
            break;
        }
    }

    if matches!(parser.peek(), Some(Token::LParen)) {
        if parts.len() == 1 && parts[0].eq_ignore_ascii_case("JSON_OBJECTAGG") {
            let _ = parser.next(); // consume '('
            let key_expr = parse_expr(parser)?;
            if !matches!(parser.peek(), Some(Token::Colon)) {
                return parser.backtrack(Expected::Description("':' in JSON_OBJECTAGG"));
            }
            let _ = parser.next(); // consume ':'
            let val_expr = parse_expr(parser)?;
            let mut absent_on_null = false;
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ABSENT"))
            {
                let _ = parser.next();
                parser.expect_keyword(Keyword::On)?;
                parser.expect_keyword(Keyword::Null)?;
                absent_on_null = true;
            } else if parser.at_keyword(Keyword::Null) {
                let _ = parser.next();
                parser.expect_keyword(Keyword::On)?;
                parser.expect_keyword(Keyword::Null)?;
                absent_on_null = false;
            }
            parser.expect_rparen()?;
            let fn_name = if absent_on_null {
                "__JSON_OBJECTAGG_ABSENT"
            } else {
                "__JSON_OBJECTAGG_NULL"
            };
            return Ok(Expr::FunctionCall {
                name: fn_name.to_string(),
                args: vec![key_expr, val_expr],
                within_group: None,
            });
        }
        if parts.len() == 1 && parts[0].eq_ignore_ascii_case("JSON_ARRAYAGG") {
            let _ = parser.next(); // consume '('
            let val_expr = parse_expr(parser)?;
            let mut order_by: Option<Box<Vec<OrderByExpr>>> = None;
            if parser.at_keyword(Keyword::Order) {
                let _ = parser.next();
                parser.expect_keyword(Keyword::By)?;
                let obs = parse_comma_list(
                    parser,
                    crate::parser::parse::statements::query::parse_order_by_expr,
                )?;
                order_by = Some(Box::new(obs));
            }
            let mut null_on_null = false;
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ABSENT"))
            {
                let _ = parser.next();
                parser.expect_keyword(Keyword::On)?;
                parser.expect_keyword(Keyword::Null)?;
                null_on_null = false;
            } else if parser.at_keyword(Keyword::Null) {
                let _ = parser.next();
                parser.expect_keyword(Keyword::On)?;
                parser.expect_keyword(Keyword::Null)?;
                null_on_null = true;
            }
            parser.expect_rparen()?;
            let fn_name = if null_on_null {
                "__JSON_ARRAYAGG_NULL"
            } else {
                "__JSON_ARRAYAGG_ABSENT"
            };
            return Ok(Expr::FunctionCall {
                name: fn_name.to_string(),
                args: vec![val_expr],
                within_group: order_by,
            });
        }
        if parts.len() == 1 && parts[0].eq_ignore_ascii_case("JSON_OBJECT") {
            let _ = parser.next(); // consume '('
            let mut args = Vec::new();
            let mut absent_on_null = false;
            loop {
                if matches!(parser.peek(), Some(Token::RParen)) {
                    break;
                }
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ABSENT"))
                {
                    let saved = parser.save();
                    let _ = parser.next();
                    if parser.at_keyword(Keyword::On) {
                        let _ = parser.next();
                        if parser.at_keyword(Keyword::Null) {
                            let _ = parser.next();
                            absent_on_null = true;
                            break;
                        }
                    }
                    parser.restore(saved);
                } else if parser.at_keyword(Keyword::Null) {
                    let saved = parser.save();
                    let _ = parser.next();
                    if parser.at_keyword(Keyword::On) {
                        let _ = parser.next();
                        if parser.at_keyword(Keyword::Null) {
                            let _ = parser.next();
                            absent_on_null = false;
                            break;
                        }
                    }
                    parser.restore(saved);
                }
                let key_expr = parse_expr(parser)?;
                if !matches!(parser.peek(), Some(Token::Colon)) {
                    return parser.backtrack(Expected::Description("':' in JSON_OBJECT"));
                }
                let _ = parser.next(); // consume ':'
                let val_expr = parse_expr(parser)?;
                args.push(key_expr);
                args.push(val_expr);
                if matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                } else {
                    // Check trailing NULL ON NULL / ABSENT ON NULL without preceding comma
                    if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ABSENT"))
                    {
                        let _ = parser.next();
                        parser.expect_keyword(Keyword::On)?;
                        parser.expect_keyword(Keyword::Null)?;
                        absent_on_null = true;
                    } else if parser.at_keyword(Keyword::Null) {
                        let _ = parser.next();
                        parser.expect_keyword(Keyword::On)?;
                        parser.expect_keyword(Keyword::Null)?;
                        absent_on_null = false;
                    }
                    break;
                }
            }
            parser.expect_rparen()?;
            let fn_name = if absent_on_null {
                "__JSON_OBJECT_ABSENT"
            } else {
                "__JSON_OBJECT_NULL"
            };
            return Ok(Expr::FunctionCall {
                name: fn_name.to_string(),
                args,
                within_group: None,
            });
        }
        if parts.len() == 1 && parts[0].eq_ignore_ascii_case("JSON_ARRAY") {
            let _ = parser.next(); // consume '('
            let mut args = Vec::new();
            let mut null_on_null = false;
            loop {
                if matches!(parser.peek(), Some(Token::RParen)) {
                    break;
                }
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ABSENT"))
                {
                    let saved = parser.save();
                    let _ = parser.next();
                    if parser.at_keyword(Keyword::On) {
                        let _ = parser.next();
                        if parser.at_keyword(Keyword::Null) {
                            let _ = parser.next();
                            null_on_null = false;
                            break;
                        }
                    }
                    parser.restore(saved);
                } else if parser.at_keyword(Keyword::Null) {
                    let saved = parser.save();
                    let _ = parser.next();
                    if parser.at_keyword(Keyword::On) {
                        let _ = parser.next();
                        if parser.at_keyword(Keyword::Null) {
                            let _ = parser.next();
                            null_on_null = true;
                            break;
                        }
                    }
                    parser.restore(saved);
                }
                let val_expr = parse_expr(parser)?;
                args.push(val_expr);
                if matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                } else {
                    if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ABSENT"))
                    {
                        let _ = parser.next();
                        parser.expect_keyword(Keyword::On)?;
                        parser.expect_keyword(Keyword::Null)?;
                        null_on_null = false;
                    } else if parser.at_keyword(Keyword::Null) {
                        let _ = parser.next();
                        parser.expect_keyword(Keyword::On)?;
                        parser.expect_keyword(Keyword::Null)?;
                        null_on_null = true;
                    }
                    break;
                }
            }
            parser.expect_rparen()?;
            let fn_name = if null_on_null {
                "__JSON_ARRAY_NULL"
            } else {
                "__JSON_ARRAY_ABSENT"
            };
            return Ok(Expr::FunctionCall {
                name: fn_name.to_string(),
                args,
                within_group: None,
            });
        }
        if parts.len() == 1
            && (parts[0].eq_ignore_ascii_case("TRY_PARSE")
                || parts[0].eq_ignore_ascii_case("PARSE"))
        {
            let is_try = parts[0].eq_ignore_ascii_case("TRY_PARSE");
            let _ = parser.next();
            let expr = parse_expr(parser)?;
            parser.expect_keyword(Keyword::As)?;
            let target = parse_data_type(parser)?;
            if parser.at_keyword(Keyword::Using) {
                let _ = parser.next();
                let _ = parse_expr(parser)?;
            }
            parser.expect_rparen()?;
            return Ok(if is_try {
                Expr::TryCast {
                    expr: Box::new(expr),
                    target,
                }
            } else {
                Expr::Cast {
                    expr: Box::new(expr),
                    target,
                }
            });
        }
        let _ = parser.next();
        let mut has_distinct = false;
        if parser.at_keyword(Keyword::Distinct) {
            let _ = parser.next();
            has_distinct = true;
        }
        let args = if matches!(parser.peek(), Some(Token::Star)) {
            let _ = parser.next();
            vec![Expr::Wildcard]
        } else if matches!(parser.peek(), Some(Token::RParen)) {
            vec![]
        } else {
            parse_comma_list(parser, parse_expr)?
        };
        parser.expect_rparen()?;
        let mut function_name = if parts.len() == 1 {
            parts.remove(0)
        } else {
            parts
                .iter()
                .map(|p| p.as_ref())
                .collect::<Vec<_>>()
                .join(".")
        };
        if has_distinct {
            function_name = format!("{}_DISTINCT", function_name.to_uppercase());
        }
        let within_group = if parser.at_keyword(Keyword::Within) {
            let _ = parser.next();
            parser.expect_keyword(Keyword::Group)?;
            parser.expect_lparen()?;
            parser.expect_keyword(Keyword::Order)?;
            parser.expect_keyword(Keyword::By)?;
            let order_by = parse_comma_list(
                parser,
                crate::parser::parse::statements::query::parse_order_by_expr,
            )?;
            parser.expect_rparen()?;
            Some(Box::new(order_by))
        } else {
            None
        };
        let mut ignore_nulls = false;
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("IGNORE"))
            && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NULLS"))
        {
            let _ = parser.next();
            let _ = parser.next();
            ignore_nulls = true;
        } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("RESPECT"))
            && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NULLS"))
        {
            let _ = parser.next();
            let _ = parser.next();
            ignore_nulls = false;
        }
        if parser.at_keyword(Keyword::Over) {
            return parse_window_over(parser, function_name, args, ignore_nulls);
        }
        Ok(Expr::FunctionCall {
            name: function_name,
            args,
            within_group,
        })
    } else if parts.len() == 1 {
        let upper = parts[0].to_uppercase();
        if matches!(
            upper.as_str(),
            "CURRENT_TIMESTAMP"
                | "CURRENT_DATE"
                | "CURRENT_TIME"
                | "CURRENT_USER"
                | "SESSION_USER"
                | "SYSTEM_USER"
                | "GETDATE"
        ) {
            Ok(Expr::FunctionCall {
                name: parts.remove(0),
                args: vec![],
                within_group: None,
            })
        } else {
            Ok(Expr::Identifier(parts.remove(0)))
        }
    } else if parts.len() > 1 {
        Ok(Expr::QualifiedIdentifier(parts))
    } else {
        Ok(Expr::Identifier(parts.remove(0)))
    }
}
