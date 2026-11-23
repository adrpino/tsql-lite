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
                | Keyword::Checkpoint
        ),
        _ => false,
    }
}

// Re-export canonical implementations from control_flow.rs
pub use super::control_flow::{parse_begin_end, parse_if, parse_try_catch};

#[allow(clippy::while_let_loop)]
pub fn parse_declare(parser: &mut Parser) -> ParseResult<Vec<DeclareVar>> {
    let mut vars = Vec::new();
    loop {
        match parser.next() {
            Some(Token::Variable(name)) => {
                let name = name.clone();
                if parser.at_keyword(Keyword::As) {
                    let _ = parser.next();
                }
                let data_type = crate::parser::parse::expressions::parse_data_type(parser)?;
                let initial_value = if let Some(Token::Operator(op)) = parser.peek() {
                    if *op == "=" {
                        let _ = parser.next();
                        Some(crate::parser::parse::expressions::parse_expr(parser)?)
                    } else {
                        None
                    }
                } else {
                    None
                };
                vars.push(DeclareVar {
                    name,
                    data_type,
                    initial_value,
                });
            }
            _ => break,
        }
        match parser.peek() {
            Some(Token::Comma) => {
                let _ = parser.next();
            }
            _ => break,
        }
    }
    Ok(vars)
}

fn consume_set_compound_op(parser: &mut Parser) -> Option<BinaryOp> {
    if !matches!(parser.peek_at(1), Some(Token::Operator(op)) if op == "=") {
        return None;
    }
    let op = match parser.peek() {
        Some(Token::Operator(s)) if s == "+" => Some(BinaryOp::Add),
        Some(Token::Operator(s)) if s == "-" => Some(BinaryOp::Subtract),
        Some(Token::Star) => Some(BinaryOp::Multiply),
        Some(Token::Operator(s)) if s == "/" => Some(BinaryOp::Divide),
        Some(Token::Operator(s)) if s == "%" => Some(BinaryOp::Modulo),
        Some(Token::Operator(s)) if s == "&" => Some(BinaryOp::BitwiseAnd),
        Some(Token::Operator(s)) if s == "|" => Some(BinaryOp::BitwiseOr),
        Some(Token::Operator(s)) if s == "^" => Some(BinaryOp::BitwiseXor),
        _ => None,
    }?;
    let _ = parser.next();
    let _ = parser.next();
    Some(op)
}

pub fn parse_set(parser: &mut Parser) -> ParseResult<Statement> {
    match parser.next() {
        Some(Token::Variable(variable)) => {
            let variable = variable.clone();
            if let Some(bin_op) = consume_set_compound_op(parser) {
                let rhs = crate::parser::parse::expressions::parse_expr(parser)?;
                return Ok(Statement::Procedural(ProceduralStatement::Set {
                    variable: variable.clone(),
                    expr: Expr::Binary {
                        left: Box::new(Expr::Variable(variable)),
                        op: bin_op,
                        right: Box::new(rhs),
                    },
                }));
            }
            if let Some(Token::Operator(op)) = parser.next() {
                if *op != "=" {
                    return parser.backtrack(Expected::Description("="));
                }
            } else {
                return parser.backtrack(Expected::Description("="));
            }
            // Cursor variable assignment: SET @cur = CURSOR [options...] FOR <select>
            if parser.at_keyword(Keyword::Cursor) {
                let _ = parser.next();
                while !parser.at_keyword(Keyword::For) && !parser.is_empty() {
                    let word = match parser.peek() {
                        Some(Token::Identifier(id)) => Some(id.to_ascii_uppercase()),
                        Some(Token::Keyword(k)) => Some(k.as_sql().to_ascii_uppercase()),
                        _ => None,
                    };
                    if matches!(
                        word.as_deref(),
                        Some(
                            "LOCAL"
                                | "GLOBAL"
                                | "FORWARD_ONLY"
                                | "SCROLL"
                                | "STATIC"
                                | "KEYSET"
                                | "DYNAMIC"
                                | "FAST_FORWARD"
                                | "READ_ONLY"
                                | "SCROLL_LOCKS"
                                | "OPTIMISTIC"
                                | "TYPE_WARNING"
                                | "INSENSITIVE"
                        )
                    ) {
                        let _ = parser.next();
                    } else {
                        break;
                    }
                }
                parser.expect_keyword(Keyword::For)?;
                let query = super::query::parse_select(parser)?;
                return Ok(Statement::Procedural(ProceduralStatement::DeclareCursor {
                    name: variable,
                    query: Box::new(query),
                }));
            }
            let expr = crate::parser::parse::expressions::parse_expr(parser)?;
            Ok(Statement::Procedural(ProceduralStatement::Set {
                variable,
                expr,
            }))
        }
        Some(Token::Keyword(Keyword::ContextInfo)) => {
            let expr = crate::parser::parse::expressions::parse_expr(parser)?;
            Ok(Statement::Session(SessionStatement::SetContextInfo(expr)))
        }
        _ => parser.backtrack(Expected::Description("variable or CONTEXT_INFO")),
    }
}

