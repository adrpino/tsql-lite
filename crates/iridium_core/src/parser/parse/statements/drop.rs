use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

fn consume_if_exists(parser: &mut Parser) -> bool {
    if parser.at_keyword(Keyword::If)
        && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Exists)))
    {
        let _ = parser.next();
        let _ = parser.next();
        true
    } else {
        false
    }
}

pub fn parse_drop(parser: &mut Parser) -> ParseResult<Statement> {
    if parser.at_keyword(Keyword::Table) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let first = super::parse_multipart_name(parser)?;
        if matches!(parser.peek(), Some(Token::Comma)) {
            let mut stmts = vec![Statement::Ddl(DdlStatement::DropTable {
                name: first,
                if_exists,
            })];
            while matches!(parser.peek(), Some(Token::Comma)) {
                let _ = parser.next();
                let next_name = super::parse_multipart_name(parser)?;
                stmts.push(Statement::Ddl(DdlStatement::DropTable {
                    name: next_name,
                    if_exists,
                }));
            }
            Ok(Statement::Procedural(ProceduralStatement::BeginEnd(stmts)))
        } else {
            Ok(Statement::Ddl(DdlStatement::DropTable {
                name: first,
                if_exists,
            }))
        }
    } else if parser.at_keyword(Keyword::View) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let name = super::parse_multipart_name(parser)?;
        Ok(Statement::Ddl(DdlStatement::DropView { name, if_exists }))
    } else if matches!(parser.peek(), Some(Token::Keyword(kw)) if matches!(kw, Keyword::Procedure | Keyword::Proc))
    {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let name = super::parse_multipart_name(parser)?;
        Ok(Statement::Ddl(DdlStatement::DropProcedure {
            name,
            if_exists,
        }))
    } else if parser.at_keyword(Keyword::Index) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let mut first = super::parse_multipart_name(parser)?;
        let (name, table) = if parser.at_keyword(Keyword::On) {
            let _ = parser.next();
            let table = super::parse_multipart_name(parser)?;
            (first, table)
        } else if first.len() >= 2 {
            // Legacy T-SQL syntax: `DROP INDEX [schema.]table.index_name`
            let idx_name = first.pop().unwrap();
            (vec![idx_name], first)
        } else {
            return parser.backtrack(Expected::Keyword(Keyword::On));
        };
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
        Ok(Statement::Ddl(DdlStatement::DropIndex {
            name,
            table,
            if_exists,
        }))
    } else if parser.at_keyword(Keyword::Type) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let name = super::parse_multipart_name(parser)?;
        Ok(Statement::Ddl(DdlStatement::DropType { name, if_exists }))
    } else if parser.at_keyword(Keyword::Schema) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let name = match parser.next() {
            Some(Token::Identifier(id)) => id.clone(),
            Some(Token::Keyword(k)) => k.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("identifier or keyword")),
        };
        Ok(Statement::Ddl(DdlStatement::DropSchema { name, if_exists }))
    } else if parser.at_keyword(Keyword::Function) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let name = super::parse_multipart_name(parser)?;
        Ok(Statement::Ddl(DdlStatement::DropFunction {
            name,
            if_exists,
        }))
    } else if parser.at_keyword(Keyword::Trigger) {
        let _ = parser.next();
        let if_exists = consume_if_exists(parser);
        let name = super::parse_multipart_name(parser)?;
        Ok(Statement::Ddl(DdlStatement::DropTrigger {
            name,
            if_exists,
        }))
    } else {
        parser.backtrack(Expected::Description("drop target"))
    }
}
