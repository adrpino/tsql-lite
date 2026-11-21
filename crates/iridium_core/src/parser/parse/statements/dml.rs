use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

fn is_statement_starter(tok: Option<&Token>) -> bool {
    match tok {
        Some(Token::Keyword(k)) => matches!(
            k,
            Keyword::Select
                | Keyword::Insert
                | Keyword::Update
                | Keyword::Delete
                | Keyword::Create
                | Keyword::Drop
                | Keyword::Alter
                | Keyword::Declare
                | Keyword::Set
                | Keyword::If
                | Keyword::While
                | Keyword::Exec
                | Keyword::Execute
                | Keyword::Print
                | Keyword::Begin
                | Keyword::Commit
                | Keyword::Rollback
                | Keyword::Save
                | Keyword::Return
                | Keyword::Break
                | Keyword::Continue
                | Keyword::Merge
                | Keyword::Truncate
                | Keyword::Open
                | Keyword::Close
                | Keyword::Deallocate
                | Keyword::Fetch
                | Keyword::With
                | Keyword::Throw
                | Keyword::End
                | Keyword::Else
                | Keyword::RaiseError
        ),
        _ => false,
    }
}

pub fn parse_insert_dispatch(parser: &mut Parser) -> ParseResult<DmlStatement> {
    if let Some(Token::Keyword(Keyword::Bulk)) = parser.peek() {
        let _ = parser.next();
        return Ok(DmlStatement::InsertBulk(Box::new(parse_insert_bulk(
            parser,
        )?)));
    }
    Ok(DmlStatement::Insert(Box::new(parse_insert(parser)?)))
}

pub fn parse_insert(parser: &mut Parser) -> ParseResult<InsertStmt> {
    if let Some(Token::Keyword(Keyword::Into)) = parser.peek() {
        let _ = parser.next();
    }
    let table = super::parse_multipart_name(parser)?;

    // Optional table hints: INSERT [INTO] tbl WITH (TABLOCK, ...)
    if parser.at_keyword(Keyword::With) && matches!(parser.peek_at(1), Some(Token::LParen)) {
        let _ = parser.next(); // WITH
        let _ = parser.next(); // (
        let mut depth = 1usize;
        while depth > 0 {
            match parser.next() {
                Some(Token::LParen) => depth += 1,
                Some(Token::RParen) => depth -= 1,
                Some(_) => {}
                None => return parser.backtrack(Expected::Description(")")),
            }
        }
    }

    let mut columns = Vec::new();
    if matches!(parser.peek(), Some(Token::LParen))
        && !matches!(
            parser.peek_at(1),
            Some(Token::Keyword(Keyword::Select | Keyword::With))
        )
    {
        let _ = parser.next();
        columns = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
            if let Some(tok) = p.next() {
                match tok {
                    Token::Identifier(id) => Ok(id.clone()),
                    Token::Keyword(kw) => Ok(kw.as_ref().to_string()),
                    _ => p.backtrack(Expected::Description("column name")),
                }
            } else {
                p.backtrack(Expected::Description("column name"))
            }
        })?;
        parser.expect_rparen()?;
    }

    let mut output = None;
    let mut output_into = None;
    let mut output_into_columns = Vec::new();
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Output))) {
        let _ = parser.next();
        let (out_cols, out_into, out_into_cols) = parse_output_clause(parser)?;
        output = Some(out_cols);
        output_into = out_into;
        output_into_columns = out_into_cols;
    }

    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Default))) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Values)?;
        return Ok(InsertStmt {
            table,
            columns,
            source: InsertSource::DefaultValues,
            output,
            output_into,
            output_into_columns,
        });
    }

    // Support parenthesized SELECT source: INSERT [INTO] tbl (SELECT ...)
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        let select_stmt = crate::parser::parse::statements::query::parse_select(parser)?;
        parser.expect_rparen()?;
        crate::parser::parse::statements::query::skip_query_option_clause(parser)?;
        return Ok(InsertStmt {
            table,
            columns,
            source: InsertSource::Select(Box::new(select_stmt)),
            output,
            output_into,
            output_into_columns,
        });
    }

    let k = match parser.next() {
        Some(Token::Keyword(k)) => *k,
        _ => return parser.backtrack(Expected::Description("VALUES, SELECT, or EXEC")),
    };

    let source = match k {
        Keyword::Values => {
            let rows = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                p.expect_lparen()?;
                let vals = crate::parser::parse::expressions::parse_comma_list(
                    p,
                    crate::parser::parse::expressions::parse_expr,
                )?;
                p.expect_rparen()?;
                Ok(vals)
            })?;
            InsertSource::Values(rows)
        }
        Keyword::Select => InsertSource::Select(Box::new(
            crate::parser::parse::statements::query::parse_select_body(parser)?,
        )),
        Keyword::Exec | Keyword::Execute => {
            let mut return_variable = None;
            if let Some(Token::Variable(v)) = parser.peek() {
                if matches!(parser.peek_at(1), Some(Token::Operator(op)) if *op == "=") {
                    return_variable = Some(v.clone());
                    let _ = parser.next();
                    let _ = parser.next();
                }
            }
            let procedure = super::parse_multipart_name(parser)?;
            let args = if !parser.is_empty()
                && !matches!(parser.peek(), Some(Token::Semicolon) | Some(Token::Go))
                && !is_statement_starter(parser.peek())
            {
                crate::parser::parse::expressions::parse_comma_list(
                    parser,
                    super::other::parse_exec_arg,
                )?
            } else {
                Vec::new()
            };
            InsertSource::Exec {
                return_variable,
                procedure,
                args,
            }
        }
        _ => return parser.backtrack(Expected::Description("VALUES, SELECT, or EXEC")),
    };

    crate::parser::parse::statements::query::skip_query_option_clause(parser)?;

    Ok(InsertStmt {
        table,
        columns,
        source,
        output,
        output_into,
        output_into_columns,
    })
}

