use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

pub fn parse_create_table(parser: &mut Parser) -> ParseResult<CreateStmt> {
    let name = super::parse_multipart_name(parser)?;
    parser.expect_lparen()?;
    let (columns, constraints) = super::ddl::parse_table_body(parser)?;
    parser.expect_rparen()?;
    // Optional storage / table options clauses in any order:
    // ON [PRIMARY] / ON [scheme](col) / TEXTIMAGE_ON [PRIMARY] / FILESTREAM_ON [fg] / WITH (...)
    loop {
        if parser.at_keyword(Keyword::On)
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TEXTIMAGE_ON") || id.eq_ignore_ascii_case("FILESTREAM_ON"))
        {
            let _ = parser.next();
            let _ = super::parse_multipart_name(parser)?;
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                let mut depth = 1usize;
                while depth > 0 {
                    match parser.next() {
                        Some(Token::LParen) => depth += 1,
                        Some(Token::RParen) => depth -= 1,
                        None => break,
                        _ => {}
                    }
                }
            }
            continue;
        }
        if parser.at_keyword(Keyword::With) {
            let saved = parser.save();
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                let mut depth = 1usize;
                while depth > 0 {
                    match parser.next() {
                        Some(Token::LParen) => depth += 1,
                        Some(Token::RParen) => depth -= 1,
                        None => break,
                        _ => {}
                    }
                }
                continue;
            }
            parser.restore(saved);
        }
        break;
    }
    Ok(CreateStmt::Table {
        name,
        columns,
        constraints,
    })
}

pub fn parse_create_view(parser: &mut Parser) -> ParseResult<CreateStmt> {
    let name = super::parse_multipart_name(parser)?;
    let mut columns = Vec::new();
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        columns = crate::parser::parse::parse_comma_list(parser, |p| match p.next() {
            Some(Token::Identifier(id)) => Ok(id.clone()),
            Some(Token::Keyword(k)) => Ok(k.as_ref().to_string()),
            _ => p.backtrack(Expected::Description("column name")),
        })?;
        parser.expect_rparen()?;
    }
    if parser.at_keyword(Keyword::With) {
        let _ = parser.next();
        let _ = crate::parser::parse::parse_comma_list(parser, |p| match p.next() {
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("SCHEMABINDING")
                    || id.eq_ignore_ascii_case("ENCRYPTION")
                    || id.eq_ignore_ascii_case("VIEW_METADATA") =>
            {
                Ok(())
            }
            Some(Token::Keyword(k))
                if k.as_ref().eq_ignore_ascii_case("SCHEMABINDING")
                    || k.as_ref().eq_ignore_ascii_case("ENCRYPTION")
                    || k.as_ref().eq_ignore_ascii_case("VIEW_METADATA") =>
            {
                Ok(())
            }
            _ => p.backtrack(Expected::Description("view attribute")),
        })?;
    }
    parser.expect_keyword(Keyword::As)?;
    let mut paren_depth = 0usize;
    while matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        paren_depth += 1;
    }
    let query = crate::parser::parse::statements::query::parse_select(parser)?;
    for _ in 0..paren_depth {
        parser.expect_rparen()?;
    }
    Ok(CreateStmt::View {
        name,
        columns,
        query,
    })
}

fn parse_procedure_with_options(parser: &mut Parser) -> ParseResult<()> {
    if !parser.at_keyword(Keyword::With) {
        return Ok(());
    }
    let _ = parser.next();
    let _ = crate::parser::parse::parse_comma_list(parser, |p| match p.peek() {
        Some(Token::Keyword(Keyword::Execute)) | Some(Token::Keyword(Keyword::Exec)) => {
            let _ = p.next();
            p.expect_keyword(Keyword::As)?;
            match p.next() {
                Some(Token::String(_))
                | Some(Token::NString(_))
                | Some(Token::Identifier(_))
                | Some(Token::Keyword(_)) => Ok(()),
                _ => p.backtrack(Expected::Description("EXECUTE AS principal")),
            }
        }
        Some(Token::Identifier(id))
            if id.eq_ignore_ascii_case("RECOMPILE")
                || id.eq_ignore_ascii_case("ENCRYPTION")
                || id.eq_ignore_ascii_case("SCHEMABINDING")
                || id.eq_ignore_ascii_case("NATIVE_COMPILATION") =>
        {
            let _ = p.next();
            Ok(())
        }
        Some(Token::Keyword(k))
            if k.as_ref().eq_ignore_ascii_case("RECOMPILE")
                || k.as_ref().eq_ignore_ascii_case("ENCRYPTION")
                || k.as_ref().eq_ignore_ascii_case("SCHEMABINDING")
                || k.as_ref().eq_ignore_ascii_case("NATIVE_COMPILATION") =>
        {
            let _ = p.next();
            Ok(())
        }
        _ => p.backtrack(Expected::Description("procedure option")),
    })?;
    Ok(())
}

