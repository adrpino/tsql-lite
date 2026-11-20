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
        let name = super::parse_multipart_name(parser)?;
        Ok(Statement::Ddl(DdlStatement::DropTable { name, if_exists }))
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
        let name = super::parse_multipart_name(parser)?;
        parser.expect_keyword(Keyword::On)?;
        let table = super::parse_multipart_name(parser)?;
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
