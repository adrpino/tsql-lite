pub mod expressions;
pub mod statements;

use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::parse::expressions::parse_expr;
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

pub use crate::parser::parse::expressions::{parse_comma_list, parse_data_type};
pub use crate::parser::parse::statements::alter::parse_alter;
pub use crate::parser::parse::statements::ddl::{
    parse_create, parse_create_index, parse_create_schema, parse_create_sequence,
    parse_create_type, parse_table_body,
};
pub use crate::parser::parse::statements::dml::{
    parse_bulk_insert, parse_delete, parse_insert_dispatch, parse_merge, parse_update,
};
pub use crate::parser::parse::statements::drop::parse_drop;
pub use crate::parser::parse::statements::other::{
    parse_begin_end, parse_declare, parse_exec_dispatch, parse_if, parse_set, parse_try_catch,
};
pub use crate::parser::parse::statements::query::{
    parse_multipart_name as multipart_name, parse_select,
};

pub fn parse_batch(parser: &mut Parser) -> ParseResult<Vec<Statement>> {
    let mut statements = Vec::new();
    while !parser.is_empty() {
        while matches!(parser.peek(), Some(Token::Semicolon)) {
            let _ = parser.next();
        }
        if parser.is_empty() {
            break;
        }
        if matches!(parser.peek(), Some(Token::Go)) {
            let _ = parser.next();
            continue;
        }
        // In T-SQL, the first statement of a batch may invoke a stored procedure without EXEC.
        let saved = parser.save();
        match parse_statement(parser) {
            Ok(stmt) => statements.push(stmt),
            Err(err) => {
                if statements.is_empty() {
                    parser.restore(saved);
                    if matches!(parser.peek(), Some(Token::Identifier(_))) {
                        if let Ok(stmt) = parse_exec_dispatch(parser) {
                            statements.push(stmt);
                            continue;
                        }
                        parser.restore(saved);
                    }
                }
                return Err(err);
            }
        }
    }
    Ok(statements)
}

pub fn parse_statement(parser: &mut Parser) -> ParseResult<Statement> {
    parser.enter_recursion()?;
    let res = parse_statement_inner(parser);
    parser.leave_recursion();
    res
}

