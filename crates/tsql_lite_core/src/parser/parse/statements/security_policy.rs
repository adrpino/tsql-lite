use crate::ast::{
    AlterSecurityPolicyAction, CatalogMetadataAction, ObjectName, SecurityPredicateSpec,
};
use crate::catalog::SecurityBlockOp;
use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

fn to_object_name(parts: Vec<String>) -> ObjectName {
    match parts.len() {
        0 => ObjectName {
            database: None,
            schema: None,
            name: String::new(),
        },
        1 => ObjectName {
            database: None,
            schema: None,
            name: parts.into_iter().next().unwrap(),
        },
        2 => {
            let mut it = parts.into_iter();
            ObjectName {
                database: None,
                schema: Some(it.next().unwrap()),
                name: it.next().unwrap(),
            }
        }
        _ => {
            let mut it = parts.into_iter();
            let db = it.next();
            let sch = it.next();
            let name = it.next().unwrap_or_default();
            ObjectName {
                database: db,
                schema: sch,
                name,
            }
        }
    }
}

fn at_word(parser: &Parser, word: &str) -> bool {
    match parser.peek() {
        Some(Token::Identifier(id)) => id.eq_ignore_ascii_case(word),
        Some(Token::Keyword(k)) => k.as_sql().eq_ignore_ascii_case(word),
        _ => false,
    }
}

fn consume_word(parser: &mut Parser, word: &str) -> bool {
    if at_word(parser, word) {
        let _ = parser.next();
        true
    } else {
        false
    }
}

fn parse_optional_block_op(parser: &mut Parser) -> Option<SecurityBlockOp> {
    if consume_word(parser, "AFTER") {
        if parser.at_keyword(Keyword::Insert) {
            let _ = parser.next();
            return Some(SecurityBlockOp::AfterInsert);
        }
        if parser.at_keyword(Keyword::Update) {
            let _ = parser.next();
            return Some(SecurityBlockOp::AfterUpdate);
        }
    } else if consume_word(parser, "BEFORE") {
        if parser.at_keyword(Keyword::Update) {
            let _ = parser.next();
            return Some(SecurityBlockOp::BeforeUpdate);
        }
        if parser.at_keyword(Keyword::Delete) {
            let _ = parser.next();
            return Some(SecurityBlockOp::BeforeDelete);
        }
    }
    None
}

fn parse_predicate_kind(parser: &mut Parser) -> ParseResult<bool> {
    let is_block = if consume_word(parser, "FILTER") {
        false
    } else if consume_word(parser, "BLOCK") {
        true
    } else {
        return parser.backtrack(Expected::Description("FILTER or BLOCK"));
    };
    if !consume_word(parser, "PREDICATE") {
        return parser.backtrack(Expected::Description("PREDICATE"));
    }
    Ok(is_block)
}

fn parse_predicate_spec(parser: &mut Parser) -> ParseResult<SecurityPredicateSpec> {
    let is_block = parse_predicate_kind(parser)?;
    let fn_parts = super::parse_multipart_name(parser)?;
    parser.expect_lparen()?;
    let function_args =
        crate::parser::parse::expressions::parse_comma_list(parser, |p| match p.next() {
            Some(Token::Identifier(id)) => Ok(id.clone()),
            Some(Token::Keyword(k)) => Ok(k.as_sql().to_string()),
            _ => p.backtrack(Expected::Description("column name")),
        })?;
    parser.expect_rparen()?;
    parser.expect_keyword(Keyword::On)?;
    let target_parts = super::parse_multipart_name(parser)?;
    let block_op = if is_block {
        Some(parse_optional_block_op(parser).unwrap_or(SecurityBlockOp::All))
    } else {
        None
    };
    Ok(SecurityPredicateSpec {
        is_block,
        block_op,
        function: to_object_name(fn_parts),
        function_args,
        target: to_object_name(target_parts),
    })
}