fn parse_result_set_column_def(
    parser: &mut Parser,
) -> ParseResult<crate::parser::ast::statements::other::ResultSetColumnDef> {
    let name = match parser.next() {
        Some(Token::Identifier(id)) => id.clone(),
        Some(Token::Keyword(k)) => k.as_ref().to_string(),
        _ => return parser.backtrack(Expected::Description("result set column name")),
    };
    let data_type = crate::parser::parse::parse_data_type(parser)?;
    // Optional COLLATE <collation_name>
    if parser.at_keyword(Keyword::Collate) {
        let _ = parser.next();
        match parser.next() {
            Some(Token::Identifier(_)) | Some(Token::Keyword(_)) => {}
            _ => return parser.backtrack(Expected::Description("collation name")),
        }
    }
    let mut nullable = None;
    if parser.at_keyword(Keyword::Not) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Null)?;
        nullable = Some(false);
    } else if parser.at_keyword(Keyword::Null) {
        let _ = parser.next();
        nullable = Some(true);
    }
    Ok(crate::parser::ast::statements::other::ResultSetColumnDef {
        name,
        data_type,
        nullable,
    })
}

fn parse_single_result_set_def(
    parser: &mut Parser,
) -> ParseResult<crate::parser::ast::statements::other::ResultSetDef> {
    use crate::parser::ast::statements::other::ResultSetDef;
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        let cols = crate::parser::parse::expressions::parse_comma_list(
            parser,
            parse_result_set_column_def,
        )?;
        parser.expect_rparen()?;
        return Ok(ResultSetDef::Columns(cols));
    }
    if parser.at_keyword(Keyword::As) {
        let _ = parser.next();
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OBJECT"))
            || matches!(parser.peek(), Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("OBJECT"))
        {
            let _ = parser.next();
            let obj = super::parse_multipart_name(parser)?;
            return Ok(ResultSetDef::AsObject(obj));
        }
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TYPE"))
            || matches!(parser.peek(), Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("TYPE"))
        {
            let _ = parser.next();
            let ty = super::parse_multipart_name(parser)?;
            return Ok(ResultSetDef::AsType(ty));
        }
        if parser.at_keyword(Keyword::For) {
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("XML"))
                || matches!(parser.peek(), Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("XML"))
            {
                let _ = parser.next();
                return Ok(ResultSetDef::AsForXml);
            }
        }
    }
    parser.backtrack(Expected::Description("result set definition"))
}