pub fn parse_update(parser: &mut Parser) -> ParseResult<UpdateStmt> {
    let mut top = None;
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Top))) {
        let _ = parser.next();
        parser.expect_lparen()?;
        top = Some(crate::parser::parse::expressions::parse_expr(parser)?);
        parser.expect_rparen()?;
    }

    let table = crate::parser::parse::statements::query::parse_table_ref(parser)?;
    parser.expect_keyword(Keyword::Set)?;
    let assignment_groups =
        crate::parser::parse::expressions::parse_comma_list(parser, parse_update_assignment_group)?;
    let assignments: Vec<UpdateAssignment> = assignment_groups.into_iter().flatten().collect();

    let mut output = None;
    let mut output_into = None;
    let mut output_into_columns = Vec::new();
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Output))) {
        let _ = parser.next();
        let (out_cols, out_into, out_into_cols) = parse_output_clause(parser)?;
        output = Some(out_cols);
        output_into = out_into;
        output_into_columns = out_into_cols;
    }

    let mut from = None;
    let mut joins = Vec::new();
    let mut applies = Vec::new();
    if let Some(Token::Keyword(Keyword::From)) = parser.peek() {
        let _ = parser.next();
        from = Some(crate::parser::parse::expressions::parse_comma_list(
            parser,
            crate::parser::parse::statements::query::parse_table_ref,
        )?);
    }

    crate::parser::parse::statements::query::parse_joins_and_applies(
        parser,
        &mut joins,
        &mut applies,
    )?;

    let mut selection = None;
    if let Some(Token::Keyword(Keyword::Where)) = parser.peek() {
        let _ = parser.next();
        selection = Some(crate::parser::parse::expressions::parse_expr(parser)?);
    }

    crate::parser::parse::statements::query::skip_query_option_clause(parser)?;

    Ok(UpdateStmt {
        table,
        assignments,
        top,
        from,
        joins,
        applies,
        selection,
        output,
        output_into,
        output_into_columns,
    })
}