pub fn parse_create_procedure(parser: &mut Parser) -> ParseResult<CreateStmt> {
    let name = super::parse_multipart_name(parser)?;
    let mut params = Vec::new();
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        params = crate::parser::parse::parse_comma_list(
            parser,
            crate::parser::parse::parse_routine_param,
        )?;
        parser.expect_rparen()?;
    } else if matches!(parser.peek(), Some(Token::Variable(_))) {
        params = crate::parser::parse::parse_comma_list(
            parser,
            crate::parser::parse::parse_routine_param,
        )?;
    }
    parse_procedure_with_options(parser)?;
    parser.expect_keyword(Keyword::As)?;
    let is_begin_block = parser.at_keyword(Keyword::Begin)
        && !matches!(
            parser.peek_at(1),
            Some(Token::Keyword(
                Keyword::Transaction | Keyword::Tran | Keyword::Try | Keyword::Distributed
            ))
        );
    let body = if is_begin_block {
        let _ = parser.next();
        match super::other::parse_begin_end(parser)? {
            Statement::Procedural(ProceduralStatement::BeginEnd(stmts)) => stmts,
            _ => unreachable!(),
        }
    } else {
        let mut stmts = Vec::new();
        while !parser.is_empty() && !matches!(parser.peek(), Some(Token::Go)) {
            while matches!(parser.peek(), Some(Token::Semicolon)) {
                let _ = parser.next();
            }
            if parser.is_empty() || matches!(parser.peek(), Some(Token::Go)) {
                break;
            }
            stmts.push(crate::parser::parse::parse_statement(parser)?);
            while matches!(parser.peek(), Some(Token::Semicolon)) {
                let _ = parser.next();
            }
        }
        stmts
    };
    Ok(CreateStmt::Procedure {
        name,
        params,
        body,
        or_alter: false,
    })
}

fn parse_function_with_options(parser: &mut Parser) -> ParseResult<()> {
    if !parser.at_keyword(Keyword::With) {
        return Ok(());
    }
    let _ = parser.next();
    let _ = crate::parser::parse::parse_comma_list(parser, |p| match p.peek() {
        Some(Token::Keyword(Keyword::Execute)) | Some(Token::Keyword(Keyword::Exec)) => {
            let _ = p.next();
            p.expect_keyword(Keyword::As)?;
            match p.next() {
                Some(Token::String(_))
                | Some(Token::NString(_))
                | Some(Token::Identifier(_))
                | Some(Token::Keyword(_)) => Ok(()),
                _ => p.backtrack(Expected::Description("EXECUTE AS principal")),
            }
        }
        Some(Token::Keyword(Keyword::Returns)) => {
            // RETURNS NULL ON NULL INPUT
            let _ = p.next();
            for _ in 0..4 {
                let _ = p.next();
            }
            Ok(())
        }
        Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("CALLED") => {
            // CALLED ON NULL INPUT
            let _ = p.next();
            for _ in 0..3 {
                let _ = p.next();
            }
            Ok(())
        }
        Some(Token::Identifier(id))
            if id.eq_ignore_ascii_case("SCHEMABINDING")
                || id.eq_ignore_ascii_case("ENCRYPTION")
                || id.eq_ignore_ascii_case("INLINE") =>
        {
            let _ = p.next();
            if matches!(p.peek(), Some(Token::Operator(op)) if op == "=") {
                let _ = p.next();
                let _ = p.next();
            }
            Ok(())
        }
        Some(Token::Keyword(k))
            if k.as_ref().eq_ignore_ascii_case("SCHEMABINDING")
                || k.as_ref().eq_ignore_ascii_case("ENCRYPTION") =>
        {
            let _ = p.next();
            Ok(())
        }
        _ => p.backtrack(Expected::Description("function option")),
    })?;
    Ok(())
}

