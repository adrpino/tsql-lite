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
        if parser.at_keyword(Keyword::With) {
            let saved = parser.save();
            let _ = parser.next();
            if parser.at_keyword(Keyword::Check)
                || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("NOCHECK"))
            {
                let _ = parser.next();
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
            return Ok(Statement::Ddl(DdlStatement::AlterTable {
                table,
                action: AlterTableAction::CheckConstraint { check, name },
            }));
        }
        if parser.at_keyword(Keyword::Add) {
            let _ = parser.next();
            let parse_add_item = |p: &mut Parser| -> ParseResult<AlterTableAction> {
                if p.at_keyword(Keyword::Constraint) {
                    let _ = p.next();
                    let constraint = super::ddl::parse_alter_table_add_constraint(p)?;
                    Ok(AlterTableAction::AddConstraint(constraint))
                } else if p.at_keyword(Keyword::Primary)
                    || p.at_keyword(Keyword::Unique)
                    || p.at_keyword(Keyword::Foreign)
                    || p.at_keyword(Keyword::Check)
                    || p.at_keyword(Keyword::Default)
                {
                    let constraint = super::ddl::parse_table_constraint(p)?;
                    Ok(AlterTableAction::AddConstraint(constraint))
                } else {
                    let col = super::ddl::parse_column_def(p)?;
                    Ok(AlterTableAction::AddColumn(col))
                }
            };
            let first = parse_add_item(parser)?;
            if !matches!(parser.peek(), Some(Token::Comma)) {
                Ok(Statement::Ddl(DdlStatement::AlterTable {
                    table,
                    action: first,
                }))
            } else {
                let mut stmts = vec![Statement::Ddl(DdlStatement::AlterTable {
                    table: table.clone(),
                    action: first,
                })];
                while matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                    let next_action = parse_add_item(parser)?;
                    stmts.push(Statement::Ddl(DdlStatement::AlterTable {
                        table: table.clone(),
                        action: next_action,
                    }));
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
            Ok(Statement::Ddl(DdlStatement::AlterTable {
                table,
                action: AlterTableAction::AlterColumn {
                    name: col_name,
                    data_type,
                    nullable,
                },
            }))
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
                    Ok(Statement::Ddl(DdlStatement::AlterTable {
                        table,
                        action: AlterTableAction::DropColumn(cols.into_iter().next().unwrap()),
                    }))
                } else {
                    let stmts = cols
                        .into_iter()
                        .map(|c| {
                            Statement::Ddl(DdlStatement::AlterTable {
                                table: table.clone(),
                                action: AlterTableAction::DropColumn(c),
                            })
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
                    Ok(Statement::Ddl(DdlStatement::AlterTable {
                        table,
                        action: AlterTableAction::DropConstraint(names.into_iter().next().unwrap()),
                    }))
                } else {
                    let stmts = names
                        .into_iter()
                        .map(|c| {
                            Statement::Ddl(DdlStatement::AlterTable {
                                table: table.clone(),
                                action: AlterTableAction::DropConstraint(c),
                            })
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
            parser.expect_lparen()?;
            let mut depth = 1usize;
            while depth > 0 {
                match parser.next() {
                    Some(Token::LParen) => depth += 1,
                    Some(Token::RParen) => depth -= 1,
                    Some(_) => {}
                    None => return parser.backtrack(Expected::Description(")")),
                }
            }
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
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
        Ok(Statement::Ddl(DdlStatement::Create(Box::new(stmt))))
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
        // Database name (or CURRENT)
        if matches!(
            parser.peek(),
            Some(Token::Identifier(_)) | Some(Token::Keyword(_))
        ) {
            let _ = parser.next();
        }
        // Support storage clauses: ADD FILEGROUP / ADD FILE / MODIFY FILE / REMOVE FILE
        if parser.at_keyword(Keyword::Add)
            || matches!(
                parser.peek(),
                Some(Token::Identifier(id))
                    if id.eq_ignore_ascii_case("MODIFY") || id.eq_ignore_ascii_case("REMOVE")
            )
        {
            let _ = parser.next();
            skip_ddl_tail(parser);
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        } else {
            parser.restore(saved);
            parser.backtrack(Expected::Description("ALTER DATABASE ADD/MODIFY/REMOVE"))
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
            Ok(Statement::Ddl(DdlStatement::AlterSchemaTransfer {
                target_schema,
                entity_type,
                source,
            }))
        } else {
            skip_ddl_tail(parser);
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(
                Vec::new(),
            )))
        }
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