fn parse_policy_with_options(parser: &mut Parser) -> ParseResult<(Option<bool>, Option<bool>)> {
    let mut state_on = None;
    let mut schema_binding = None;
    if parser.at_keyword(Keyword::With) {
        let _ = parser.next();
        parser.expect_lparen()?;
        loop {
            if consume_word(parser, "STATE") {
                if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                    let _ = parser.next();
                }
                if parser.at_keyword(Keyword::On) || consume_word(parser, "ON") {
                    if parser.at_keyword(Keyword::On) {
                        let _ = parser.next();
                    }
                    state_on = Some(true);
                } else if consume_word(parser, "OFF") {
                    state_on = Some(false);
                } else {
                    return parser.backtrack(Expected::Description("ON or OFF"));
                }
            } else if consume_word(parser, "SCHEMABINDING") {
                if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                    let _ = parser.next();
                }
                if parser.at_keyword(Keyword::On) || consume_word(parser, "ON") {
                    if parser.at_keyword(Keyword::On) {
                        let _ = parser.next();
                    }
                    schema_binding = Some(true);
                } else if consume_word(parser, "OFF") {
                    schema_binding = Some(false);
                } else {
                    return parser.backtrack(Expected::Description("ON or OFF"));
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
        parser.expect_rparen()?;
    }
    // Optional trailing NOT FOR REPLICATION
    if parser.at_keyword(Keyword::Not) {
        let saved = parser.save();
        let _ = parser.next();
        if parser.at_keyword(Keyword::For) {
            let _ = parser.next();
            if consume_word(parser, "REPLICATION") {
                // consumed
            } else {
                parser.restore(saved);
            }
        } else {
            parser.restore(saved);
        }
    }
    Ok((state_on, schema_binding))
}

pub fn parse_create_security_policy(parser: &mut Parser) -> ParseResult<Statement> {
    let name_parts = super::parse_multipart_name(parser)?;
    let mut predicates = Vec::new();
    while parser.at_keyword(Keyword::Add) {
        let _ = parser.next();
        predicates.push(parse_predicate_spec(parser)?);
        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
        } else {
            break;
        }
    }
    let (state_opt, sb_opt) = parse_policy_with_options(parser)?;
    Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
        Box::new(CatalogMetadataAction::CreateSecurityPolicy {
            name: to_object_name(name_parts),
            predicates,
            state_on: state_opt.unwrap_or(true),
            schema_binding: sb_opt.unwrap_or(true),
        }),
    ))))
}

pub fn parse_alter_security_policy(parser: &mut Parser) -> ParseResult<Statement> {
    let name_parts = super::parse_multipart_name(parser)?;
    let mut actions = Vec::new();
    loop {
        if parser.at_keyword(Keyword::Add) {
            let _ = parser.next();
            actions.push(AlterSecurityPolicyAction::AddPredicate(
                parse_predicate_spec(parser)?,
            ));
        } else if parser.at_keyword(Keyword::Alter) {
            let _ = parser.next();
            actions.push(AlterSecurityPolicyAction::AlterPredicate(
                parse_predicate_spec(parser)?,
            ));
        } else if parser.at_keyword(Keyword::Drop) {
            let _ = parser.next();
            let is_block = parse_predicate_kind(parser)?;
            parser.expect_keyword(Keyword::On)?;
            let target_parts = super::parse_multipart_name(parser)?;
            let block_op = if is_block {
                Some(parse_optional_block_op(parser).unwrap_or(SecurityBlockOp::All))
            } else {
                None
            };
            actions.push(AlterSecurityPolicyAction::DropPredicate {
                is_block,
                target: to_object_name(target_parts),
                block_op,
            });
        } else if parser.at_keyword(Keyword::With) {
            let (state_opt, _) = parse_policy_with_options(parser)?;
            if let Some(state_on) = state_opt {
                actions.push(AlterSecurityPolicyAction::SetState(state_on));
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
    Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
        Box::new(CatalogMetadataAction::AlterSecurityPolicy {
            name: to_object_name(name_parts),
            actions,
        }),
    ))))
}

pub fn parse_drop_security_policy(parser: &mut Parser) -> ParseResult<Statement> {
    let mut if_exists = false;
    if parser.at_keyword(Keyword::If) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Exists)?;
        if_exists = true;
    }
    let name_parts = super::parse_multipart_name(parser)?;
    Ok(Statement::Ddl(Box::new(DdlStatement::CatalogMetadata(
        Box::new(CatalogMetadataAction::DropSecurityPolicy {
            name: to_object_name(name_parts),
            if_exists,
        }),
    ))))
}