fn parse_statement_inner(parser: &mut Parser) -> ParseResult<Statement> {
    // Label definition: `<label>:` (must not be a `::` scope qualifier)
    if matches!(parser.peek_at(1), Some(Token::Colon))
        && !matches!(parser.peek_at(2), Some(Token::Colon))
    {
        match parser.peek() {
            Some(Token::Identifier(id)) => {
                let label = id.clone();
                let _ = parser.next();
                let _ = parser.next();
                return Ok(Statement::Procedural(ProceduralStatement::Label(label)));
            }
            Some(Token::Keyword(k)) => {
                let label = k.as_sql().to_string();
                let _ = parser.next();
                let _ = parser.next();
                return Ok(Statement::Procedural(ProceduralStatement::Label(label)));
            }
            _ => {}
        }
    }

    if parser.at_keyword(Keyword::With) {
        let _ = parser.next();
        let ctes = parse_comma_list(parser, parse_cte_def)?;
        let body = parse_statement(parser)?;
        return Ok(Statement::WithCte {
            ctes,
            body: Box::new(body),
        });
    }

    match parser.peek() {
        Some(Token::Keyword(k)) => match *k {
            Keyword::Select => {
                let s = parse_select(parser)?;
                if let Some(assigns) = try_select_assign(&s) {
                    Ok(Statement::Dml(DmlStatement::SelectAssign(Box::new(
                        SelectAssignStmt {
                            assignments: assigns,
                            from: s.from,
                            joins: s.joins,
                            applies: s.applies,
                            selection: s.selection,
                            order_by: s.order_by,
                            top: s.top,
                            max_recursion: s.extra.as_deref().and_then(|e| e.max_recursion),
                        },
                    ))))
                } else {
                    Ok(Statement::Dml(DmlStatement::Select(Box::new(s))))
                }
            }
            Keyword::Insert => {
                let _ = parser.next();
                Ok(Statement::Dml(parse_insert_dispatch(parser)?))
            }
            Keyword::Update => {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("STATISTICS"))
                    || matches!(parser.peek(), Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("STATISTICS"))
                {
                    let _ = parser.next();
                    let _ = multipart_name(parser)?;
                    // Optional index name or parenthesized list of statistics names
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        let _ = parse_comma_list(parser, |p| match p.next() {
                            Some(Token::Identifier(_)) | Some(Token::Keyword(_)) => Ok(()),
                            _ => p.backtrack(Expected::Description("statistics name")),
                        })?;
                        parser.expect_rparen()?;
                    } else if matches!(parser.peek(), Some(Token::Identifier(_))) {
                        let _ = parser.next();
                    }
                    // Optional WITH FULLSCAN / SAMPLE N PERCENT / etc.
                    if parser.at_keyword(Keyword::With) {
                        let _ = parser.next();
                        while let Some(tok) = parser.peek() {
                            if matches!(
                                tok,
                                Token::Semicolon
                                    | Token::Go
                                    | Token::Keyword(
                                        Keyword::Select
                                            | Keyword::Insert
                                            | Keyword::Update
                                            | Keyword::Delete
                                            | Keyword::Declare
                                            | Keyword::Set
                                            | Keyword::If
                                            | Keyword::Else
                                            | Keyword::End
                                            | Keyword::Begin
                                            | Keyword::Return
                                            | Keyword::Exec
                                            | Keyword::Execute
                                            | Keyword::Create
                                            | Keyword::Drop
                                            | Keyword::Alter
                                            | Keyword::While
                                            | Keyword::Merge
                                            | Keyword::With
                                    )
                            ) {
                                break;
                            }
                            let _ = parser.next();
                        }
                    }
                    return Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                        Vec::new(),
                    )));
                }
                Ok(Statement::Dml(DmlStatement::Update(Box::new(
                    parse_update(parser)?,
                ))))
            }
            Keyword::Delete => {
                let _ = parser.next();
                Ok(Statement::Dml(DmlStatement::Delete(Box::new(
                    parse_delete(parser)?,
                ))))
            }
            Keyword::Create => {
                let _ = parser.next();
                if parser.at_keyword(Keyword::Or) {
                    let saved = parser.save();
                    let _ = parser.next();
                    if parser.at_keyword(Keyword::Alter) {
                        let _ = parser.next();
                        let mut stmt = parse_create(parser)?;
                        match &mut stmt {
                            CreateStmt::Procedure { or_alter, .. }
                            | CreateStmt::View { or_alter, .. }
                            | CreateStmt::Function { or_alter, .. }
                            | CreateStmt::Trigger { or_alter, .. } => {
                                *or_alter = true;
                            }
                            _ => {}
                        }
                        return Ok(Statement::Ddl(Box::new(DdlStatement::Create(Box::new(
                            stmt,
                        )))));
                    }
                    parser.restore(saved);
                }
                if parser.at_keyword(Keyword::Unique) {
                    let _ = parser.next();
                    let mut is_clustered = false;
                    if parser.at_keyword(Keyword::Clustered) {
                        let _ = parser.next();
                        is_clustered = true;
                    } else if parser.at_keyword(Keyword::Nonclustered) {
                        let _ = parser.next();
                        is_clustered = false;
                    }
                    parser.expect_keyword(Keyword::Index)?;
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CreateIndex(
                        Box::new(parse_create_index(parser, true, is_clustered)?),
                    ))));
                }
                if parser.at_keyword(Keyword::Clustered) {
                    let _ = parser.next();
                    parser.expect_keyword(Keyword::Index)?;
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CreateIndex(
                        Box::new(parse_create_index(parser, false, true)?),
                    ))));
                }
                if parser.at_keyword(Keyword::Nonclustered) {
                    let _ = parser.next();
                    parser.expect_keyword(Keyword::Index)?;
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CreateIndex(
                        Box::new(parse_create_index(parser, false, false)?),
                    ))));
                }
                if parser.at_keyword(Keyword::Index) {
                    let _ = parser.next();
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CreateIndex(
                        Box::new(parse_create_index(parser, false, false)?),
                    ))));
                }
                if parser.at_keyword(Keyword::Type) {
                    let _ = parser.next();
                    return parse_create_type(parser);
                }
                if parser.at_keyword(Keyword::Database) {
                    let _ = parser.next();
                    let db_name = match parser.next() {
                        Some(Token::Identifier(id)) => id.clone(),
                        Some(Token::Keyword(k)) => k.as_sql().to_string(),
                        _ => return parser.backtrack(Expected::Description("database name")),
                    };
                    statements::alter::skip_ddl_tail(parser);
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                        Box::new(crate::ast::CatalogMetadataAction::CreateDatabase {
                            name: db_name,
                        }),
                    ))));
                }
                if parser.at_keyword(Keyword::Column) {
                    let _ = parser.next();
                    return statements::column_encryption::parse_create_column_key(parser);
                }
                if parser.at_keyword(Keyword::Schema) {
                    let _ = parser.next();
                    return parse_create_schema(parser);
                }
                if parser.at_keyword(Keyword::Synonym) {
                    let _ = parser.next();
                    let name = multipart_name(parser)?;
                    parser.expect_keyword(Keyword::For)?;
                    let base_object = multipart_name(parser)?;
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CreateSynonym {
                        name,
                        base_object,
                    })));
                }
                if parser.at_keyword(Keyword::Sequence) {
                    let _ = parser.next();
                    return parse_create_sequence(parser);
                }
                if parser.at_keyword(Keyword::User) {
                    let _ = parser.next();
                    let name = match parser.next() {
                        Some(Token::Identifier(id)) => id.clone(),
                        Some(Token::Keyword(k)) => k.as_sql().to_string(),
                        _ => return parser.backtrack(Expected::Description("user name")),
                    };
                    let mut default_schema = None;
                    // Consume optional FOR/FROM LOGIN <x> | WITHOUT LOGIN and WITH DEFAULT_SCHEMA = <schema>
                    while !parser.is_empty()
                        && !matches!(parser.peek(), Some(Token::Semicolon) | Some(Token::Go))
                    {
                        if matches!(
                            parser.peek(),
                            Some(Token::Keyword(
                                Keyword::Create
                                    | Keyword::Alter
                                    | Keyword::Drop
                                    | Keyword::Select
                                    | Keyword::Insert
                                    | Keyword::Update
                                    | Keyword::Delete
                                    | Keyword::Declare
                                    | Keyword::Set
                                    | Keyword::Exec
                                    | Keyword::Execute
                            ))
                        ) {
                            break;
                        }
                        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("DEFAULT_SCHEMA"))
                        {
                            let _ = parser.next();
                            if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                                let _ = parser.next();
                                match parser.next() {
                                    Some(Token::Identifier(s)) => default_schema = Some(s.clone()),
                                    Some(Token::Keyword(k)) => {
                                        default_schema = Some(k.as_sql().to_string())
                                    }
                                    _ => {}
                                }
                            }
                        } else {
                            let _ = parser.next();
                        }
                    }
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                        Box::new(crate::ast::CatalogMetadataAction::CreateUser {
                            name,
                            default_schema,
                        }),
                    ))));
                }
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SECURITY"))
                    && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("POLICY"))
                {
                    let _ = parser.next(); // SECURITY
                    let _ = parser.next(); // POLICY
                    return statements::security_policy::parse_create_security_policy(parser);
                }
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ROLE"))
                {
                    let _ = parser.next();
                    let name = match parser.next() {
                        Some(Token::Identifier(id)) => id.clone(),
                        Some(Token::Keyword(k)) => k.as_sql().to_string(),
                        _ => return parser.backtrack(Expected::Description("role name")),
                    };
                    statements::alter::skip_ddl_tail(parser);
                    return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                        Box::new(crate::ast::CatalogMetadataAction::CreateRole { name }),
                    ))));
                }
                if parser.at_keyword(Keyword::Partition) {
                    let _ = parser.next();
                    if parser.at_keyword(Keyword::Function) {
                        let _ = parser.next();
                        let name = match parser.next() {
                            Some(Token::Identifier(id)) => id.clone(),
                            Some(Token::Keyword(k)) => k.as_sql().to_string(),
                            _ => {
                                return parser
                                    .backtrack(Expected::Description("partition function name"))
                            }
                        };
                        parser.expect_lparen()?;
                        let parsed_dt = expressions::parse_data_type(parser)?;
                        parser.expect_rparen()?;
                        let data_type = crate::parser::lower::common::lower_data_type(parsed_dt)
                            .unwrap_or(crate::ast::DataTypeSpec::Int);
                        let _ = parser.at_keyword(Keyword::As).then(|| parser.next());
                        let _ = parser.at_keyword(Keyword::Range).then(|| parser.next());
                        let mut range_right = false;
                        if parser.at_keyword(Keyword::Right) {
                            let _ = parser.next();
                            range_right = true;
                        } else if parser.at_keyword(Keyword::Left) {
                            let _ = parser.next();
                            range_right = false;
                        }
                        let mut boundary_values = Vec::new();
                        if parser.at_keyword(Keyword::For) {
                            let _ = parser.next();
                            if parser.at_keyword(Keyword::Values) {
                                let _ = parser.next();
                            }
                            if matches!(parser.peek(), Some(Token::LParen)) {
                                let _ = parser.next();
                                while !parser.is_empty()
                                    && !matches!(parser.peek(), Some(Token::RParen))
                                {
                                    let mut val_text = String::new();
                                    let mut depth = 0usize;
                                    while let Some(tok) = parser.peek() {
                                        if depth == 0 && matches!(tok, Token::Comma | Token::RParen)
                                        {
                                            break;
                                        }
                                        match parser.next() {
                                            Some(Token::LParen) => {
                                                depth += 1;
                                                val_text.push('(');
                                            }
                                            Some(Token::RParen) => {
                                                depth = depth.saturating_sub(1);
                                                val_text.push(')');
                                            }
                                            Some(Token::String(s)) | Some(Token::NString(s)) => {
                                                val_text.push_str(s);
                                            }
                                            Some(Token::Number { raw, .. }) => {
                                                val_text.push_str(raw);
                                            }
                                            Some(Token::Operator(op)) => val_text.push_str(op),
                                            Some(Token::Identifier(id)) => val_text.push_str(id),
                                            Some(Token::Keyword(k)) => {
                                                val_text.push_str(k.as_sql())
                                            }
                                            _ => {}
                                        }
                                    }
                                    if !val_text.trim().is_empty() {
                                        boundary_values.push(val_text.trim().to_string());
                                    }
                                    if matches!(parser.peek(), Some(Token::Comma)) {
                                        let _ = parser.next();
                                    }
                                }
                                let _ = parser.expect_rparen();
                            }
                        }
                        return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                            Box::new(crate::ast::CatalogMetadataAction::CreatePartitionFunction {
                                name,
                                data_type,
                                range_right,
                                boundary_values,
                            }),
                        ))));
                    } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SCHEME"))
                    {
                        let _ = parser.next();
                        let name = match parser.next() {
                            Some(Token::Identifier(id)) => id.clone(),
                            Some(Token::Keyword(k)) => k.as_sql().to_string(),
                            _ => {
                                return parser
                                    .backtrack(Expected::Description("partition scheme name"))
                            }
                        };
                        let _ = parser.at_keyword(Keyword::As).then(|| parser.next());
                        let _ = parser.at_keyword(Keyword::Partition).then(|| parser.next());
                        let function_name = match parser.next() {
                            Some(Token::Identifier(id)) => id.clone(),
                            Some(Token::Keyword(k)) => k.as_sql().to_string(),
                            _ => {
                                return parser
                                    .backtrack(Expected::Description("partition function name"))
                            }
                        };
                        let all_to = if parser.at_keyword(Keyword::All) {
                            let _ = parser.next();
                            true
                        } else {
                            false
                        };
                        if is_to_token(parser.peek()) {
                            let _ = parser.next();
                        }
                        let mut filegroups = Vec::new();
                        if matches!(parser.peek(), Some(Token::LParen)) {
                            let _ = parser.next();
                            while !parser.is_empty()
                                && !matches!(parser.peek(), Some(Token::RParen))
                            {
                                match parser.next() {
                                    Some(Token::Identifier(fg)) => filegroups.push(fg.clone()),
                                    Some(Token::Keyword(k)) => {
                                        filegroups.push(k.as_sql().to_string())
                                    }
                                    Some(Token::Comma) => {}
                                    _ => {}
                                }
                            }
                            let _ = parser.expect_rparen();
                        }
                        return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                            Box::new(crate::ast::CatalogMetadataAction::CreatePartitionScheme {
                                name,
                                function_name,
                                all_to,
                                filegroups,
                            }),
                        ))));
                    }
                    statements::alter::skip_ddl_tail(parser);
                    return Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                        Vec::new(),
                    )));
                }
                if parser.at_keyword(Keyword::Statistics)
                    || parser.at_keyword(Keyword::Column)
                    || matches!(
                        parser.peek(),
                        Some(Token::Identifier(id))
                            if id.eq_ignore_ascii_case("LOGIN")
                                || id.eq_ignore_ascii_case("FULLTEXT")
                                || id.eq_ignore_ascii_case("MASTER")
                                || id.eq_ignore_ascii_case("CERTIFICATE")
                                || id.eq_ignore_ascii_case("SYMMETRIC")
                                || id.eq_ignore_ascii_case("ASYMMETRIC")
                                || id.eq_ignore_ascii_case("SECURITY")
                                || id.eq_ignore_ascii_case("XML")
                    )
                {
                    let _ = parser.next();
                    statements::alter::skip_ddl_tail(parser);
                    return Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                        Vec::new(),
                    )));
                }
                Ok(Statement::Ddl(Box::new(DdlStatement::Create(Box::new(
                    parse_create(parser)?,
                )))))
            }
            Keyword::Drop => {
                let _ = parser.next();
                if parser.at_keyword(Keyword::Synonym) {
                    let _ = parser.next();
                    let name = multipart_name(parser)?;
                    return Ok(Statement::Ddl(Box::new(DdlStatement::DropSynonym(name))));
                }
                if parser.at_keyword(Keyword::Sequence) {
                    let _ = parser.next();
                    let name = multipart_name(parser)?;
                    return Ok(Statement::Ddl(Box::new(DdlStatement::DropSequence(name))));
                }
                parse_drop(parser)
            }
            Keyword::Truncate => {
                let _ = parser.next();
                parser.expect_keyword(Keyword::Table)?;
                let table = multipart_name(parser)?;
                Ok(Statement::Ddl(Box::new(DdlStatement::TruncateTable(table))))
            }
            Keyword::Alter => {
                let _ = parser.next();
                parse_alter(parser)
            }
            Keyword::Declare => {
                let _ = parser.next();
                parse_declare_dispatch(parser)
            }
            Keyword::Merge => {
                let _ = parser.next();
                Ok(Statement::Dml(DmlStatement::Merge(Box::new(parse_merge(
                    parser,
                )?))))
            }
            Keyword::Bulk => {
                let _ = parser.next();
                parser.expect_keyword(Keyword::Insert)?;
                Ok(Statement::Dml(DmlStatement::BulkInsert(Box::new(
                    parse_bulk_insert(parser)?,
                ))))
            }
            Keyword::Set => {
                let _ = parser.next();
                if parser.at_keyword(Keyword::ContextInfo) {
                    let _ = parser.next();
                    let expr = parse_expr(parser)?;
                    Ok(Statement::Session(SessionStatement::SetContextInfo(expr)))
                } else {
                    parse_set_dispatch(parser)
                }
            }
            Keyword::Use => {
                let _ = parser.next();
                let database = parse_use_database_name(parser)?;
                Ok(Statement::Session(SessionStatement::UseDatabase(database)))
            }
            Keyword::If => {
                let _ = parser.next();
                Ok(parse_if(parser)?)
            }
            Keyword::While => {
                let _ = parser.next();
                let condition = parse_expr(parser)?;
                let stmt = parse_statement(parser)?;
                Ok(Statement::Procedural(ProceduralStatement::While {
                    condition,
                    stmt: Box::new(stmt),
                }))
            }
            Keyword::Exec | Keyword::Execute => {
                let _ = parser.next();
                Ok(parse_exec_dispatch(parser)?)
            }
            Keyword::Print => {
                let _ = parser.next();
                Ok(Statement::Procedural(ProceduralStatement::Print(
                    parse_expr(parser)?,
                )))
            }
            Keyword::RaiseError => {
                let _ = parser.next();
                parser.expect_lparen()?;
                let message = parse_expr(parser)?;
                parser.expect_comma()?;
                let severity = parse_expr(parser)?;
                parser.expect_comma()?;
                let state = parse_expr(parser)?;
                let mut args = Vec::new();
                while matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                    args.push(parse_expr(parser)?);
                }
                parser.expect_rparen()?;
                // Optional WITH NOWAIT | LOG | SETERROR (comma-separated)
                if parser.at_keyword(Keyword::With) {
                    let _ = parser.next();
                    loop {
                        match parser.peek() {
                            Some(Token::Identifier(id))
                                if id.eq_ignore_ascii_case("NOWAIT")
                                    || id.eq_ignore_ascii_case("LOG")
                                    || id.eq_ignore_ascii_case("SETERROR") =>
                            {
                                let _ = parser.next();
                            }
                            Some(Token::Keyword(k))
                                if k.as_sql().eq_ignore_ascii_case("NOWAIT")
                                    || k.as_sql().eq_ignore_ascii_case("LOG")
                                    || k.as_sql().eq_ignore_ascii_case("SETERROR") =>
                            {
                                let _ = parser.next();
                            }
                            _ => break,
                        }
                        if matches!(parser.peek(), Some(Token::Comma)) {
                            let _ = parser.next();
                        } else {
                            break;
                        }
                    }
                }
                Ok(Statement::Procedural(ProceduralStatement::Raiserror {
                    message,
                    severity,
                    state,
                    args,
                }))
            }
            Keyword::Throw => {
                let _ = parser.next();
                if parser.is_empty()
                    || matches!(
                        parser.peek(),
                        Some(Token::Semicolon)
                            | Some(Token::Go)
                            | Some(Token::Keyword(
                                Keyword::End
                                    | Keyword::Else
                                    | Keyword::Select
                                    | Keyword::Insert
                                    | Keyword::Update
                                    | Keyword::Delete
                                    | Keyword::Declare
                                    | Keyword::Set
                                    | Keyword::If
                                    | Keyword::While
                                    | Keyword::Begin
                                    | Keyword::Exec
                                    | Keyword::Execute
                                    | Keyword::Return
                                    | Keyword::Break
                                    | Keyword::Continue
                                    | Keyword::Print
                                    | Keyword::Throw
                                    | Keyword::RaiseError
                                    | Keyword::Commit
                                    | Keyword::Rollback
                                    | Keyword::Save
                                    | Keyword::Create
                                    | Keyword::Drop
                                    | Keyword::Alter
                                    | Keyword::Merge
                                    | Keyword::With
                            ))
                    )
                {
                    Ok(Statement::Procedural(ProceduralStatement::Throw {
                        error_number: None,
                        message: None,
                        state: None,
                    }))
                } else {
                    let error_number = parse_expr(parser)?;
                    parser.expect_comma()?;
                    let message = parse_expr(parser)?;
                    parser.expect_comma()?;
                    let state = parse_expr(parser)?;
                    Ok(Statement::Procedural(ProceduralStatement::Throw {
                        error_number: Some(error_number),
                        message: Some(message),
                        state: Some(state),
                    }))
                }
            }
            Keyword::Break => {
                let _ = parser.next();
                Ok(Statement::Procedural(ProceduralStatement::Break))
            }
            Keyword::Continue => {
                let _ = parser.next();
                Ok(Statement::Procedural(ProceduralStatement::Continue))
            }
            Keyword::Return => {
                let _ = parser.next();
                let next_is_label = matches!(parser.peek_at(1), Some(Token::Colon))
                    && !matches!(parser.peek_at(2), Some(Token::Colon));
                let expr = if !parser.is_empty()
                    && !next_is_label
                    && !matches!(
                        parser.peek(),
                        Some(Token::Semicolon)
                            | Some(Token::Go)
                            | Some(Token::Keyword(
                                Keyword::End
                                    | Keyword::Else
                                    | Keyword::Select
                                    | Keyword::Insert
                                    | Keyword::Update
                                    | Keyword::Delete
                                    | Keyword::Declare
                                    | Keyword::Set
                                    | Keyword::If
                                    | Keyword::While
                                    | Keyword::Begin
                                    | Keyword::Exec
                                    | Keyword::Execute
                                    | Keyword::Return
                                    | Keyword::Break
                                    | Keyword::Continue
                                    | Keyword::Print
                                    | Keyword::Throw
                                    | Keyword::RaiseError
                                    | Keyword::Commit
                                    | Keyword::Rollback
                                    | Keyword::Save
                                    | Keyword::Create
                                    | Keyword::Drop
                                    | Keyword::Alter
                                    | Keyword::Merge
                                    | Keyword::With
                            ))
                    ) {
                    Some(parse_expr(parser)?)
                } else {
                    None
                };
                Ok(Statement::Procedural(ProceduralStatement::Return(expr)))
            }
            Keyword::Begin => {
                let _ = parser.next();
                parse_begin_dispatch(parser)
            }
            Keyword::Commit => {
                let _ = parser.next();
                statements::transaction::parse_commit_transaction(parser)
            }
            Keyword::Rollback => {
                let _ = parser.next();
                statements::transaction::parse_rollback_transaction(parser)
            }
            Keyword::Save => {
                let _ = parser.next();
                statements::transaction::parse_save_transaction(parser)
            }
            Keyword::Open => {
                let _ = parser.next();
                statements::cursor::parse_open_cursor(parser)
            }
            Keyword::Close => {
                let _ = parser.next();
                statements::cursor::parse_close_cursor(parser)
            }
            Keyword::Deallocate => {
                let _ = parser.next();
                statements::cursor::parse_deallocate_cursor(parser)
            }
            Keyword::Fetch => {
                let _ = parser.next();
                statements::cursor::parse_fetch_cursor(parser)
            }
            Keyword::Checkpoint => {
                let _ = parser.next();
                // Optional checkpoint_duration (integer literal or variable).
                if matches!(
                    parser.peek(),
                    Some(Token::Number { .. }) | Some(Token::Variable(_))
                ) {
                    let _ = parser.next();
                }
                Ok(Statement::Session(SessionStatement::Checkpoint))
            }
            _ => parser.backtrack(Expected::Description("statement keyword")),
        },
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("REVERT") => {
            let _ = parser.next();
            if parser.at_keyword(Keyword::With) {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("COOKIE"))
                {
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                        let _ = parser.next();
                        let _ = parse_expr(parser)?;
                    }
                }
            }
            Ok(Statement::Session(SessionStatement::Revert))
        }
        Some(Token::Identifier(id))
            if id.eq_ignore_ascii_case("DISABLE") || id.eq_ignore_ascii_case("ENABLE") =>
        {
            let enable = id.eq_ignore_ascii_case("ENABLE");
            let _ = parser.next();
            parser.expect_keyword(Keyword::Trigger)?;
            let trigger_name = if parser.at_keyword(Keyword::All) {
                let _ = parser.next();
                None
            } else {
                Some(multipart_name(parser)?)
            };
            parser.expect_keyword(Keyword::On)?;
            let table = if parser.at_keyword(Keyword::Database) {
                let _ = parser.next();
                vec!["__DATABASE__".to_string()]
            } else if parser.at_keyword(Keyword::All)
                && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SERVER"))
            {
                let _ = parser.next();
                let _ = parser.next();
                vec!["__DATABASE__".to_string()]
            } else {
                multipart_name(parser)?
            };
            Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                table,
                action: AlterTableAction::ToggleTrigger {
                    trigger_name,
                    enable,
                },
            })))
        }
        Some(Token::Identifier(id))
            if id.eq_ignore_ascii_case("GRANT")
                || id.eq_ignore_ascii_case("REVOKE")
                || id.eq_ignore_ascii_case("DENY") =>
        {
            parse_dcl_permission_stmt(parser)
        }
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("DBCC") => {
            let _ = parser.next();
            let cmd = match parser.next() {
                Some(Token::Identifier(c)) => c.clone(),
                Some(Token::Keyword(k)) => k.as_sql().to_string(),
                _ => return parser.backtrack(Expected::Description("DBCC command")),
            };
            if !cmd.eq_ignore_ascii_case("CHECKIDENT") {
                return parser.backtrack(Expected::Description("supported DBCC command"));
            }
            parser.expect_lparen()?;
            let table = parse_expr(parser)?;
            let mut reseed = true;
            let mut new_reseed_value = None;
            if matches!(parser.peek(), Some(Token::Comma)) {
                let _ = parser.next();
                match parser.next() {
                    Some(Token::Identifier(m)) if m.eq_ignore_ascii_case("NORESEED") => {
                        reseed = false;
                    }
                    Some(Token::Identifier(m)) if m.eq_ignore_ascii_case("RESEED") => {
                        reseed = true;
                    }
                    _ => return parser.backtrack(Expected::Description("RESEED or NORESEED")),
                }
                if matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                    new_reseed_value = Some(parse_expr(parser)?);
                }
            }
            parser.expect_rparen()?;
            if parser.at_keyword(Keyword::With) {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("NO_INFOMSGS"))
                {
                    let _ = parser.next();
                }
            }
            Ok(Statement::Procedural(ProceduralStatement::DbccCheckIdent {
                table,
                reseed,
                new_reseed_value,
            }))
        }
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("GOTO") => {
            let _ = parser.next();
            let label = match parser.next() {
                Some(Token::Identifier(lbl)) => lbl.clone(),
                Some(Token::Keyword(k)) => k.as_sql().to_string(),
                _ => return parser.backtrack(Expected::Description("GOTO label name")),
            };
            Ok(Statement::Procedural(ProceduralStatement::Goto(label)))
        }
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("WAITFOR") => {
            let _ = parser.next();
            // WAITFOR DELAY <expr> | WAITFOR TIME <expr>
            match parser.next() {
                Some(Token::Identifier(k)) if k.eq_ignore_ascii_case("DELAY") => {}
                Some(Token::Keyword(Keyword::Time)) => {}
                _ => return parser.backtrack(Expected::Description("DELAY or TIME")),
            }
            let _ = parse_expr(parser)?;
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        }
        _ => parser.backtrack(Expected::Description("statement keyword")),
    }
}