fn extract_xml_modify_sql_ref(dml: &str) -> Option<Expr> {
    let lower = dml.to_ascii_lowercase();
    if let Some(pos) = lower.find("sql:column(") {
        let rest = &dml[pos + "sql:column(".len()..];
        let end = rest.find(')')?;
        let inner = rest[..end].trim().trim_matches('"').trim_matches('\'');
        let parts: Vec<String> = inner
            .split('.')
            .map(|p| p.trim().trim_matches(|c| c == '[' || c == ']').to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() > 1 {
            return Some(Expr::QualifiedIdentifier(parts));
        } else if let Some(col) = parts.into_iter().next() {
            return Some(Expr::Identifier(col));
        }
    }
    if let Some(pos) = lower.find("sql:variable(") {
        let rest = &dml[pos + "sql:variable(".len()..];
        let end = rest.find(')')?;
        let inner = rest[..end].trim().trim_matches('"').trim_matches('\'');
        if inner.starts_with('@') {
            return Some(Expr::Variable(inner.to_string()));
        }
    }
    None
}

fn consume_compound_assign_op(parser: &mut Parser) -> Option<BinaryOp> {
    let is_eq_next = matches!(parser.peek_at(1), Some(Token::Operator(op)) if op == "=");
    if !is_eq_next {
        return None;
    }
    let op = match parser.peek() {
        Some(Token::Operator(s)) if s == "+" => Some(BinaryOp::Add),
        Some(Token::Operator(s)) if s == "-" => Some(BinaryOp::Subtract),
        Some(Token::Star) => Some(BinaryOp::Multiply),
        Some(Token::Operator(s)) if s == "/" => Some(BinaryOp::Divide),
        Some(Token::Operator(s)) if s == "%" => Some(BinaryOp::Modulo),
        _ => None,
    }?;
    let _ = parser.next();
    let _ = parser.next();
    Some(op)
}

fn parse_update_assignment_group(parser: &mut Parser) -> ParseResult<Vec<UpdateAssignment>> {
    let parts = super::parse_multipart_name(parser)?;
    let lhs = parts
        .last()
        .cloned()
        .ok_or_else(|| parser.error(Expected::Description("column name")))?;

    // XML .modify('...') method call: SET XmlCol.modify('...') or SET alias.XmlCol.modify('...')
    if parts.len() >= 2
        && lhs.eq_ignore_ascii_case("modify")
        && matches!(parser.peek(), Some(Token::LParen))
    {
        let _ = parser.next(); // (
        let dml_expr = crate::parser::parse::expressions::parse_expr(parser)?;
        parser.expect_rparen()?;
        let col_parts = parts[..parts.len() - 1].to_vec();
        let target_col = col_parts
            .last()
            .cloned()
            .ok_or_else(|| parser.error(Expected::Description("column name")))?;
        let target_expr = if col_parts.len() > 1 {
            Expr::QualifiedIdentifier(col_parts)
        } else if target_col.starts_with('@') {
            Expr::Variable(target_col.clone())
        } else {
            Expr::Identifier(target_col.clone())
        };
        let mut fn_args = vec![target_expr, dml_expr.clone()];
        // If DML literal references sql:column("...") or sql:variable("@..."), pass the
        // referenced expression as the 3rd argument so the row evaluator resolves it normally.
        if let Expr::String(ref dml) | Expr::UnicodeString(ref dml) = dml_expr {
            if let Some(arg_expr) = extract_xml_modify_sql_ref(dml) {
                fn_args.push(arg_expr);
            }
        }
        return Ok(vec![UpdateAssignment {
            column: target_col,
            expr: Expr::FunctionCall {
                name: "__XML_MODIFY".to_string(),
                args: fn_args,
                within_group: Vec::new(),
            },
        }]);
    }

    // Compound assignment: col += expr, @v += expr, etc.
    if let Some(bin_op) = consume_compound_assign_op(parser) {
        let rhs = crate::parser::parse::expressions::parse_expr(parser)?;
        let lhs_expr = if parts.len() > 1 {
            Expr::QualifiedIdentifier(parts)
        } else if lhs.starts_with('@') {
            Expr::Variable(lhs.clone())
        } else {
            Expr::Identifier(lhs.clone())
        };
        return Ok(vec![UpdateAssignment {
            column: lhs,
            expr: Expr::Binary {
                left: Box::new(lhs_expr),
                op: bin_op,
                right: Box::new(rhs),
            },
        }]);
    }

    if let Some(Token::Operator(op)) = parser.next() {
        if *op != "=" {
            return parser.backtrack(Expected::Description("="));
        }
    } else {
        return parser.backtrack(Expected::Description("="));
    }

    // Chained variable + column assignment: SET @var = col = expr  or  SET @var = col += expr
    if lhs.starts_with('@') {
        let saved = parser.save();
        if let Ok(col_parts) = super::parse_multipart_name(parser) {
            let col_name = col_parts.last().cloned().unwrap_or_default();
            if !col_name.is_empty() && !col_name.starts_with('@') {
                if let Some(bin_op) = consume_compound_assign_op(parser) {
                    let rhs = crate::parser::parse::expressions::parse_expr(parser)?;
                    let col_expr = if col_parts.len() > 1 {
                        Expr::QualifiedIdentifier(col_parts)
                    } else {
                        Expr::Identifier(col_name.clone())
                    };
                    let combined = Expr::Binary {
                        left: Box::new(col_expr),
                        op: bin_op,
                        right: Box::new(rhs),
                    };
                    return Ok(vec![
                        UpdateAssignment {
                            column: col_name,
                            expr: combined.clone(),
                        },
                        UpdateAssignment {
                            column: lhs,
                            expr: combined,
                        },
                    ]);
                } else if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                    let _ = parser.next();
                    let rhs = crate::parser::parse::expressions::parse_expr(parser)?;
                    return Ok(vec![
                        UpdateAssignment {
                            column: col_name,
                            expr: rhs.clone(),
                        },
                        UpdateAssignment {
                            column: lhs,
                            expr: rhs,
                        },
                    ]);
                }
            }
        }
        parser.restore(saved);
    }

    let expr = crate::parser::parse::expressions::parse_expr(parser)?;
    Ok(vec![UpdateAssignment { column: lhs, expr }])
}