fn parse_exec_with_options(
    parser: &mut Parser,
) -> ParseResult<Option<crate::parser::ast::statements::other::ResultSetsClause>> {
    use crate::parser::ast::statements::other::ResultSetsClause;
    if !parser.at_keyword(Keyword::With) {
        return Ok(None);
    }
    let is_recompile = matches!(
        parser.peek_at(1),
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("RECOMPILE")
    ) || matches!(
        parser.peek_at(1),
        Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("RECOMPILE")
    );
    let is_result_sets = matches!(
        parser.peek_at(1),
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("RESULT")
    );
    if !is_recompile && !is_result_sets {
        return Ok(None);
    }
    let _ = parser.next(); // WITH
    let mut result_sets = None;
    loop {
        if matches!(
            parser.peek(),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("RECOMPILE")
        ) || matches!(
            parser.peek(),
            Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("RECOMPILE")
        ) {
            let _ = parser.next();
        } else if matches!(
            parser.peek(),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("RESULT")
        ) {
            let _ = parser.next(); // RESULT
            if matches!(
                parser.peek(),
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SETS")
            ) {
                let _ = parser.next(); // SETS
            }
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                let defs = crate::parser::parse::expressions::parse_comma_list(
                    parser,
                    parse_single_result_set_def,
                )?;
                parser.expect_rparen()?;
                result_sets = Some(ResultSetsClause::Definitions(defs));
            } else if matches!(
                parser.peek(),
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NONE")
            ) {
                let _ = parser.next();
                result_sets = Some(ResultSetsClause::None);
            } else if matches!(
                parser.peek(),
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("UNDEFINED")
            ) {
                let _ = parser.next();
                result_sets = Some(ResultSetsClause::Undefined);
            } else {
                return parser.backtrack(Expected::Description(
                    "RESULT SETS definition, NONE, or UNDEFINED",
                ));
            }
        } else {
            break;
        }
        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
        } else {
            break;
        }
    }
    Ok(result_sets)
}

pub(super) fn parse_exec_dynamic_tail(
    parser: &mut Parser,
) -> ParseResult<Option<crate::parser::ast::statements::other::ResultSetsClause>> {
    // Optional `AS LOGIN|USER = '...'` and/or linked-server `AT [server]`
    loop {
        if parser.at_keyword(Keyword::As)
            && matches!(
                parser.peek_at(1),
                Some(Token::Keyword(Keyword::User)) | Some(Token::Identifier(_))
            )
        {
            let _ = parser.next(); // AS
            let _ = parser.next(); // LOGIN | USER
            if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                let _ = parser.next();
                let _ = crate::parser::parse::expressions::parse_expr(parser)?;
            }
            continue;
        }
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("AT")) {
            let _ = parser.next(); // AT
            let _ = super::parse_multipart_name(parser)?;
            continue;
        }
        break;
    }
    parse_exec_with_options(parser)
}