pub fn parse_create_function(parser: &mut Parser) -> ParseResult<CreateStmt> {
    let name = super::parse_multipart_name(parser)?;
    let mut params = Vec::new();
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        if !matches!(parser.peek(), Some(Token::RParen)) {
            params = crate::parser::parse::parse_comma_list(
                parser,
                crate::parser::parse::parse_routine_param,
            )?;
        }
        parser.expect_rparen()?;
    }

    parser.expect_keyword(Keyword::Returns)?;
    let mut returns = None;
    let mut is_table_return = false;
    let mut multi_stmt_table: Option<(String, Vec<ColumnDef>, Vec<TableConstraint>)> = None;
    if parser.at_keyword(Keyword::Table) {
        let _ = parser.next();
        is_table_return = true;
    } else if let Some(Token::Variable(var_name)) = parser.peek().cloned() {
        if matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Table))) {
            let _ = parser.next();
            let _ = parser.next();
            parser.expect_lparen()?;
            let (cols, cons) = super::ddl::parse_table_body(parser)?;
            parser.expect_rparen()?;
            multi_stmt_table = Some((var_name, cols, cons));
        } else {
            returns = Some(crate::parser::parse::expressions::parse_data_type(parser)?);
        }
    } else {
        returns = Some(crate::parser::parse::expressions::parse_data_type(parser)?);
    }

    parse_function_with_options(parser)?;
    parser.expect_keyword(Keyword::As)?;

    let body = if let Some((table_var, columns, constraints)) = multi_stmt_table {
        parser.expect_keyword(Keyword::Begin)?;
        let stmts = match super::other::parse_begin_end(parser)? {
            Statement::Procedural(ProceduralStatement::BeginEnd(stmts)) => stmts,
            _ => unreachable!(),
        };
        FunctionBody::MultiStatementTable {
            table_var,
            columns,
            constraints,
            body: stmts,
        }
    } else if parser.at_keyword(Keyword::Begin) {
        let _ = parser.next();
        match super::other::parse_begin_end(parser)? {
            Statement::Procedural(ProceduralStatement::BeginEnd(stmts)) => {
                FunctionBody::Block(stmts)
            }
            _ => unreachable!(),
        }
    } else if is_table_return {
        parser.expect_keyword(Keyword::Return)?;
        let mut paren_depth = 0usize;
        while matches!(parser.peek(), Some(Token::LParen)) {
            let _ = parser.next();
            paren_depth += 1;
        }
        let body = if parser.at_keyword(Keyword::With) {
            let _ = parser.next();
            let ctes = crate::parser::parse::parse_comma_list(
                parser,
                crate::parser::parse::parse_cte_def,
            )?;
            let query = crate::parser::parse::statements::query::parse_select(parser)?;
            FunctionBody::TableWithCte { ctes, query }
        } else {
            let query = crate::parser::parse::statements::query::parse_select(parser)?;
            FunctionBody::Table(query)
        };
        for _ in 0..paren_depth {
            parser.expect_rparen()?;
        }
        body
    } else if parser.at_keyword(Keyword::Return) {
        let _ = parser.next();
        let expr = crate::parser::parse::expressions::parse_expr(parser)?;
        FunctionBody::ScalarReturn(expr)
    } else {
        parser.expect_keyword(Keyword::Table)?;
        parser.expect_keyword(Keyword::Return)?;
        parser.expect_lparen()?;
        let query = crate::parser::parse::statements::query::parse_select(parser)?;
        parser.expect_rparen()?;
        FunctionBody::Table(query)
    };
    Ok(CreateStmt::Function {
        name,
        params,
        returns,
        body,
    })
}

pub fn parse_create_trigger(parser: &mut Parser) -> ParseResult<CreateStmt> {
    let name = super::parse_multipart_name(parser)?;
    parser.expect_keyword(Keyword::On)?;
    let table = super::parse_multipart_name(parser)?;

    let mut is_instead_of = false;
    if parser.at_keyword(Keyword::Instead) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Of)?;
        is_instead_of = true;
    } else if matches!(parser.peek(), Some(Token::Keyword(k)) if matches!(k, Keyword::After | Keyword::For))
    {
        let _ = parser.next();
    }

    let events = crate::parser::parse::expressions::parse_comma_list(parser, |p| match p.next() {
        Some(Token::Keyword(k)) => match *k {
            Keyword::Insert => Ok(crate::parser::ast::TriggerEvent::Insert),
            Keyword::Update => Ok(crate::parser::ast::TriggerEvent::Update),
            Keyword::Delete => Ok(crate::parser::ast::TriggerEvent::Delete),
            _ => p.backtrack(Expected::Description("INSERT, UPDATE, or DELETE")),
        },
        _ => p.backtrack(Expected::Description("INSERT, UPDATE, or DELETE")),
    })?;

    parser.expect_keyword(Keyword::As)?;
    let body = if parser.at_keyword(Keyword::Begin) {
        let _ = parser.next();
        match super::other::parse_begin_end(parser)? {
            Statement::Procedural(ProceduralStatement::BeginEnd(stmts)) => stmts,
            _ => unreachable!(),
        }
    } else {
        vec![crate::parser::parse::parse_statement(parser)?]
    };
    Ok(CreateStmt::Trigger {
        name,
        table,
        events,
        is_instead_of,
        body,
    })
}
