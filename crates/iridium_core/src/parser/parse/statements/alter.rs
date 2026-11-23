use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

/// Skips remaining tokens of an administrative/storage/security DDL statement up
/// to the next statement boundary (`;`, `GO`, or top-level statement starter at
/// parenthesis depth 0).
pub(crate) fn skip_ddl_tail(parser: &mut Parser) {
    let mut depth = 0usize;
    while let Some(tok) = parser.peek() {
        if depth == 0
            && matches!(
                tok,
                Token::Semicolon
                    | Token::Go
                    | Token::Keyword(
                        Keyword::Create
                            | Keyword::Alter
                            | Keyword::Drop
                            | Keyword::Select
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
                            | Keyword::Print
                            | Keyword::While
                            | Keyword::Merge
                            | Keyword::Use
                            | Keyword::Truncate
                    )
            )
            || matches!(
                tok,
                Token::Identifier(id)
                    if id.eq_ignore_ascii_case("GRANT")
                        || id.eq_ignore_ascii_case("DENY")
                        || id.eq_ignore_ascii_case("REVOKE")
            )
        {
            break;
        }
        match parser.next() {
            Some(Token::LParen) => depth += 1,
            Some(Token::RParen) => depth = depth.saturating_sub(1),
            None => break,
            _ => {}
        }
    }
}