fn is_to_token(tok: Option<&Token>) -> bool {
    matches!(tok, Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TO"))
}

fn parse_dcl_permission_stmt(parser: &mut Parser) -> ParseResult<Statement> {
    let action = match parser.next() {
        Some(Token::Identifier(id)) => id.to_ascii_uppercase(),
        Some(Token::Keyword(k)) => k.as_sql().to_ascii_uppercase(),
        _ => "GRANT".to_string(),
    };
    // Parse permission list (comma-separated phrases like `SELECT`, `VIEW DEFINITION`, `EXECUTE`)
    let mut permissions: Vec<String> = Vec::new();
    let mut cur_perm = String::new();
    while !parser.is_empty()
        && !matches!(parser.peek(), Some(Token::Semicolon) | Some(Token::Go))
        && !parser.at_keyword(Keyword::On)
        && !is_to_token(parser.peek())
        && !parser.at_keyword(Keyword::From)
    {
        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
            let trimmed = cur_perm.trim().to_string();
            if !trimmed.is_empty() {
                permissions.push(trimmed);
            }
            cur_perm.clear();
        } else if matches!(parser.peek(), Some(Token::LParen)) {
            let _ = parser.next();
            let mut depth = 1usize;
            while depth > 0 {
                match parser.next() {
                    Some(Token::LParen) => depth += 1,
                    Some(Token::RParen) => depth -= 1,
                    Some(_) => {}
                    None => return parser.backtrack(Expected::Description(")")),
                }
            }
        } else {
            match parser.next() {
                Some(Token::Identifier(id)) => {
                    if !cur_perm.is_empty() {
                        cur_perm.push(' ');
                    }
                    cur_perm.push_str(id);
                }
                Some(Token::Keyword(k)) => {
                    if !cur_perm.is_empty() {
                        cur_perm.push(' ');
                    }
                    cur_perm.push_str(k.as_sql());
                }
                _ => {}
            }
        }
    }
    let trimmed = cur_perm.trim().to_string();
    if !trimmed.is_empty() {
        permissions.push(trimmed);
    }

    // Optional ON [class::]object[(col, ...)]
    let mut target_class: Option<String> = None;
    let mut target_name: Option<crate::ast::ObjectName> = None;
    if parser.at_keyword(Keyword::On) {
        let _ = parser.next();
        if matches!(
            parser.peek(),
            Some(Token::Identifier(_)) | Some(Token::Keyword(_))
        ) {
            let saved = parser.save();
            let cls_tok = match parser.next() {
                Some(Token::Identifier(id)) => id.to_ascii_uppercase(),
                Some(Token::Keyword(k)) => k.as_sql().to_ascii_uppercase(),
                _ => String::new(),
            };
            if matches!(parser.peek(), Some(Token::Operator(op)) if op == "::") {
                let _ = parser.next();
                target_class = Some(cls_tok);
            } else {
                parser.restore(saved);
            }
        }
        if !is_to_token(parser.peek()) && !parser.at_keyword(Keyword::From) {
            let parts = multipart_name(parser)?;
            target_name = Some(crate::parser::lower::common::lower_object_name(parts));
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
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
        }
    }
    // TO | FROM <principal>, ...
    let mut grantees: Vec<String> = Vec::new();
    if is_to_token(parser.peek()) || parser.at_keyword(Keyword::From) {
        let _ = parser.next();
        loop {
            match parser.next() {
                Some(Token::Identifier(id)) => grantees.push(id.clone()),
                Some(Token::Keyword(k)) => grantees.push(k.as_sql().to_string()),
                _ => return parser.backtrack(Expected::Description("principal name")),
            }
            if matches!(parser.peek(), Some(Token::Comma)) {
                let _ = parser.next();
            } else {
                break;
            }
        }
    }
    // Optional trailing WITH GRANT OPTION / CASCADE / AS <principal>
    let mut with_grant_option = false;
    if parser.at_keyword(Keyword::With) {
        let saved = parser.save();
        let _ = parser.next();
        if matches!(
            parser.peek(),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("GRANT")
        ) {
            let _ = parser.next();
            with_grant_option = true;
            if matches!(
                parser.peek(),
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OPTION")
            ) {
                let _ = parser.next();
            }
        } else {
            parser.restore(saved);
        }
    }
    if parser.at_keyword(Keyword::Cascade) {
        let _ = parser.next();
    }
    if parser.at_keyword(Keyword::As) {
        let _ = parser.next();
        let _ = parser.next();
    }
    Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
        Box::new(crate::ast::CatalogMetadataAction::Permission {
            action,
            permissions,
            target_class,
            target_name,
            grantees,
            with_grant_option,
        }),
    ))))
}