fn parse_update_assignment(parser: &mut Parser) -> ParseResult<UpdateAssignment> {
    let mut group = parse_update_assignment_group(parser)?;
    Ok(group.remove(0))
}

pub fn parse_delete(parser: &mut Parser) -> ParseResult<DeleteStmt> {
    let mut top = None;
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Top))) {
        let _ = parser.next();
        parser.expect_lparen()?;
        top = Some(crate::parser::parse::expressions::parse_expr(parser)?);
        parser.expect_rparen()?;
    }

    if matches!(parser.peek(), Some(Token::Keyword(Keyword::From))) {
        let _ = parser.next();
    }

    let target = crate::parser::parse::statements::query::parse_table_ref(parser)?;
    let table = target
        .name_as_object()
        .map(|name| {
            let mut parts = Vec::new();
            if let Some(schema) = &name.schema {
                parts.push(schema.clone());
            }
            parts.push(name.name.clone());
            parts
        })
        .ok_or_else(|| parser.error(Expected::Description("table name")))?;

    let mut output = None;
    let mut output_into = None;
    let mut output_into_columns = Vec::new();
    // T-SQL places OUTPUT before the second FROM in `DELETE alias OUTPUT ... FROM tbl alias`
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Output))) {
        let _ = parser.next();
        let (out_cols, out_into, out_into_cols) = parse_output_clause(parser)?;
        output = Some(out_cols);
        output_into = out_into;
        output_into_columns = out_into_cols;
    }

    let mut from = vec![target.clone()];
    let mut joins = Vec::new();
    let mut applies = Vec::new();
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::From))) {
        let _ = parser.next();
        from = crate::parser::parse::expressions::parse_comma_list(
            parser,
            crate::parser::parse::statements::query::parse_table_ref,
        )?;
    }

    crate::parser::parse::statements::query::parse_joins_and_applies(
        parser,
        &mut joins,
        &mut applies,
    )?;

    if output.is_none() && matches!(parser.peek(), Some(Token::Keyword(Keyword::Output))) {
        let _ = parser.next();
        let (out_cols, out_into, out_into_cols) = parse_output_clause(parser)?;
        output = Some(out_cols);
        output_into = out_into;
        output_into_columns = out_into_cols;
    }

    let mut selection = None;
    if let Some(Token::Keyword(Keyword::Where)) = parser.peek() {
        let _ = parser.next();
        selection = Some(crate::parser::parse::expressions::parse_expr(parser)?);
    }

    crate::parser::parse::statements::query::skip_query_option_clause(parser)?;

    Ok(DeleteStmt {
        table,
        top,
        from,
        joins,
        applies,
        selection,
        output,
        output_into,
        output_into_columns,
    })
}