pub fn parse_exec_dispatch(parser: &mut Parser) -> ParseResult<Statement> {
    if parser.at_keyword(Keyword::As) {
        let _ = parser.next();
        // EXECUTE AS { USER | LOGIN } = 'principal' [WITH NO REVERT | WITH COOKIE INTO @var]
        match parser.peek() {
            Some(Token::Keyword(Keyword::User)) | Some(Token::Identifier(_)) => {
                let _ = parser.next();
            }
            _ => {}
        }
        if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
            let _ = parser.next();
        }
        let principal = match parser.next() {
            Some(Token::String(s)) | Some(Token::NString(s)) | Some(Token::Identifier(s)) => {
                s.clone()
            }
            Some(Token::Keyword(k)) => k.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("principal name for EXECUTE AS")),
        };
        if parser.at_keyword(Keyword::With) {
            let _ = parser.next();
            while let Some(tok) = parser.peek() {
                if matches!(tok, Token::Semicolon) {
                    break;
                }
                if matches!(tok, Token::Keyword(Keyword::No | Keyword::Into))
                    || matches!(tok, Token::Identifier(id) if id.eq_ignore_ascii_case("REVERT") || id.eq_ignore_ascii_case("COOKIE"))
                    || matches!(tok, Token::Variable(_))
                {
                    let _ = parser.next();
                } else {
                    break;
                }
            }
        }
        return Ok(Statement::Session(SessionStatement::ExecuteAs(principal)));
    }
    match parser.peek() {
        Some(Token::LParen) => {
            let _ = parser.next();
            let sql_expr = crate::parser::parse::expressions::parse_expr(parser)?;
            // Optional pass-through arguments: `EXEC('SELECT ?', @p1, @p2) AT [RemoteServer]`
            while matches!(parser.peek(), Some(Token::Comma)) {
                let _ = parser.next();
                let _ = parse_exec_arg(parser)?;
            }
            parser.expect_rparen()?;
            let result_sets = parse_exec_dynamic_tail(parser)?;
            Ok(Statement::Procedural(ProceduralStatement::ExecDynamic {
                sql_expr,
                result_sets,
            }))
        }
        Some(Token::Identifier(_)) | Some(Token::Keyword(_)) | Some(Token::Variable(_)) => {
            let mut return_variable = None;
            if let Some(Token::Variable(v)) = parser.peek() {
                if matches!(parser.peek_at(1), Some(Token::Operator(op)) if *op == "=") {
                    return_variable = Some(v.clone());
                    let _ = parser.next();
                    let _ = parser.next();
                }
            }

            let name = super::parse_multipart_name(parser)?;
            if return_variable.is_none()
                && name
                    .last()
                    .is_some_and(|n| n.eq_ignore_ascii_case("sp_executesql"))
                && (name.len() == 1
                    || (name.len() == 2
                        && (name[0].eq_ignore_ascii_case("sys")
                            || name[0].eq_ignore_ascii_case("dbo")))
                    || (name.len() == 3
                        && name[0].eq_ignore_ascii_case("master")
                        && name[1].eq_ignore_ascii_case("sys")))
                && !(matches!(parser.peek(), Some(Token::Variable(_)))
                    && matches!(parser.peek_at(1), Some(Token::Operator(op)) if op == "="))
            {
                let sql_expr = crate::parser::parse::expressions::parse_expr(parser)?;
                let mut params_def = None;
                if matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                    params_def = Some(crate::parser::parse::expressions::parse_expr(parser)?);
                }
                let mut args = Vec::new();
                while matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                    args.push(parse_exec_arg(parser)?);
                }
                let result_sets = parse_exec_with_options(parser)?;
                return Ok(Statement::Procedural(ProceduralStatement::SpExecuteSql {
                    sql_expr,
                    params_def,
                    args,
                    result_sets,
                }));
            }

            let mut args = Vec::new();
            if !parser.is_empty()
                && !matches!(parser.peek(), Some(Token::Semicolon) | Some(Token::Go))
                && !is_statement_starter(parser.peek())
            {
                args = crate::parser::parse::expressions::parse_comma_list(parser, parse_exec_arg)?;
            }
            let result_sets = parse_exec_with_options(parser)?;
            Ok(Statement::Procedural(ProceduralStatement::ExecProcedure {
                return_variable,
                name,
                args,
                result_sets,
            }))
        }
        _ => parser.backtrack(Expected::Description("procedure name or expression")),
    }
}

pub(super) fn parse_exec_arg(parser: &mut Parser) -> ParseResult<ExecArg> {
    let mut name = None;
    if let Some(Token::Variable(v)) = parser.peek() {
        let v = v.clone();
        if matches!(parser.peek_at(1), Some(Token::Operator(op)) if *op == "=") {
            let _ = parser.next();
            let _ = parser.next();
            name = Some(v);
        }
    }
    let expr = crate::parser::parse::expressions::parse_expr(parser)?;
    let mut is_output = false;
    if let Some(Token::Keyword(k)) = parser.peek() {
        if matches!(k, Keyword::Output | Keyword::Out) {
            let _ = parser.next();
            is_output = true;
        }
    }
    Ok(ExecArg {
        name,
        expr,
        is_output,
    })
}