fn parse_declare_dispatch(parser: &mut Parser) -> ParseResult<Statement> {
    if let Some(Token::Variable(var_name)) = parser.peek() {
        let var_name = var_name.clone();
        let next_is_table = matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Table)));
        let next_is_as_table = matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::As)))
            && matches!(parser.peek_at(2), Some(Token::Keyword(Keyword::Table)));
        if next_is_table || next_is_as_table {
            let _ = parser.next();
            if next_is_as_table {
                let _ = parser.next();
            }
            let _ = parser.next();
            parser.expect_lparen()?;
            let (columns, constraints) = parse_table_body(parser)?;
            parser.expect_rparen()?;
            return Ok(Statement::Procedural(
                ProceduralStatement::DeclareTableVar {
                    name: var_name,
                    columns,
                    constraints,
                },
            ));
        }
    }
    if let Some(Token::Identifier(cursor_name)) = parser.peek() {
        let is_cursor_decl = matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Cursor)))
            || (matches!(
                parser.peek_at(1),
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("INSENSITIVE") || id.eq_ignore_ascii_case("SCROLL")
            ) && matches!(parser.peek_at(2), Some(Token::Keyword(Keyword::Cursor))));
        if is_cursor_decl {
            let cursor_name = cursor_name.clone();
            let _ = parser.next();
            if !parser.at_keyword(Keyword::Cursor) {
                let _ = parser.next();
            }
            let _ = parser.next(); // CURSOR
                                   // Optional T-SQL cursor options before FOR (LOCAL, GLOBAL, FORWARD_ONLY, SCROLL,
                                   // STATIC, KEYSET, DYNAMIC, FAST_FORWARD, READ_ONLY, SCROLL_LOCKS, OPTIMISTIC, TYPE_WARNING).
            while !parser.at_keyword(Keyword::For) && !parser.is_empty() {
                match parser.peek() {
                    Some(Token::Identifier(id))
                        if matches!(
                            id.to_ascii_uppercase().as_str(),
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
                        ) =>
                    {
                        let _ = parser.next();
                    }
                    Some(Token::Keyword(k))
                        if matches!(
                            k.as_sql().to_ascii_uppercase().as_str(),
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
                        ) =>
                    {
                        let _ = parser.next();
                    }
                    _ => break,
                }
            }
            parser.expect_keyword(Keyword::For)?;
            let query = parse_select(parser)?;
            return Ok(Statement::Procedural(ProceduralStatement::DeclareCursor {
                name: cursor_name,
                query: Box::new(query),
            }));
        }
    }
    Ok(Statement::Procedural(ProceduralStatement::Declare(
        parse_declare(parser)?,
    )))
}