#[allow(clippy::type_complexity)]
pub fn parse_output_clause(
    parser: &mut Parser,
) -> ParseResult<(Vec<OutputColumn>, Option<Vec<String>>, Vec<String>)> {
    let columns = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
        let saved = p.save();
        let fast_source = match p.peek() {
            Some(Token::Keyword(Keyword::Inserted)) => Some(OutputSource::Inserted),
            Some(Token::Keyword(Keyword::Deleted)) => Some(OutputSource::Deleted),
            Some(Token::Identifier(k)) if k.eq_ignore_ascii_case("INSERTED") => {
                Some(OutputSource::Inserted)
            }
            Some(Token::Identifier(k)) if k.eq_ignore_ascii_case("DELETED") => {
                Some(OutputSource::Deleted)
            }
            _ => None,
        };
        if let Some(source) = fast_source {
            if matches!(p.peek_at(1), Some(Token::Dot)) {
                let _ = p.next();
                let _ = p.next();
                let col_parsed = if matches!(p.peek(), Some(Token::Star)) {
                    let _ = p.next();
                    Some(("*".to_string(), true))
                } else {
                    match p.next() {
                        Some(Token::Identifier(id)) => Some((id.clone(), false)),
                        Some(Token::Keyword(kw)) => Some((kw.as_ref().to_string(), false)),
                        _ => None,
                    }
                };
                if let Some((column, is_wildcard)) = col_parsed {
                    let is_simple_boundary = p.is_empty()
                        || matches!(
                            p.peek(),
                            Some(Token::Comma)
                                | Some(Token::Semicolon)
                                | Some(Token::Go)
                                | Some(Token::Keyword(
                                    Keyword::As
                                        | Keyword::Into
                                        | Keyword::Values
                                        | Keyword::Select
                                        | Keyword::Exec
                                        | Keyword::Execute
                                        | Keyword::From
                                        | Keyword::Where
                                        | Keyword::Default
                                ))
                        );
                    if is_simple_boundary {
                        let alias = if let Some(Token::Keyword(Keyword::As)) = p.peek() {
                            let _ = p.next();
                            match p.next() {
                                Some(Token::Identifier(id)) => Some(id.clone()),
                                Some(Token::Keyword(kw)) => Some(kw.as_ref().to_string()),
                                _ => return p.backtrack(Expected::Description("alias")),
                            }
                        } else {
                            None
                        };
                        return Ok(OutputColumn {
                            source,
                            column,
                            expr: None,
                            alias,
                            is_wildcard,
                        });
                    }
                }
            }
            p.restore(saved);
        }

        // General OUTPUT expression ($action, SOURCE.col, GETUTCDATE(), literals, @vars, etc.)
        let expr = crate::parser::parse::expressions::parse_expr(p)?;
        let alias = if let Some(Token::Keyword(Keyword::As)) = p.peek() {
            let _ = p.next();
            match p.next() {
                Some(Token::Identifier(id)) => Some(id.clone()),
                Some(Token::Keyword(kw)) => Some(kw.as_ref().to_string()),
                Some(Token::String(s)) => Some(s.clone()),
                _ => return p.backtrack(Expected::Description("alias")),
            }
        } else {
            None
        };
        let col_label = alias.clone().unwrap_or_else(|| match &expr {
            Expr::Identifier(id) => id.clone(),
            Expr::QualifiedIdentifier(parts) => parts.last().cloned().unwrap_or_default(),
            _ => String::new(),
        });
        Ok(OutputColumn {
            source: OutputSource::Expr,
            column: col_label,
            expr: Some(expr),
            alias,
            is_wildcard: false,
        })
    })?;
    let mut output_into = None;
    let mut output_into_columns = Vec::new();
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Into))) {
        let _ = parser.next();
        output_into = Some(super::parse_multipart_name(parser)?);
        if matches!(parser.peek(), Some(Token::LParen)) {
            let _ = parser.next();
            output_into_columns =
                crate::parser::parse::expressions::parse_comma_list(parser, |p| match p.next() {
                    Some(Token::Identifier(id)) => Ok(id.clone()),
                    Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                    _ => p.backtrack(Expected::Description("column name")),
                })?;
            parser.expect_rparen()?;
        }
    }
    Ok((columns, output_into, output_into_columns))
}

