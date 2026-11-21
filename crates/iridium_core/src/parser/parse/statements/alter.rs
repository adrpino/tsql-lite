use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

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
            if parser.at_keyword(Keyword::Constraint) {
                let _ = parser.next();
                let constraint = super::ddl::parse_alter_table_add_constraint(parser)?;
                Ok(Statement::Ddl(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::AddConstraint(constraint),
                }))
            } else {
                let col = super::ddl::parse_column_def(parser)?;
                Ok(Statement::Ddl(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::AddColumn(col),
                }))
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
            if parser.at_keyword(Keyword::Column) {
                let _ = parser.next();
                let col_name = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("column name")),
                };
                Ok(Statement::Ddl(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::DropColumn(col_name),
                }))
            } else if parser.at_keyword(Keyword::Constraint) {
                let _ = parser.next();
                let constraint_name = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("constraint name")),
                };
                Ok(Statement::Ddl(DdlStatement::AlterTable {
                    table,
                    action: AlterTableAction::DropConstraint(constraint_name),
                }))
            } else {
                parser.backtrack(Expected::Description("column or constraint"))
            }
        } else {
            parser.backtrack(Expected::Description("add or drop"))
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
    } else {
        parser.backtrack(Expected::Description("table or procedure"))
    }
}