fn parse_set_dispatch(parser: &mut Parser) -> ParseResult<Statement> {
    if parser.at_keyword(Keyword::Transaction) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Isolation)?;
        parser.expect_keyword(Keyword::Level)?;
        let mut level_keywords = Vec::new();
        while let Some(Token::Keyword(k)) = parser.peek() {
            if matches!(
                k,
                Keyword::Read
                    | Keyword::Uncommitted
                    | Keyword::Committed
                    | Keyword::Repeatable
                    | Keyword::Serializable
                    | Keyword::Snapshot
            ) {
                level_keywords.push(*k);
                let _ = parser.next();
            } else {
                break;
            }
        }
        let iso = match level_keywords.as_slice() {
            [Keyword::Read, Keyword::Uncommitted] => {
                crate::parser::ast::IsolationLevel::ReadUncommitted
            }
            [Keyword::Read, Keyword::Committed] => {
                crate::parser::ast::IsolationLevel::ReadCommitted
            }
            [Keyword::Repeatable, Keyword::Read] => {
                crate::parser::ast::IsolationLevel::RepeatableRead
            }
            [Keyword::Serializable] => crate::parser::ast::IsolationLevel::Serializable,
            [Keyword::Snapshot] => crate::parser::ast::IsolationLevel::Snapshot,
            _ => crate::parser::ast::IsolationLevel::ReadCommitted,
        };
        return Ok(Statement::Session(
            SessionStatement::SetTransactionIsolationLevel(iso),
        ));
    }
    if matches_set_name(parser.peek(), "IDENTITY_INSERT") || parser.at_keyword(Keyword::Identity) {
        let _ = parser.next();
        if matches_set_name(parser.peek(), "INSERT") || parser.at_keyword(Keyword::Insert) {
            let _ = parser.next();
        }
        let table = multipart_name(parser)?;
        let on = match parser.next() {
            Some(Token::Keyword(k)) if *k == Keyword::On => true,
            Some(Token::Keyword(k)) if *k == Keyword::Off => false,
            _ => return parser.backtrack(Expected::Description("ON or OFF")),
        };
        return Ok(Statement::Session(SessionStatement::SetIdentityInsert {
            table,
            on,
        }));
    }

    fn matches_set_name(tok: Option<&Token>, expected: &str) -> bool {
        match tok {
            Some(Token::Identifier(id)) => id.eq_ignore_ascii_case(expected),
            Some(Token::Keyword(k)) => k.as_ref().eq_ignore_ascii_case(expected),
            _ => false,
        }
    }

    fn match_bool_session_option(tok: Option<&Token>) -> Option<crate::parser::ast::SessionOption> {
        let name = match tok {
            Some(Token::Identifier(id)) => id.as_str(),
            Some(Token::Keyword(k)) => k.as_ref(),
            _ => return None,
        };
        if name.eq_ignore_ascii_case("ANSI_DEFAULTS") {
            Some(crate::parser::ast::SessionOption::AnsiDefaults)
        } else if name.eq_ignore_ascii_case("ANSI_NULLS") {
            Some(crate::parser::ast::SessionOption::AnsiNulls)
        } else if name.eq_ignore_ascii_case("QUOTED_IDENTIFIER") {
            Some(crate::parser::ast::SessionOption::QuotedIdentifier)
        } else if name.eq_ignore_ascii_case("NOCOUNT") {
            Some(crate::parser::ast::SessionOption::NoCount)
        } else if name.eq_ignore_ascii_case("XACT_ABORT") {
            Some(crate::parser::ast::SessionOption::XactAbort)
        } else if name.eq_ignore_ascii_case("FMTONLY") {
            Some(crate::parser::ast::SessionOption::FmtOnly)
        } else if name.eq_ignore_ascii_case("NOEXEC") {
            Some(crate::parser::ast::SessionOption::NoExec)
        } else if name.eq_ignore_ascii_case("SHOWPLAN_XML") {
            Some(crate::parser::ast::SessionOption::ShowplanXml)
        } else if name.eq_ignore_ascii_case("STATISTICS_XML") {
            Some(crate::parser::ast::SessionOption::StatisticsXml)
        } else if name.eq_ignore_ascii_case("ANSI_NULL_DFLT_ON") {
            Some(crate::parser::ast::SessionOption::AnsiNullDfltOn)
        } else if name.eq_ignore_ascii_case("ANSI_PADDING") {
            Some(crate::parser::ast::SessionOption::AnsiPadding)
        } else if name.eq_ignore_ascii_case("ANSI_WARNINGS") {
            Some(crate::parser::ast::SessionOption::AnsiWarnings)
        } else if name.eq_ignore_ascii_case("ARITHABORT") {
            Some(crate::parser::ast::SessionOption::ArithAbort)
        } else if name.eq_ignore_ascii_case("CONCAT_NULL_YIELDS_NULL") {
            Some(crate::parser::ast::SessionOption::ConcatNullYieldsNull)
        } else if name.eq_ignore_ascii_case("CURSOR_CLOSE_ON_COMMIT") {
            Some(crate::parser::ast::SessionOption::CursorCloseOnCommit)
        } else if name.eq_ignore_ascii_case("IMPLICIT_TRANSACTIONS") {
            Some(crate::parser::ast::SessionOption::ImplicitTransactions)
        } else if name.eq_ignore_ascii_case("NUMERIC_ROUNDABORT")
            || name.eq_ignore_ascii_case("ARITHIGNORE")
            || name.eq_ignore_ascii_case("ANSI_NULL_DFLT_OFF")
            || name.eq_ignore_ascii_case("FORCEPLAN")
            || name.eq_ignore_ascii_case("PARSEONLY")
            || name.eq_ignore_ascii_case("REMOTE_PROC_TRANSACTIONS")
        {
            Some(crate::parser::ast::SessionOption::Unsupported(
                name.to_string(),
            ))
        } else {
            None
        }
    }

    fn parse_bool_setting(
        parser: &mut Parser,
        first_option: crate::parser::ast::SessionOption,
    ) -> ParseResult<Statement> {
        let _ = parser.next();
        let mut options = vec![first_option];
        while matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
            let Some(next_opt) = match_bool_session_option(parser.peek()) else {
                return parser.backtrack(Expected::Description("SET option name"));
            };
            let _ = parser.next();
            options.push(next_opt);
        }
        let val = match parser.next() {
            Some(Token::Keyword(k)) if *k == Keyword::On => true,
            Some(Token::Keyword(k)) if *k == Keyword::Off => false,
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => true,
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => false,
            _ => return parser.backtrack(Expected::Description("ON or OFF")),
        };
        if options.len() == 1 {
            Ok(Statement::Session(SessionStatement::SetOption {
                option: options.into_iter().next().unwrap(),
                value: crate::parser::ast::SessionOptionValue::Bool(val),
            }))
        } else {
            let stmts = options
                .into_iter()
                .map(|option| {
                    Statement::Session(SessionStatement::SetOption {
                        option,
                        value: crate::parser::ast::SessionOptionValue::Bool(val),
                    })
                })
                .collect();
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(stmts)))
        }
    }

    fn parse_text_setting(
        parser: &mut Parser,
        option: crate::parser::ast::SessionOption,
    ) -> ParseResult<Statement> {
        let _ = parser.next();
        let text = match parser.next() {
            Some(Token::String(s)) => s.clone(),
            Some(Token::Identifier(id)) => id.clone(),
            Some(Token::Keyword(k)) => k.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("text")),
        };
        Ok(Statement::Session(SessionStatement::SetOption {
            option,
            value: crate::parser::ast::SessionOptionValue::Text(text),
        }))
    }

    fn parse_signed_int(parser: &mut Parser) -> ParseResult<i64> {
        let sign = if matches!(parser.peek(), Some(Token::Operator(op)) if *op == "-") {
            let _ = parser.next();
            -1i64
        } else if matches!(parser.peek(), Some(Token::Operator(op)) if *op == "+") {
            let _ = parser.next();
            1i64
        } else {
            1i64
        };

        if let Some(Token::Number { value: n, .. }) = parser.next() {
            Ok(sign * (*n as i64))
        } else {
            parser.backtrack(Expected::Description("number"))
        }
    }

    fn is_set_boundary(tok: Option<&Token>) -> bool {
        matches!(tok, None | Some(Token::Semicolon) | Some(Token::Go))
            || matches!(tok, Some(Token::Keyword(Keyword::Set)))
    }

    fn parse_generic_set_option(
        parser: &mut Parser,
        option_name: String,
    ) -> ParseResult<Statement> {
        let value = match parser.peek() {
            Some(Token::Keyword(k)) if *k == Keyword::On => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Bool(true)
            }
            Some(Token::Keyword(k)) if *k == Keyword::Off => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Bool(false)
            }
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Bool(true)
            }
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Bool(false)
            }
            Some(Token::Number { .. }) => {
                crate::parser::ast::SessionOptionValue::Int(parse_signed_int(parser)?)
            }
            Some(Token::Operator(op)) if *op == "-" || *op == "+" => {
                crate::parser::ast::SessionOptionValue::Int(parse_signed_int(parser)?)
            }
            Some(_) => {
                let mut parts = Vec::new();
                while !is_set_boundary(parser.peek()) {
                    let Some(tok) = parser.next() else { break };
                    match tok {
                        Token::Identifier(v)
                        | Token::String(v)
                        | Token::NString(v)
                        | Token::Variable(v) => parts.push(v.clone()),
                        Token::Keyword(k) => parts.push(k.as_ref().to_string()),
                        Token::Number { raw, .. } => parts.push(raw.clone()),
                        Token::Operator(op) => parts.push(op.clone()),
                        Token::Comma => parts.push(",".to_string()),
                        Token::Colon => parts.push(":".to_string()),
                        Token::Dot => parts.push(".".to_string()),
                        Token::LParen => parts.push("(".to_string()),
                        Token::RParen => parts.push(")".to_string()),
                        Token::Star => parts.push("*".to_string()),
                        Token::Tilde => parts.push("~".to_string()),
                        Token::BinaryLiteral(v) => parts.push(v.clone()),
                        Token::Semicolon | Token::Go => break,
                    }
                }
                if parts.is_empty() {
                    return parser.backtrack(Expected::Description("SET option value"));
                }
                crate::parser::ast::SessionOptionValue::Text(parts.join(" "))
            }
            None => return parser.backtrack(Expected::Description("SET option value")),
        };

        Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::Unsupported(option_name),
            value,
        }))
    }

    if matches_set_name(parser.peek(), "ANSI_DEFAULTS") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::AnsiDefaults);
    }
    if matches_set_name(parser.peek(), "ANSI_NULLS") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::AnsiNulls);
    }
    if matches_set_name(parser.peek(), "QUOTED_IDENTIFIER") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::QuotedIdentifier);
    }
    if matches_set_name(parser.peek(), "NOCOUNT") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::NoCount);
    }
    if matches_set_name(parser.peek(), "XACT_ABORT") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::XactAbort);
    }
    if matches_set_name(parser.peek(), "FMTONLY") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::FmtOnly);
    }
    if matches_set_name(parser.peek(), "NOEXEC") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::NoExec);
    }
    if matches_set_name(parser.peek(), "SHOWPLAN_XML") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::ShowplanXml);
    }
    if matches_set_name(parser.peek(), "STATISTICS_XML") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::StatisticsXml);
    }
    if matches_set_name(parser.peek(), "ANSI_NULL_DFLT_ON") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::AnsiNullDfltOn);
    }
    if matches_set_name(parser.peek(), "ANSI_PADDING") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::AnsiPadding);
    }
    if matches_set_name(parser.peek(), "ANSI_WARNINGS") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::AnsiWarnings);
    }
    if matches_set_name(parser.peek(), "ARITHABORT") {
        return parse_bool_setting(parser, crate::parser::ast::SessionOption::ArithAbort);
    }
    if matches_set_name(parser.peek(), "CONCAT_NULL_YIELDS_NULL") {
        return parse_bool_setting(
            parser,
            crate::parser::ast::SessionOption::ConcatNullYieldsNull,
        );
    }
    if matches_set_name(parser.peek(), "CURSOR_CLOSE_ON_COMMIT") {
        return parse_bool_setting(
            parser,
            crate::parser::ast::SessionOption::CursorCloseOnCommit,
        );
    }
    if matches_set_name(parser.peek(), "IMPLICIT_TRANSACTIONS") {
        return parse_bool_setting(
            parser,
            crate::parser::ast::SessionOption::ImplicitTransactions,
        );
    }
    if matches_set_name(parser.peek(), "DATEFIRST") {
        let _ = parser.next();
        let val = parse_signed_int(parser)?;
        return Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::DateFirst,
            value: crate::parser::ast::SessionOptionValue::Int(val),
        }));
    }
    if matches_set_name(parser.peek(), "LANGUAGE") {
        return parse_text_setting(parser, crate::parser::ast::SessionOption::Language);
    }
    if matches_set_name(parser.peek(), "DATEFORMAT") {
        return parse_text_setting(parser, crate::parser::ast::SessionOption::DateFormat);
    }
    if matches_set_name(parser.peek(), "LOCK_TIMEOUT") {
        let _ = parser.next();
        let val = parse_signed_int(parser)?;
        return Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::LockTimeout,
            value: crate::parser::ast::SessionOptionValue::Int(val),
        }));
    }
    if matches_set_name(parser.peek(), "ROWCOUNT") {
        let _ = parser.next();
        let val = parse_signed_int(parser)?;
        return Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::RowCount,
            value: crate::parser::ast::SessionOptionValue::Int(val),
        }));
    }
    if matches_set_name(parser.peek(), "TEXTSIZE") {
        let _ = parser.next();
        let val = parse_signed_int(parser)?;
        return Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::TextSize,
            value: crate::parser::ast::SessionOptionValue::Int(val),
        }));
    }
    if matches_set_name(parser.peek(), "QUERY_GOVERNOR_COST_LIMIT") {
        let _ = parser.next();
        let val = parse_signed_int(parser)?;
        return Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::QueryGovernorCostLimit,
            value: crate::parser::ast::SessionOptionValue::Int(val),
        }));
    }
    if matches_set_name(parser.peek(), "DEADLOCK_PRIORITY") {
        let _ = parser.next();
        let value = match parser.peek() {
            Some(Token::Keyword(Keyword::Low)) => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Text("LOW".to_string())
            }
            Some(Token::Keyword(Keyword::Normal)) => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Text("NORMAL".to_string())
            }
            Some(Token::Keyword(Keyword::High)) => {
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Text("HIGH".to_string())
            }
            Some(Token::Number { .. }) | Some(Token::Operator(_)) => {
                crate::parser::ast::SessionOptionValue::Int(parse_signed_int(parser)?)
            }
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("LOW")
                    || id.eq_ignore_ascii_case("NORMAL")
                    || id.eq_ignore_ascii_case("HIGH") =>
            {
                let value = id.clone();
                let _ = parser.next();
                crate::parser::ast::SessionOptionValue::Text(value)
            }
            _ => return parser.backtrack(Expected::Description("deadlock priority")),
        };
        return Ok(Statement::Session(SessionStatement::SetOption {
            option: crate::parser::ast::SessionOption::DeadlockPriority,
            value,
        }));
    }

    if parser.at_keyword(Keyword::Statistics) {
        let _ = parser.next();
        if parser.at_keyword(Keyword::Io) {
            let _ = parser.next();
            // IO already consumed; parse ON/OFF directly instead of parse_bool_setting
            let val = match parser.next() {
                Some(Token::Keyword(k)) if *k == Keyword::On => true,
                Some(Token::Keyword(k)) if *k == Keyword::Off => false,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => true,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => false,
                _ => return parser.backtrack(Expected::Description("ON or OFF")),
            };
            return Ok(Statement::Session(SessionStatement::SetOption {
                option: crate::parser::ast::SessionOption::StatisticsIo,
                value: crate::parser::ast::SessionOptionValue::Bool(val),
            }));
        }
        if parser.at_keyword(Keyword::Time) {
            let _ = parser.next();
            let val = match parser.next() {
                Some(Token::Keyword(k)) if *k == Keyword::On => true,
                Some(Token::Keyword(k)) if *k == Keyword::Off => false,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => true,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => false,
                _ => return parser.backtrack(Expected::Description("ON or OFF")),
            };
            return Ok(Statement::Session(SessionStatement::SetOption {
                option: crate::parser::ast::SessionOption::StatisticsTime,
                value: crate::parser::ast::SessionOptionValue::Bool(val),
            }));
        }
        if parser.at_keyword(Keyword::Xml) {
            let _ = parser.next();
            let val = match parser.next() {
                Some(Token::Keyword(k)) if *k == Keyword::On => true,
                Some(Token::Keyword(k)) if *k == Keyword::Off => false,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => true,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => false,
                _ => return parser.backtrack(Expected::Description("ON or OFF")),
            };
            return Ok(Statement::Session(SessionStatement::SetOption {
                option: crate::parser::ast::SessionOption::StatisticsXml,
                value: crate::parser::ast::SessionOptionValue::Bool(val),
            }));
        }
    }
    if parser.at_keyword(Keyword::Showplan) {
        let _ = parser.next();
        if parser.at_keyword(Keyword::All) {
            let _ = parser.next();
            let val = match parser.next() {
                Some(Token::Keyword(k)) if *k == Keyword::On => true,
                Some(Token::Keyword(k)) if *k == Keyword::Off => false,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => true,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => false,
                _ => return parser.backtrack(Expected::Description("ON or OFF")),
            };
            return Ok(Statement::Session(SessionStatement::SetOption {
                option: crate::parser::ast::SessionOption::ShowplanAll,
                value: crate::parser::ast::SessionOptionValue::Bool(val),
            }));
        }
        if parser.at_keyword(Keyword::Xml) {
            let _ = parser.next();
            let val = match parser.next() {
                Some(Token::Keyword(k)) if *k == Keyword::On => true,
                Some(Token::Keyword(k)) if *k == Keyword::Off => false,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ON") => true,
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("OFF") => false,
                _ => return parser.backtrack(Expected::Description("ON or OFF")),
            };
            return Ok(Statement::Session(SessionStatement::SetOption {
                option: crate::parser::ast::SessionOption::ShowplanXml,
                value: crate::parser::ast::SessionOptionValue::Bool(val),
            }));
        }
    }

    if let Some(tok) = parser.peek() {
        let option_name = match tok {
            Token::Identifier(id) => Some(id.clone()),
            Token::Keyword(k) => Some(k.as_ref().to_string()),
            _ => None,
        };
        if let Some(option_name) = option_name {
            let _ = parser.next();
            return parse_generic_set_option(parser, option_name);
        }
    }
    parse_set(parser)
}