pub fn parse_bulk_insert(parser: &mut Parser) -> ParseResult<BulkInsertStmt> {
    let table = super::parse_multipart_name(parser)?;
    parser.expect_keyword(Keyword::From)?;
    let from = match parser.next() {
        Some(Token::String(s)) | Some(Token::NString(s)) => s.clone(),
        _ => return parser.backtrack(Expected::Description("file path string")),
    };

    let mut options = Vec::new();
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::With))) {
        let _ = parser.next();
        parser.expect_lparen()?;
        options = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
            let opt_name = if let Some(tok) = p.next() {
                match tok {
                    Token::Identifier(id) => id.clone(),
                    Token::Keyword(kw) => kw.as_ref().to_string(),
                    _ => return p.backtrack(Expected::Description("option name")),
                }
            } else {
                return p.backtrack(Expected::Description("option name"));
            };

            match opt_name.to_uppercase().as_str() {
                "CHECK_CONSTRAINTS" => Ok(BulkInsertOption::CheckConstraints),
                "FIRE_TRIGGERS" => Ok(BulkInsertOption::FireTriggers),
                "KEEPIDENTITY" => Ok(BulkInsertOption::KeepIdentity),
                "KEEPNULLS" => Ok(BulkInsertOption::KeepNulls),
                "TABLOCK" => Ok(BulkInsertOption::TabLock),
                "FORMAT" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::String(s)) | Some(Token::NString(s)) => {
                            Ok(BulkInsertOption::Format(s.clone()))
                        }
                        _ => p.backtrack(Expected::Description("format string")),
                    }
                }
                "DATA_SOURCE" => {
                    // Placeholder for DATA_SOURCE, we'll just skip it for now or handle as part of FORMAT
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    let _ = p.next(); // skip value
                    Ok(BulkInsertOption::Format("DATA_SOURCE".to_string()))
                }
                "DATAFILETYPE" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::String(s)) | Some(Token::NString(s)) => {
                            Ok(BulkInsertOption::DataFiletype(s.clone()))
                        }
                        _ => p.backtrack(Expected::Description("datafiletype string")),
                    }
                }
                "FIELDTERMINATOR" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::String(s)) | Some(Token::NString(s)) => {
                            Ok(BulkInsertOption::FieldTerminator(s.clone()))
                        }
                        _ => p.backtrack(Expected::Description("fieldterminator string")),
                    }
                }
                "ROWTERMINATOR" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::String(s)) | Some(Token::NString(s)) => {
                            Ok(BulkInsertOption::RowTerminator(s.clone()))
                        }
                        _ => p.backtrack(Expected::Description("rowterminator string")),
                    }
                }
                "FIRSTROW" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::Number { value, .. }) => {
                            Ok(BulkInsertOption::FirstRow(*value as i64))
                        }
                        _ => p.backtrack(Expected::Description("firstrow number")),
                    }
                }
                "LASTROW" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::Number { value, .. }) => {
                            Ok(BulkInsertOption::LastRow(*value as i64))
                        }
                        _ => p.backtrack(Expected::Description("lastrow number")),
                    }
                }
                "ERRORFILE" => {
                    if let Some(Token::Operator(op)) = p.next() {
                        if op != "=" {
                            return p.backtrack(Expected::Description("="));
                        }
                    }
                    match p.next() {
                        Some(Token::String(s)) | Some(Token::NString(s)) => {
                            Ok(BulkInsertOption::ErrorFile(s.clone()))
                        }
                        _ => p.backtrack(Expected::Description("errorfile string")),
                    }
                }
                _ => p.backtrack(Expected::Description("supported BULK INSERT option")),
            }
        })?;
        parser.expect_rparen()?;
    }

    Ok(BulkInsertStmt {
        table,
        from,
        options,
    })
}

pub fn parse_insert_bulk(parser: &mut Parser) -> ParseResult<InsertBulkStmt> {
    let table = super::parse_multipart_name(parser)?;
    parser.expect_lparen()?;
    let columns = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
        let name = if let Some(tok) = p.next() {
            match tok {
                Token::Identifier(id) => id.clone(),
                Token::Keyword(kw) => kw.as_ref().to_string(),
                _ => return p.backtrack(Expected::Description("column name")),
            }
        } else {
            return p.backtrack(Expected::Description("column name"));
        };
        let data_type = crate::parser::parse::expressions::parse_data_type(p)?;
        // Handle optional NULL/NOT NULL
        let mut is_nullable = None;
        if matches!(p.peek(), Some(Token::Keyword(Keyword::Null))) {
            let _ = p.next();
            is_nullable = Some(true);
        } else if matches!(p.peek(), Some(Token::Keyword(Keyword::Not))) {
            let _ = p.next();
            p.expect_keyword(Keyword::Null)?;
            is_nullable = Some(false);
        }

        Ok(ColumnDef {
            name,
            data_type,
            is_nullable,
            is_identity: false,
            identity_spec: None,
            is_primary_key: false,
            is_unique: false,
            default_expr: None,
            default_constraint_name: None,
            check_expr: None,
            check_constraint_name: None,
            computed_expr: None,
            foreign_key: None,
            collation: None,
            is_clustered: false,
        })
    })?;
    parser.expect_rparen()?;

    Ok(InsertBulkStmt { table, columns })
}