pub fn parse_alter(parser: &mut Parser) -> ParseResult<Statement> {
    if parser.at_keyword(Keyword::Table) {
        let _ = parser.next();
        let table = super::parse_multipart_name(parser)?;
        // Optional WITH CHECK / WITH NOCHECK prefix before ADD, CHECK, or NOCHECK
        let mut with_check: Option<bool> = None;
        if parser.at_keyword(Keyword::With) {
            let saved = parser.save();
            let _ = parser.next();
            if parser.at_keyword(Keyword::Check) {
                let _ = parser.next();
                with_check = Some(true);
            } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NOCHECK"))
            {
                let _ = parser.next();
                with_check = Some(false);
            } else {
                parser.restore(saved);
            }
        }
        let is_nocheck = matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NOCHECK"));
        if parser.at_keyword(Keyword::Check) || is_nocheck {
            let check = !is_nocheck;
            let _ = parser.next();
            parser.expect_keyword(Keyword::Constraint)?;
            let name = if parser.at_keyword(Keyword::All) {
                let _ = parser.next();
                None
            } else {
                match parser.next() {
                    Some(Token::Identifier(id)) => Some(id.clone()),
                    Some(Token::Keyword(k)) => Some(k.as_ref().to_string()),
                    _ => return parser.backtrack(Expected::Description("constraint name or ALL")),
                }
            };
            return Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                table,
                action: AlterTableAction::CheckConstraint {
                    check,
                    with_check,
                    name,
                },
            })));
        }
        if parser.at_keyword(Keyword::Add) {
            let _ = parser.next();
            let parse_add_item = |p: &mut Parser| -> ParseResult<AlterTableAction> {
                if p.at_keyword(Keyword::Constraint) {
                    let _ = p.next();
                    let constraint = super::ddl::parse_alter_table_add_constraint(p)?;
                    Ok(AlterTableAction::AddConstraint {
                        constraint,
                        with_check,
                    })
                } else if p.at_keyword(Keyword::Primary)
                    || p.at_keyword(Keyword::Unique)
                    || p.at_keyword(Keyword::Foreign)
                    || p.at_keyword(Keyword::Check)
                    || p.at_keyword(Keyword::Default)
                {
                    let constraint = super::ddl::parse_table_constraint(p)?;
                    Ok(AlterTableAction::AddConstraint {
                        constraint,
                        with_check,
                    })
                } else {
                    let col = super::ddl::parse_column_def(p)?;
                    Ok(AlterTableAction::AddColumn(col))
                }
            };
            let first = parse_add_item(parser)?;
            if !matches!(parser.peek(), Some(Token::Comma)) {
                Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                    table,
                    action: first,
                })))
            } else {
                let mut stmts = vec![Statement::Ddl(Box::new(DdlStatement::AlterTable {
                    table: table.clone(),
                    action: first,
                }))];
                while matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                    let next_action = parse_add_item(parser)?;
                    stmts.push(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                        table: table.clone(),
                        action: next_action,
                    })));
                }
                Ok(Statement::Procedural(ProceduralStatement::BeginEnd(stmts)))
            }
        } else if parser.at_keyword(Keyword::Alter) {
            let _ = parser.next();
            parser.expect_keyword(Keyword::Column)?;
            let col_name = match parser.next() {
                Some(Token::Identifier(id)) => id.clone(),
                Some(Token::Keyword(k)) => k.as_ref().to_string(),
                _ => return parser.backtrack(Expected::Description("column name")),
            };
            // ALTER TABLE t ALTER COLUMN c ADD MASKED WITH (FUNCTION = '...')
            // ALTER TABLE t ALTER COLUMN c DROP MASKED
            if parser.at_keyword(Keyword::Add)
                && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MASKED"))
            {
                let _ = parser.next(); // ADD
                let _ = parser.next(); // MASKED
                let mut mask_fn: Option<String> = None;
                if parser.at_keyword(Keyword::With) {
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if parser.at_keyword(Keyword::Function)
                            || matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("FUNCTION"))
                        {
                            let _ = parser.next();
                            if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                                let _ = parser.next();
                            }
                            match parser.next() {
                                Some(Token::String(func)) | Some(Token::NString(func)) => {
                                    mask_fn = Some(func.clone());
                                }
                                _ => {}
                            }
                        }
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
                return Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::SetColumnMask {
                        column: col_name,
                        masking_function: mask_fn,
                    },
                })));
            }
            if parser.at_keyword(Keyword::Drop)
                && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MASKED"))
            {
                let _ = parser.next(); // DROP
                let _ = parser.next(); // MASKED
                return Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::SetColumnMask {
                        column: col_name,
                        masking_function: None,
                    },
                })));
            }
            let data_type = crate::parser::parse::expressions::parse_data_type(parser)?;
            let mut nullable = None;
            if parser.at_keyword(Keyword::Not) {
                let _ = parser.next();
                parser.expect_keyword(Keyword::Null)?;
                nullable = Some(false);
            } else if parser.at_keyword(Keyword::Null) {
                let _ = parser.next();
                nullable = Some(true);
            }
            Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                table,
                action: AlterTableAction::AlterColumn {
                    name: col_name,
                    data_type,
                    nullable,
                },
            })))
        } else if parser.at_keyword(Keyword::Drop) {
            let _ = parser.next();
            let parse_ident = |p: &mut Parser, desc: &'static str| -> ParseResult<String> {
                match p.next() {
                    Some(Token::Identifier(id)) => Ok(id.clone()),
                    Some(Token::Keyword(k)) => Ok(k.as_ref().to_string()),
                    _ => p.backtrack(Expected::Description(desc)),
                }
            };
            if parser.at_keyword(Keyword::Column) {
                let _ = parser.next();
                // Optional `IF EXISTS`
                if parser.at_keyword(Keyword::If)
                    && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Exists)))
                {
                    let _ = parser.next();
                    let _ = parser.next();
                }
                let cols = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                    parse_ident(p, "column name")
                })?;
                if cols.len() == 1 {
                    Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                        table,
                        action: AlterTableAction::DropColumn(cols.into_iter().next().unwrap()),
                    })))
                } else {
                    let stmts = cols
                        .into_iter()
                        .map(|c| {
                            Statement::Ddl(Box::new(DdlStatement::AlterTable {
                                table: table.clone(),
                                action: AlterTableAction::DropColumn(c),
                            }))
                        })
                        .collect();
                    Ok(Statement::Procedural(ProceduralStatement::BeginEnd(stmts)))
                }
            } else if parser.at_keyword(Keyword::Constraint) {
                let _ = parser.next();
                if parser.at_keyword(Keyword::If)
                    && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Exists)))
                {
                    let _ = parser.next();
                    let _ = parser.next();
                }
                let names = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                    parse_ident(p, "constraint name")
                })?;
                if names.len() == 1 {
                    Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                        table,
                        action: AlterTableAction::DropConstraint(names.into_iter().next().unwrap()),
                    })))
                } else {
                    let stmts = names
                        .into_iter()
                        .map(|c| {
                            Statement::Ddl(Box::new(DdlStatement::AlterTable {
                                table: table.clone(),
                                action: AlterTableAction::DropConstraint(c),
                            }))
                        })
                        .collect();
                    Ok(Statement::Procedural(ProceduralStatement::BeginEnd(stmts)))
                }
            } else {
                parser.backtrack(Expected::Description("column or constraint"))
            }
        } else if parser.at_keyword(Keyword::Set) {
            // Standard SQL Server table option clauses:
            // ALTER TABLE [t] SET (SYSTEM_VERSIONING = ON/OFF (...), LOCK_ESCALATION = ...)
            let _ = parser.next();
            if let Some((enabled, history_table)) =
                super::create::parse_system_versioning_options(parser)?
            {
                Ok(Statement::Ddl(Box::new(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::SetSystemVersioning {
                        enabled,
                        history_table,
                    },
                })))
            } else {
                Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                    Vec::new(),
                )))
            }
        } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SWITCH"))
        {
            // ALTER TABLE [src] SWITCH [PARTITION n] TO [dst] [PARTITION m] [WITH (...)]
            let _ = parser.next();
            if parser.at_keyword(Keyword::Partition) {
                let _ = parser.next();
                let _ = crate::parser::parse::expressions::parse_expr(parser)?;
            }
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TO"))
            {
                let _ = parser.next();
                let _ = super::parse_multipart_name(parser)?;
                if parser.at_keyword(Keyword::Partition) {
                    let _ = parser.next();
                    let _ = crate::parser::parse::expressions::parse_expr(parser)?;
                }
            }
            if parser.at_keyword(Keyword::With) {
                let _ = parser.next();
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
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        } else {
            parser.backtrack(Expected::Description("add, alter, drop, or set"))
        }
    } else if parser.at_keyword(Keyword::Procedure) || parser.at_keyword(Keyword::Proc) {
        let _ = parser.next();
        let mut stmt = super::create::parse_create_procedure(parser)?;
        if let CreateStmt::Procedure {
            ref mut or_alter, ..
        } = stmt
        {
            *or_alter = true;
        }
        Ok(Statement::Ddl(Box::new(DdlStatement::Create(Box::new(
            stmt,
        )))))
    } else if parser.at_keyword(Keyword::View) {
        let _ = parser.next();
        let mut stmt = super::create::parse_create_view(parser)?;
        if let CreateStmt::View {
            ref mut or_alter, ..
        } = stmt
        {
            *or_alter = true;
        }
        Ok(Statement::Ddl(Box::new(DdlStatement::Create(Box::new(
            stmt,
        )))))
    } else if parser.at_keyword(Keyword::Function) {
        let _ = parser.next();
        let mut stmt = super::create::parse_create_function(parser)?;
        if let CreateStmt::Function {
            ref mut or_alter, ..
        } = stmt
        {
            *or_alter = true;
        }
        Ok(Statement::Ddl(Box::new(DdlStatement::Create(Box::new(
            stmt,
        )))))
    } else if parser.at_keyword(Keyword::Trigger) {
        let _ = parser.next();
        let mut stmt = super::create::parse_create_trigger(parser)?;
        if let CreateStmt::Trigger {
            ref mut or_alter, ..
        } = stmt
        {
            *or_alter = true;
        }
        Ok(Statement::Ddl(Box::new(DdlStatement::Create(Box::new(
            stmt,
        )))))
    } else if parser.at_keyword(Keyword::Sequence) {
        let _ = parser.next();
        parse_alter_sequence(parser)
    } else if parser.at_keyword(Keyword::Partition) {
        let _ = parser.next();
        let is_scheme = matches!(
            parser.peek(),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SCHEME")
        );
        let is_function = parser.at_keyword(Keyword::Function);
        if is_scheme {
            let _ = parser.next();
            let _ = super::parse_multipart_name(parser)?;
            if parser.at_keyword(Keyword::Next) {
                let _ = parser.next();
                if matches!(
                    parser.peek(),
                    Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("USED")
                ) {
                    let _ = parser.next();
                }
                if matches!(
                    parser.peek(),
                    Some(Token::Identifier(_)) | Some(Token::Keyword(_))
                ) && !matches!(
                    parser.peek(),
                    Some(Token::Keyword(
                        Keyword::Alter
                            | Keyword::Create
                            | Keyword::Drop
                            | Keyword::Select
                            | Keyword::Insert
                            | Keyword::Update
                            | Keyword::Delete
                            | Keyword::End
                            | Keyword::Set
                            | Keyword::Declare
                    ))
                ) {
                    let _ = parser.next();
                }
            }
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        } else if is_function {
            let _ = parser.next();
            let _ = super::parse_multipart_name(parser)?;
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                parser.expect_rparen()?;
            }
            // SPLIT RANGE (expr) | MERGE RANGE (expr)
            if parser.at_keyword(Keyword::Merge)
                || matches!(
                    parser.peek(),
                    Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SPLIT")
                )
            {
                let _ = parser.next();
                if parser.at_keyword(Keyword::Range) {
                    let _ = parser.next();
                }
                parser.expect_lparen()?;
                let _ = crate::parser::parse::expressions::parse_expr(parser)?;
                parser.expect_rparen()?;
            }
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        } else {
            parser.backtrack(Expected::Description("SCHEME or FUNCTION"))
        }
    } else if parser.at_keyword(Keyword::Database) {
        let saved = parser.save();
        let _ = parser.next();
        // Database name (or CURRENT, or SCOPED CONFIGURATION with no db name)
        if !matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SCOPED"))
            && matches!(
                parser.peek(),
                Some(Token::Identifier(_)) | Some(Token::Keyword(_))
            )
        {
            let _ = parser.next();
        }
        // Support storage clauses: ADD FILEGROUP / ADD FILE / MODIFY FILE / REMOVE FILE
        if parser.at_keyword(Keyword::Add) {
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("FILEGROUP"))
            {
                let _ = parser.next();
                let fg_name = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_sql().to_string(),
                    _ => return parser.backtrack(Expected::Description("filegroup name")),
                };
                skip_ddl_tail(parser);
                return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                    Box::new(crate::ast::CatalogMetadataAction::AddFilegroup { name: fg_name }),
                ))));
            }
            skip_ddl_tail(parser);
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        } else if matches!(
            parser.peek(),
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("MODIFY")
                    || id.eq_ignore_ascii_case("REMOVE")
                    || id.eq_ignore_ascii_case("SCOPED")
                    || id.eq_ignore_ascii_case("COLLATE")
        ) || parser.at_keyword(Keyword::Set)
        {
            // Support ALTER DATABASE <db> SET ..., ALTER DATABASE <db> COLLATE ...,
            // ALTER DATABASE SCOPED CONFIGURATION SET ..., MODIFY/REMOVE FILE ...
            let _ = parser.next();
            if matches!(
                parser.peek(),
                Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CONFIGURATION")
            ) {
                let _ = parser.next();
                if parser.at_keyword(Keyword::Set)
                    || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CLEAR"))
                {
                    let _ = parser.next();
                }
            }
            skip_ddl_tail(parser);
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        } else {
            parser.restore(saved);
            parser.backtrack(Expected::Description(
                "ALTER DATABASE ADD/MODIFY/REMOVE/SET",
            ))
        }
    } else if parser.at_keyword(Keyword::Schema) {
        let _ = parser.next();
        let target_schema = match parser.next() {
            Some(Token::Identifier(id)) => id.clone(),
            Some(Token::Keyword(k)) => k.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("schema name")),
        };
        if matches!(
            parser.peek(),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TRANSFER")
        ) {
            let _ = parser.next();
            let entity_type = if matches!(parser.peek_at(1), Some(Token::Operator(op)) if op == "::")
            {
                let cls = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_ref().to_string(),
                    _ => String::new(),
                };
                let _ = parser.next(); // '::'
                Some(cls)
            } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("XML"))
                && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Schema)))
                && matches!(parser.peek_at(2), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("COLLECTION"))
                && matches!(parser.peek_at(3), Some(Token::Operator(op)) if op == "::")
            {
                for _ in 0..4 {
                    let _ = parser.next();
                }
                Some("XML SCHEMA COLLECTION".to_string())
            } else {
                None
            };
            let source = super::parse_multipart_name(parser)?;
            Ok(Statement::Ddl(Box::new(
                DdlStatement::AlterSchemaTransfer {
                    target_schema,
                    entity_type,
                    source,
                },
            )))
        } else {
            skip_ddl_tail(parser);
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        }
    } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ROLE"))
    {
        let _ = parser.next();
        let role = match parser.next() {
            Some(Token::Identifier(id)) => id.clone(),
            Some(Token::Keyword(k)) => k.as_sql().to_string(),
            _ => return parser.backtrack(Expected::Description("role name")),
        };
        let add = if parser.at_keyword(Keyword::Add) {
            let _ = parser.next();
            Some(true)
        } else if parser.at_keyword(Keyword::Drop) {
            let _ = parser.next();
            Some(false)
        } else {
            None
        };
        if let Some(add_flag) = add {
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MEMBER"))
            {
                let _ = parser.next();
                let member = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_sql().to_string(),
                    _ => return parser.backtrack(Expected::Description("member name")),
                };
                return Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
                    Box::new(crate::ast::CatalogMetadataAction::AlterRoleMember {
                        role,
                        member,
                        add: add_flag,
                    }),
                ))));
            }
        }
        skip_ddl_tail(parser);
        Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
            Vec::new(),
        )))
    } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SECURITY"))
        && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("POLICY"))
    {
        let _ = parser.next(); // SECURITY
        let _ = parser.next(); // POLICY
        super::security_policy::parse_alter_security_policy(parser)
    } else if parser.at_keyword(Keyword::User)
        || parser.at_keyword(Keyword::Index)
        || matches!(
            parser.peek(),
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("ROLE")
                    || id.eq_ignore_ascii_case("LOGIN")
                    || id.eq_ignore_ascii_case("FULLTEXT")
                    || id.eq_ignore_ascii_case("SECURITY")
                    || id.eq_ignore_ascii_case("CERTIFICATE")
                    || id.eq_ignore_ascii_case("SYMMETRIC")
                    || id.eq_ignore_ascii_case("ASYMMETRIC")
                    || id.eq_ignore_ascii_case("MASTER")
        )
    {
        let _ = parser.next();
        // Consume target name and optional verb (`ADD`, `DROP`, `SET`, `WITH`, `REBUILD`, etc.)
        // so subclauses like `ALTER ROLE [r] DROP MEMBER [u]` do not stop at `DROP`.
        if matches!(
            parser.peek(),
            Some(Token::Identifier(_)) | Some(Token::Keyword(_))
        ) {
            let _ = super::parse_multipart_name(parser);
        }
        if parser.at_keyword(Keyword::Drop)
            || parser.at_keyword(Keyword::Add)
            || parser.at_keyword(Keyword::Set)
            || parser.at_keyword(Keyword::With)
        {
            let _ = parser.next();
        }
        skip_ddl_tail(parser);
        Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
            Vec::new(),
        )))
    } else {
        parser.backtrack(Expected::Description("table, procedure, or partition"))
    }
}