fn parse_use_database_name(parser: &mut Parser) -> ParseResult<String> {
    match parser.next() {
        Some(Token::Identifier(id)) => Ok(id.clone()),
        Some(Token::Keyword(k)) => Ok(k.as_ref().to_string()),
        Some(Token::String(s)) => Ok(s.clone()),
        Some(Token::NString(s)) => Ok(s.clone()),
        _ => parser.backtrack(Expected::Description("database name")),
    }
}

fn parse_begin_dispatch(parser: &mut Parser) -> ParseResult<Statement> {
    if parser.at_keyword(Keyword::Distributed) {
        let _ = parser.next();
    }
    if let Some(Token::Keyword(k2)) = parser.peek() {
        if *k2 == Keyword::Try {
            let _ = parser.next();
            return parse_try_catch(parser);
        }
        if matches!(k2, Keyword::Tran | Keyword::Transaction) {
            return statements::transaction::parse_begin_transaction(parser);
        }
    }
    parse_begin_end(parser)
}

pub(crate) fn parse_cte_def(parser: &mut Parser) -> ParseResult<CteDef> {
    let name = if let Some(tok) = parser.next() {
        match tok {
            Token::Identifier(id) => id.clone(),
            Token::Keyword(k) => k.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("identifier or keyword")),
        }
    } else {
        return parser.backtrack(Expected::Description("identifier or keyword"));
    };

    let mut columns = Vec::new();
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        columns = parse_comma_list(parser, |p| {
            if let Some(tok) = p.next() {
                match tok {
                    Token::Identifier(id) => Ok(id.clone()),
                    Token::Keyword(k) => Ok(k.as_ref().to_string()),
                    _ => p.backtrack(Expected::Description("column name")),
                }
            } else {
                p.backtrack(Expected::Description("column name"))
            }
        })?;
        parser.expect_rparen()?;
    }

    parser.expect_keyword(Keyword::As)?;
    parser.expect_lparen()?;
    let query = parse_select(parser)?;
    parser.expect_rparen()?;

    Ok(CteDef {
        name,
        columns,
        query,
    })
}