pub fn parse_merge(parser: &mut Parser) -> ParseResult<MergeStmt> {
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Into))) {
        let _ = parser.next();
    }
    let target = crate::parser::parse::statements::query::parse_table_ref(parser)?;
    parser.expect_keyword(Keyword::Using)?;
    let source = crate::parser::parse::statements::query::parse_table_ref(parser)?;
    parser.expect_keyword(Keyword::On)?;
    let on_condition = crate::parser::parse::expressions::parse_expr(parser)?;

    let mut when_clauses = Vec::new();
    while let Some(Token::Keyword(Keyword::When)) = parser.peek() {
        let _ = parser.next();

        let when = if matches!(parser.peek(), Some(Token::Keyword(Keyword::Matched))) {
            let _ = parser.next();
            MergeWhen::Matched
        } else if matches!(parser.peek(), Some(Token::Keyword(Keyword::Not))) {
            let _ = parser.next();
            parser.expect_keyword(Keyword::Matched)?;
            if matches!(parser.peek(), Some(Token::Keyword(Keyword::By))) {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TARGET"))
                    || matches!(parser.peek(), Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("TARGET"))
                {
                    let _ = parser.next();
                    MergeWhen::NotMatched
                } else {
                    parser.expect_keyword(Keyword::Source)?;
                    MergeWhen::NotMatchedBySource
                }
            } else {
                MergeWhen::NotMatched
            }
        } else {
            return parser.backtrack(Expected::Description("MATCHED or NOT MATCHED"));
        };

        let mut condition = None;
        if matches!(parser.peek(), Some(Token::Keyword(Keyword::And))) {
            let _ = parser.next();
            condition = Some(crate::parser::parse::expressions::parse_expr(parser)?);
        }

        parser.expect_keyword(Keyword::Then)?;

        let action = match parser.peek() {
            Some(Token::Keyword(Keyword::Update)) => {
                let _ = parser.next();
                parser.expect_keyword(Keyword::Set)?;
                let assignments = crate::parser::parse::expressions::parse_comma_list(
                    parser,
                    parse_update_assignment,
                )?;
                MergeAction::Update { assignments }
            }
            Some(Token::Keyword(Keyword::Delete)) => {
                let _ = parser.next();
                MergeAction::Delete
            }
            Some(Token::Keyword(Keyword::Insert)) => {
                let _ = parser.next();
                let mut columns = Vec::new();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    columns = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                        if let Some(tok) = p.next() {
                            match tok {
                                Token::Identifier(id) => Ok(id.clone()),
                                Token::Keyword(kw) => Ok(kw.as_ref().to_string()),
                                _ => p.backtrack(Expected::Description("column name")),
                            }
                        } else {
                            p.backtrack(Expected::Description("column name"))
                        }
                    })?;
                    parser.expect_rparen()?;
                }
                parser.expect_keyword(Keyword::Values)?;
                parser.expect_lparen()?;
                let values = crate::parser::parse::expressions::parse_comma_list(
                    parser,
                    crate::parser::parse::expressions::parse_expr,
                )?;
                parser.expect_rparen()?;
                MergeAction::Insert { columns, values }
            }
            _ => return parser.backtrack(Expected::Description("UPDATE, DELETE, or INSERT")),
        };

        when_clauses.push(MergeWhenClause {
            when,
            condition,
            action,
        });
    }

    let mut output = None;
    let mut output_into = None;
    let mut output_into_columns = Vec::new();
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Output))) {
        let _ = parser.next();
        let (out_cols, out_into, out_into_cols) = parse_output_clause(parser)?;
        output = Some(out_cols);
        output_into = out_into;
        output_into_columns = out_into_cols;
    }

    crate::parser::parse::statements::query::skip_query_option_clause(parser)?;

    Ok(MergeStmt {
        target,
        source,
        on_condition,
        when_clauses,
        output,
        output_into,
        output_into_columns,
    })
}