fn parse_signed_i64(parser: &mut Parser) -> ParseResult<i64> {
    let mut neg = false;
    if matches!(parser.peek(), Some(Token::Operator(op)) if op == "-") {
        let _ = parser.next();
        neg = true;
    } else if matches!(parser.peek(), Some(Token::Operator(op)) if op == "+") {
        let _ = parser.next();
    }
    match parser.next() {
        Some(Token::Number { value, .. }) => {
            let v = *value as i64;
            Ok(if neg { -v } else { v })
        }
        _ => parser.backtrack(Expected::Description("integer")),
    }
}

fn parse_alter_sequence(parser: &mut Parser) -> ParseResult<Statement> {
    let name = super::parse_multipart_name(parser)?;
    let mut restart: Option<Option<i64>> = None;
    let mut increment_by: Option<i64> = None;
    let mut min_value: Option<Option<i64>> = None;
    let mut max_value: Option<Option<i64>> = None;
    let mut cycle: Option<bool> = None;

    while !parser.is_empty() {
        if matches!(parser.peek(), Some(Token::Semicolon) | Some(Token::Go)) {
            break;
        }
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("RESTART"))
        {
            let _ = parser.next();
            if parser.at_keyword(Keyword::With) {
                let _ = parser.next();
                restart = Some(Some(parse_signed_i64(parser)?));
            } else {
                restart = Some(None);
            }
        } else if parser.at_keyword(Keyword::Increment)
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("INCREMENT"))
        {
            let _ = parser.next();
            parser.expect_keyword(Keyword::By)?;
            increment_by = Some(parse_signed_i64(parser)?);
        } else if parser.at_keyword(Keyword::Minvalue)
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MINVALUE"))
        {
            let _ = parser.next();
            min_value = Some(Some(parse_signed_i64(parser)?));
        } else if parser.at_keyword(Keyword::Maxvalue)
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MAXVALUE"))
        {
            let _ = parser.next();
            max_value = Some(Some(parse_signed_i64(parser)?));
        } else if parser.at_keyword(Keyword::Cycle)
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CYCLE"))
        {
            let _ = parser.next();
            cycle = Some(true);
        } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CACHE"))
        {
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::Number { .. })) {
                let _ = parser.next();
            }
        } else if parser.at_keyword(Keyword::No)
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NO"))
        {
            let _ = parser.next();
            if parser.at_keyword(Keyword::Minvalue)
                || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MINVALUE"))
            {
                let _ = parser.next();
                min_value = Some(None);
            } else if parser.at_keyword(Keyword::Maxvalue)
                || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MAXVALUE"))
            {
                let _ = parser.next();
                max_value = Some(None);
            } else if parser.at_keyword(Keyword::Cycle)
                || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CYCLE"))
            {
                let _ = parser.next();
                cycle = Some(false);
            } else if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CACHE"))
            {
                let _ = parser.next();
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Ok(Statement::Ddl(Box::new(DdlStatement::AlterSequence {
        name,
        restart,
        increment_by,
        min_value,
        max_value,
        cycle,
    })))
}