pub fn parse_routine_param(parser: &mut Parser) -> ParseResult<RoutineParam> {
    let name = if let Some(Token::Variable(v)) = parser.next() {
        v.clone()
    } else {
        return parser.backtrack(Expected::Description("variable"));
    };
    if parser.at_keyword(Keyword::As) {
        let _ = parser.next();
    }
    let data_type = parse_data_type(parser)?;
    let mut is_output = false;
    let mut is_readonly = false;
    let mut default = None;

    loop {
        if matches!(parser.peek(), Some(Token::Keyword(k)) if matches!(k, Keyword::Output | Keyword::Out))
        {
            let _ = parser.next();
            is_output = true;
            continue;
        }
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("READONLY"))
            || matches!(parser.peek(), Some(Token::Keyword(k)) if k.as_sql().eq_ignore_ascii_case("READONLY"))
        {
            let _ = parser.next();
            is_readonly = true;
            continue;
        }
        if default.is_none() && matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
            let _ = parser.next();
            default = Some(parse_expr(parser)?);
            continue;
        }
        if parser.at_keyword(Keyword::Null) {
            let _ = parser.next();
            if default.is_none() {
                default = Some(Expr::Null);
            }
            continue;
        }
        break;
    }

    Ok(RoutineParam {
        name,
        data_type,
        is_output,
        is_readonly,
        default,
    })
}

fn try_select_assign(select: &SelectStmt) -> Option<Vec<SelectAssignTarget>> {
    let mut assigns = Vec::new();
    for item in &select.projection {
        if let Expr::Binary {
            left,
            op: BinaryOp::Eq,
            right,
        } = &item.expr
        {
            if let Expr::Variable(v) = &**left {
                assigns.push(SelectAssignTarget {
                    variable: v.clone(),
                    expr: (**right).clone(),
                });
                continue;
            }
        }
        return None;
    }
    if assigns.is_empty() {
        return None;
    }
    Some(assigns)
}
