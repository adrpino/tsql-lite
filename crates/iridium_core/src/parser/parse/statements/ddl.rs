use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

pub use super::create::{
    parse_create_function, parse_create_procedure, parse_create_table, parse_create_trigger,
    parse_create_view,
};

pub fn parse_create(parser: &mut Parser) -> ParseResult<CreateStmt> {
    if parser.at_keyword(Keyword::Table) {
        let _ = parser.next();
        parse_create_table(parser)
    } else if parser.at_keyword(Keyword::View) {
        let _ = parser.next();
        parse_create_view(parser)
    } else if matches!(parser.peek(), Some(Token::Keyword(kw)) if matches!(kw, Keyword::Procedure | Keyword::Proc))
    {
        let _ = parser.next();
        parse_create_procedure(parser)
    } else if parser.at_keyword(Keyword::Function) {
        let _ = parser.next();
        parse_create_function(parser)
    } else if parser.at_keyword(Keyword::Trigger) {
        let _ = parser.next();
        parse_create_trigger(parser)
    } else {
        parser.backtrack(Expected::Description(
            "TABLE, VIEW, PROCEDURE, FUNCTION, or TRIGGER",
        ))
    }
}

pub fn parse_column_def(parser: &mut Parser) -> ParseResult<ColumnDef> {
    let name = if let Some(tok) = parser.next() {
        match tok {
            Token::Identifier(id) => id.clone(),
            Token::Keyword(k) => k.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("column name")),
        }
    } else {
        return parser.backtrack(Expected::Description("column name"));
    };
    let data_type = if matches!(parser.peek(), Some(Token::Keyword(Keyword::As))) {
        DataType::SqlVariant
    } else {
        crate::parser::parse::expressions::parse_data_type(parser)?
    };

    let mut is_nullable = None;
    let mut is_identity = false;
    let mut identity_spec = None;
    let mut is_primary_key = false;
    let mut is_unique = false;
    let mut default_expr = None;
    let mut default_constraint_name = None;
    let mut check_expr = None;
    let mut check_constraint_name = None;
    let mut computed_expr = None;
    let mut foreign_key = None;
    let mut collation = None;
    let mut is_clustered = false;
    let mut with_values = false;
    let mut masking_function = None;

    loop {
        match parser.peek() {
            Some(Token::Keyword(Keyword::Null)) => {
                let _ = parser.next();
                is_nullable = Some(true);
            }
            Some(Token::Keyword(Keyword::Not)) => {
                let saved = parser.save();
                let _ = parser.next();
                if parser.at_keyword(Keyword::Null) {
                    let _ = parser.next();
                    is_nullable = Some(false);
                } else if parser.at_keyword(Keyword::For)
                    && matches!(parser.peek_at(1), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("REPLICATION"))
                {
                    let _ = parser.next();
                    let _ = parser.next();
                } else {
                    parser.restore(saved);
                    break;
                }
            }
            Some(Token::Keyword(Keyword::Identity)) => {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    let seed = parse_signed_int(parser).unwrap_or(1);
                    parser.expect_comma()?;
                    let inc = parse_signed_int(parser).unwrap_or(1);
                    parser.expect_rparen()?;
                    identity_spec = Some((seed, inc));
                }
                is_identity = true;
            }
            Some(Token::Keyword(Keyword::Primary)) => {
                // If PRIMARY KEY [CLUSTERED|NONCLUSTERED] is followed by '(', it is a table-level
                // constraint placed after a column definition without a separating comma.
                let saved = parser.save();
                let _ = parser.next();
                parser.expect_keyword(Keyword::Key)?;
                let mut col_clustered = true;
                if parser.at_keyword(Keyword::Clustered) {
                    let _ = parser.next();
                    col_clustered = true;
                } else if parser.at_keyword(Keyword::Nonclustered) {
                    let _ = parser.next();
                    col_clustered = false;
                }
                if matches!(parser.peek(), Some(Token::LParen)) {
                    parser.restore(saved);
                    break;
                }
                is_clustered = col_clustered;
                is_primary_key = true;
                skip_optional_index_storage_clauses(parser)?;
            }
            Some(Token::Keyword(Keyword::Unique)) => {
                let _ = parser.next();
                if parser.at_keyword(Keyword::Clustered) {
                    let _ = parser.next();
                    is_clustered = true;
                } else if parser.at_keyword(Keyword::Nonclustered) {
                    let _ = parser.next();
                    is_clustered = false;
                }
                is_unique = true;
                skip_optional_index_storage_clauses(parser)?;
            }
            Some(Token::Keyword(Keyword::Default)) => {
                let _ = parser.next();
                default_expr = Some(crate::parser::parse::expressions::parse_expr(parser)?);
            }
            Some(Token::Keyword(Keyword::Check)) => {
                let _ = parser.next();
                skip_optional_not_for_replication(parser);
                parser.expect_lparen()?;
                check_expr = Some(crate::parser::parse::expressions::parse_expr(parser)?);
                parser.expect_rparen()?;
            }
            Some(Token::Keyword(Keyword::Constraint)) => {
                let _ = parser.next();
                let constraint_name = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("constraint name")),
                };
                match parser.next() {
                    Some(Token::Keyword(Keyword::Default)) => {
                        default_expr = Some(crate::parser::parse::expressions::parse_expr(parser)?);
                        default_constraint_name = Some(constraint_name);
                    }
                    Some(Token::Keyword(Keyword::Check)) => {
                        skip_optional_not_for_replication(parser);
                        parser.expect_lparen()?;
                        check_expr = Some(crate::parser::parse::expressions::parse_expr(parser)?);
                        parser.expect_rparen()?;
                        check_constraint_name = Some(constraint_name);
                    }
                    _ => return parser.backtrack(Expected::Description("DEFAULT or CHECK")),
                }
            }
            Some(Token::Keyword(Keyword::References)) => {
                let _ = parser.next();
                let ref_table = super::parse_multipart_name(parser)?;
                let mut ref_columns = Vec::new();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    ref_columns =
                        crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                            match p.next() {
                                Some(Token::Identifier(id)) => Ok(id.clone()),
                                Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                                _ => p.backtrack(Expected::Description("column name")),
                            }
                        })?;
                    parser.expect_rparen()?;
                }
                foreign_key = Some(ForeignKeyRef {
                    ref_table,
                    ref_columns,
                    on_delete: None,
                    on_update: None,
                });
            }
            Some(Token::Keyword(Keyword::As)) => {
                let _ = parser.next();
                computed_expr = Some(crate::parser::parse::expressions::parse_expr(parser)?);
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("PERSISTED"))
                {
                    let _ = parser.next();
                }
            }
            Some(Token::Keyword(Keyword::Index)) => {
                // Inline column-level index: [col] type INDEX ix_name [CLUSTERED | NONCLUSTERED]
                let _ = parser.next();
                match parser.next() {
                    Some(Token::Identifier(_)) | Some(Token::Keyword(_)) => {}
                    _ => return parser.backtrack(Expected::Description("index name")),
                }
                if parser.at_keyword(Keyword::Clustered) || parser.at_keyword(Keyword::Nonclustered)
                {
                    let _ = parser.next();
                }
            }
            Some(Token::Keyword(Keyword::Collate)) => {
                let _ = parser.next();
                let collation_name = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("collation name")),
                };
                collation = Some(collation_name);
            }
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("ROWGUIDCOL")
                    || id.eq_ignore_ascii_case("SPARSE")
                    || id.eq_ignore_ascii_case("FILESTREAM")
                    || id.eq_ignore_ascii_case("HIDDEN") =>
            {
                let _ = parser.next();
            }
            Some(Token::Keyword(Keyword::With)) => {
                let saved = parser.save();
                let _ = parser.next();
                if parser.at_keyword(Keyword::Values)
                    || matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("VALUES"))
                {
                    let _ = parser.next();
                    with_values = true;
                } else {
                    parser.restore(saved);
                    break;
                }
            }
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("MASKED") => {
                let _ = parser.next();
                if parser.at_keyword(Keyword::With) {
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next(); // (
                        if parser.at_keyword(Keyword::Function)
                            || matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("FUNCTION"))
                        {
                            let _ = parser.next();
                            if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
                                let _ = parser.next();
                            }
                            match parser.next() {
                                Some(Token::String(func)) | Some(Token::NString(func)) => {
                                    masking_function = Some(func.clone());
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
            }
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("ENCRYPTED") => {
                let _ = parser.next();
                if parser.at_keyword(Keyword::With) {
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        skip_balanced_parens(parser)?;
                    }
                }
            }
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("GENERATED") => {
                // GENERATED ALWAYS AS ROW START|END [HIDDEN]
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("ALWAYS"))
                {
                    let _ = parser.next();
                }
                if parser.at_keyword(Keyword::As) {
                    let _ = parser.next();
                }
                if matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("ROW"))
                    || parser.at_keyword(Keyword::Row)
                {
                    let _ = parser.next();
                }
                if parser.at_keyword(Keyword::Start)
                    || parser.at_keyword(Keyword::End)
                    || matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("START"))
                {
                    let _ = parser.next();
                }
                if matches!(parser.peek(), Some(Token::Identifier(w)) if w.eq_ignore_ascii_case("HIDDEN"))
                {
                    let _ = parser.next();
                }
            }
            _ => break,
        }
    }

    Ok(ColumnDef {
        name,
        data_type,
        is_nullable,
        is_identity,
        identity_spec,
        is_primary_key,
        is_unique,
        default_expr,
        default_constraint_name,
        check_expr,
        check_constraint_name,
        computed_expr,
        foreign_key,
        collation,
        is_clustered,
        with_values,
        masking_function,
    })
}

pub fn parse_table_body(
    parser: &mut Parser,
) -> ParseResult<(Vec<ColumnDef>, Vec<TableConstraint>)> {
    let mut columns = Vec::new();
    let mut constraints = Vec::new();

    loop {
        if matches!(parser.peek(), Some(Token::RParen)) || parser.is_empty() {
            break;
        }

        // Inline table index definition: INDEX name [UNIQUE] [CLUSTERED|NONCLUSTERED] (cols) [INCLUDE (...)] [WHERE ...] [WITH (...)]
        let is_inline_index = matches!(parser.peek(), Some(Token::Keyword(Keyword::Index)))
            || matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("INDEX"));
        let is_period = matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("PERIOD"))
            && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::For)));
        if is_inline_index {
            skip_inline_table_index(parser)?;
        } else if is_period {
            // PERIOD FOR SYSTEM_TIME (start_col, end_col)
            let _ = parser.next(); // PERIOD
            let _ = parser.next(); // FOR
            if matches!(
                parser.peek(),
                Some(Token::Identifier(_)) | Some(Token::Keyword(_))
            ) {
                let _ = parser.next(); // SYSTEM_TIME
            }
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                let start_col = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("period start column")),
                };
                if matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                }
                let end_col = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(k)) => k.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("period end column")),
                };
                parser.expect_rparen()?;
                constraints.push(TableConstraint::SystemTimePeriod { start_col, end_col });
            }
        } else {
            let mut is_constraint = false;
            if let Some(Token::Keyword(kw)) = parser.peek() {
                if matches!(
                    kw,
                    Keyword::Constraint
                        | Keyword::Primary
                        | Keyword::Unique
                        | Keyword::Foreign
                        | Keyword::Check
                ) {
                    is_constraint = true;
                }
            }

            if is_constraint {
                constraints.push(parse_table_constraint(parser)?);
            } else {
                columns.push(parse_column_def(parser)?);
            }
        }

        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
            continue;
        }
        // T-SQL allows a table-level constraint right after a column definition without a comma.
        if matches!(
            parser.peek(),
            Some(Token::Keyword(
                Keyword::Constraint
                    | Keyword::Primary
                    | Keyword::Unique
                    | Keyword::Foreign
                    | Keyword::Check
                    | Keyword::Index
            ))
        ) {
            continue;
        }
        break;
    }

    Ok((columns, constraints))
}

fn skip_balanced_parens(parser: &mut Parser) -> ParseResult<()> {
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
    Ok(())
}

fn skip_optional_not_for_replication(parser: &mut Parser) {
    if parser.at_keyword(Keyword::Not)
        && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::For)))
        && matches!(parser.peek_at(2), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("REPLICATION"))
    {
        let _ = parser.next();
        let _ = parser.next();
        let _ = parser.next();
    }
}

fn skip_optional_index_storage_clauses(parser: &mut Parser) -> ParseResult<()> {
    loop {
        if parser.at_keyword(Keyword::With) {
            let saved = parser.save();
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::LParen)) {
                skip_balanced_parens(parser)?;
                continue;
            }
            parser.restore(saved);
        }
        if parser.at_keyword(Keyword::On) {
            let saved = parser.save();
            let _ = parser.next();
            if matches!(
                parser.peek(),
                Some(Token::Keyword(Keyword::Delete | Keyword::Update))
            ) {
                parser.restore(saved);
                break;
            }
            let _ = super::parse_multipart_name(parser)?;
            if matches!(parser.peek(), Some(Token::LParen)) {
                skip_balanced_parens(parser)?;
            }
            continue;
        }
        break;
    }
    Ok(())
}

fn skip_inline_table_index(parser: &mut Parser) -> ParseResult<()> {
    let _ = parser.next(); // INDEX
                           // Index name
    match parser.next() {
        Some(Token::Identifier(_)) | Some(Token::Keyword(_)) => {}
        _ => return parser.backtrack(Expected::Description("index name")),
    }
    // Optional UNIQUE / CLUSTERED / NONCLUSTERED in any order
    loop {
        if parser.at_keyword(Keyword::Unique)
            || parser.at_keyword(Keyword::Clustered)
            || parser.at_keyword(Keyword::Nonclustered)
        {
            let _ = parser.next();
        } else {
            break;
        }
    }
    // Key columns (...)
    skip_balanced_parens(parser)?;
    // Optional INCLUDE (...), WHERE ..., WITH (...), ON filegroup
    loop {
        match parser.peek() {
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("INCLUDE") => {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    skip_balanced_parens(parser)?;
                }
            }
            Some(Token::Keyword(Keyword::With)) => {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    skip_balanced_parens(parser)?;
                }
            }
            Some(Token::Keyword(Keyword::Where)) => {
                let _ = parser.next();
                let _ = crate::parser::parse::expressions::parse_expr(parser)?;
            }
            Some(Token::Keyword(Keyword::On)) => {
                let _ = parser.next();
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    skip_balanced_parens(parser)?;
                }
            }
            _ => break,
        }
    }
    Ok(())
}

pub fn parse_table_constraint(parser: &mut Parser) -> ParseResult<TableConstraint> {
    let mut name = None;
    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Constraint))) {
        let _ = parser.next();
        name = Some(match parser.next() {
            Some(Token::Identifier(id)) => id.clone(),
            Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("constraint name")),
        });
    }

    let kw = match parser.next() {
        Some(Token::Keyword(kw)) => *kw,
        _ => return parser.backtrack(Expected::Description("constraint keyword")),
    };

    match kw {
        Keyword::Primary => {
            parser.expect_keyword(Keyword::Key)?;
            let mut is_clustered = true; // PRIMARY KEY is clustered by default
            if parser.at_keyword(Keyword::Clustered) {
                let _ = parser.next();
                is_clustered = true;
            } else if parser.at_keyword(Keyword::Nonclustered) {
                let _ = parser.next();
                is_clustered = false;
            }
            parser.expect_lparen()?;
            let columns =
                crate::parser::parse::expressions::parse_comma_list(parser, parse_index_column)?;
            parser.expect_rparen()?;
            skip_optional_index_storage_clauses(parser)?;
            Ok(TableConstraint::PrimaryKey {
                name,
                columns,
                is_clustered,
            })
        }
        Keyword::Unique => {
            let mut is_clustered = false;
            if parser.at_keyword(Keyword::Clustered) {
                let _ = parser.next();
                is_clustered = true;
            } else if parser.at_keyword(Keyword::Nonclustered) {
                let _ = parser.next();
                is_clustered = false;
            }
            parser.expect_lparen()?;
            let columns =
                crate::parser::parse::expressions::parse_comma_list(parser, parse_index_column)?;
            parser.expect_rparen()?;
            skip_optional_index_storage_clauses(parser)?;
            Ok(TableConstraint::Unique {
                name,
                columns,
                is_clustered,
            })
        }
        Keyword::Foreign => {
            parser.expect_keyword(Keyword::Key)?;
            parser.expect_lparen()?;
            let columns =
                crate::parser::parse::expressions::parse_comma_list(parser, |p| match p.next() {
                    Some(Token::Identifier(id)) => Ok(id.clone()),
                    Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                    _ => p.backtrack(Expected::Description("column name")),
                })?;
            parser.expect_rparen()?;
            parser.expect_keyword(Keyword::References)?;
            let ref_table = super::parse_multipart_name(parser)?;
            let mut ref_columns = Vec::new();
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                ref_columns = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                    match p.next() {
                        Some(Token::Identifier(id)) => Ok(id.clone()),
                        Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                        _ => p.backtrack(Expected::Description("column name")),
                    }
                })?;
                parser.expect_rparen()?;
            }
            let mut on_delete = None;
            let mut on_update = None;
            while let Some(Token::Keyword(kw)) = parser.peek() {
                if *kw == Keyword::On {
                    let saved = parser.save();
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Delete))) {
                        let _ = parser.next();
                        on_delete = Some(parse_referential_action(parser)?);
                    } else if matches!(parser.peek(), Some(Token::Keyword(Keyword::Update))) {
                        let _ = parser.next();
                        on_update = Some(parse_referential_action(parser)?);
                    } else {
                        parser.restore(saved);
                        break;
                    }
                } else {
                    break;
                }
            }
            skip_optional_not_for_replication(parser);
            Ok(TableConstraint::ForeignKey {
                name,
                columns,
                ref_table,
                ref_columns,
                on_delete,
                on_update,
            })
        }
        Keyword::Check => {
            skip_optional_not_for_replication(parser);
            parser.expect_lparen()?;
            let expr = crate::parser::parse::expressions::parse_expr(parser)?;
            parser.expect_rparen()?;
            Ok(TableConstraint::Check { name, expr })
        }
        Keyword::Default => {
            let expr = crate::parser::parse::expressions::parse_expr(parser)?;
            parser.expect_keyword(Keyword::For)?;
            let column = match parser.next() {
                Some(Token::Identifier(id)) => id.clone(),
                Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
                _ => return parser.backtrack(Expected::Description("column name")),
            };
            Ok(TableConstraint::Default { name, column, expr })
        }
        _ => parser.backtrack(Expected::Description("constraint type")),
    }
}

pub fn parse_create_index(
    parser: &mut Parser,
    is_unique: bool,
    is_clustered: bool,
) -> ParseResult<CreateIndexStmt> {
    let name = super::parse_multipart_name(parser)?;
    parser.expect_keyword(Keyword::On)?;
    let table = super::parse_multipart_name(parser)?;
    parser.expect_lparen()?;
    let columns = crate::parser::parse::expressions::parse_comma_list(parser, parse_index_column)?;
    parser.expect_rparen()?;

    // Optional INCLUDE (col1, col2, ...)
    if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("INCLUDE")) {
        let _ = parser.next();
        parser.expect_lparen()?;
        let _ = crate::parser::parse::expressions::parse_comma_list(parser, parse_index_column)?;
        parser.expect_rparen()?;
    }

    // Optional filtered index WHERE <predicate>
    let filter_predicate = if parser.at_keyword(Keyword::Where) {
        let _ = parser.next();
        Some(Box::new(crate::parser::parse::expressions::parse_expr(
            parser,
        )?))
    } else {
        None
    };

    let mut options = Vec::new();
    if parser.at_keyword(Keyword::With) {
        let _ = parser.next();
        parser.expect_lparen()?;
        let parsed_opts =
            crate::parser::parse::expressions::parse_comma_list(parser, parse_index_option_opt)?;
        parser.expect_rparen()?;
        options.extend(parsed_opts.into_iter().flatten());
    }

    // Optional ON <filegroup> or ON <partition_scheme>(<column>)
    if parser.at_keyword(Keyword::On) {
        let _ = parser.next();
        let _ = super::parse_multipart_name(parser)?;
        if matches!(parser.peek(), Some(Token::LParen)) {
            skip_balanced_parens(parser)?;
        }
    }

    Ok(CreateIndexStmt {
        name,
        table,
        is_unique,
        is_clustered,
        columns,
        options,
        filter_predicate,
    })
}

fn parse_index_column(parser: &mut Parser) -> ParseResult<IndexColumn> {
    let name = match parser.next() {
        Some(Token::Identifier(id)) => id.clone(),
        Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
        _ => return parser.backtrack(Expected::Description("column name")),
    };
    let mut is_desc = false;
    if parser.at_keyword(Keyword::Desc) {
        let _ = parser.next();
        is_desc = true;
    } else if parser.at_keyword(Keyword::Asc) {
        let _ = parser.next();
        is_desc = false;
    }
    Ok(IndexColumn { name, is_desc })
}

fn parse_index_option_opt(parser: &mut Parser) -> ParseResult<Option<IndexOption>> {
    if parser.at_keyword(Keyword::Fillfactor) {
        let _ = parser.next();
        if let Some(Token::Operator(op)) = parser.next() {
            if op != "=" {
                return parser.backtrack(Expected::Description("="));
            }
        } else {
            return parser.backtrack(Expected::Description("="));
        }
        if let Some(Token::Number { value: n, .. }) = parser.next() {
            Ok(Some(IndexOption::FillFactor(*n as u8)))
        } else {
            parser.backtrack(Expected::Description("number"))
        }
    } else {
        // Accept any standard SQL Server index option such as PAD_INDEX = ON,
        // SORT_IN_TEMPDB = OFF, ALLOW_ROW_LOCKS = ON, DATA_COMPRESSION = PAGE, etc.
        match parser.next() {
            Some(Token::Identifier(_)) | Some(Token::Keyword(_)) => {}
            _ => return parser.backtrack(Expected::Description("index option")),
        }
        if matches!(parser.peek(), Some(Token::Operator(op)) if op == "=") {
            let _ = parser.next();
            let _ = parser.next();
        }
        Ok(None)
    }
}

pub fn parse_create_type(parser: &mut Parser) -> ParseResult<Statement> {
    let name = super::parse_multipart_name(parser)?;
    if parser.at_keyword(Keyword::From) {
        let _ = parser.next();
        let base_type = crate::parser::parse::expressions::parse_data_type(parser)?;
        let mut nullable = true;
        if parser.at_keyword(Keyword::Not) {
            let _ = parser.next();
            parser.expect_keyword(Keyword::Null)?;
            nullable = false;
        } else if parser.at_keyword(Keyword::Null) {
            let _ = parser.next();
            nullable = true;
        }
        return Ok(Statement::Ddl(Box::new(DdlStatement::CreateScalarType {
            name,
            base_type,
            nullable,
        })));
    }
    parser.expect_keyword(Keyword::As)?;
    parser.expect_keyword(Keyword::Table)?;
    parser.expect_lparen()?;
    let (columns, _constraints) = parse_table_body(parser)?;
    parser.expect_rparen()?;
    Ok(Statement::Ddl(Box::new(DdlStatement::CreateType {
        name,
        columns,
    })))
}

pub fn parse_create_schema(parser: &mut Parser) -> ParseResult<Statement> {
    let name = match parser.next() {
        Some(Token::Identifier(id)) => id.clone(),
        Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
        _ => return parser.backtrack(Expected::Description("schema name")),
    };
    if parser.at_keyword(Keyword::Authorization) {
        let _ = parser.next();
        match parser.next() {
            Some(Token::Identifier(_)) | Some(Token::Keyword(_)) => {}
            _ => return parser.backtrack(Expected::Description("authorization principal")),
        }
    }
    Ok(Statement::Ddl(Box::new(DdlStatement::CreateSchema(name))))
}

pub fn parse_create_sequence(parser: &mut Parser) -> ParseResult<Statement> {
    let name = super::parse_multipart_name(parser)?;
    let mut data_type = None;
    let mut start_with = None;
    let mut increment_by = None;
    let mut min_value = None;
    let mut max_value = None;
    let mut cycle = false;

    while !parser.is_empty() {
        if matches!(parser.peek(), Some(Token::Semicolon) | Some(Token::Go)) {
            break;
        }

        if parser.at_keyword(Keyword::As) {
            let _ = parser.next();
            data_type = Some(crate::parser::parse::expressions::parse_data_type(parser)?);
        } else if parser.at_keyword(Keyword::Start) {
            let _ = parser.next();
            parser.expect_keyword(Keyword::With)?;
            start_with = Some(parse_signed_int(parser)?);
        } else if parser.at_keyword(Keyword::Increment) {
            let _ = parser.next();
            parser.expect_keyword(Keyword::By)?;
            increment_by = Some(parse_signed_int(parser)?);
        } else if parser.at_keyword(Keyword::Minvalue) {
            let _ = parser.next();
            min_value = Some(parse_signed_int(parser)?);
        } else if parser.at_keyword(Keyword::Maxvalue) {
            let _ = parser.next();
            max_value = Some(parse_signed_int(parser)?);
        } else if parser.at_keyword(Keyword::No) {
            let _ = parser.next();
            if parser.at_keyword(Keyword::Minvalue) {
                let _ = parser.next();
            } else if parser.at_keyword(Keyword::Maxvalue) {
                let _ = parser.next();
            } else if parser.at_keyword(Keyword::Cycle) {
                let _ = parser.next();
                cycle = false;
            } else {
                return parser.backtrack(Expected::Description("MINVALUE, MAXVALUE, or CYCLE"));
            }
        } else if parser.at_keyword(Keyword::Cycle) {
            let _ = parser.next();
            cycle = true;
        } else {
            break;
        }
    }

    Ok(Statement::Ddl(Box::new(DdlStatement::CreateSequence {
        name,
        data_type,
        start_with,
        increment_by,
        min_value,
        max_value,
        cycle,
    })))
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

fn parse_referential_action(parser: &mut Parser) -> ParseResult<ReferentialAction> {
    match parser.next() {
        Some(Token::Keyword(k)) => match *k {
            Keyword::No => {
                parser.expect_keyword(Keyword::Action)?;
                Ok(ReferentialAction::NoAction)
            }
            Keyword::Cascade => Ok(ReferentialAction::Cascade),
            Keyword::Set => {
                if matches!(parser.peek(), Some(Token::Keyword(Keyword::Null))) {
                    let _ = parser.next();
                    Ok(ReferentialAction::SetNull)
                } else if matches!(parser.peek(), Some(Token::Keyword(Keyword::Default))) {
                    let _ = parser.next();
                    Ok(ReferentialAction::SetDefault)
                } else {
                    parser.backtrack(Expected::Description("NULL or DEFAULT"))
                }
            }
            _ => parser.backtrack(Expected::Description("referential action")),
        },
        _ => parser.backtrack(Expected::Description("referential action")),
    }
}

pub fn parse_alter_table_add_constraint(parser: &mut Parser) -> ParseResult<TableConstraint> {
    let constraint_name = match parser.next() {
        Some(Token::Identifier(id)) => id.clone(),
        Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
        _ => return parser.backtrack(Expected::Description("constraint name")),
    };
    let constraint = if parser.at_keyword(Keyword::Primary) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Key)?;
        let mut is_clustered = true; // PRIMARY KEY is clustered by default
        if parser.at_keyword(Keyword::Clustered) {
            let _ = parser.next();
            is_clustered = true;
        } else if parser.at_keyword(Keyword::Nonclustered) {
            let _ = parser.next();
            is_clustered = false;
        }
        parser.expect_lparen()?;
        let columns =
            crate::parser::parse::expressions::parse_comma_list(parser, parse_index_column)?;
        parser.expect_rparen()?;
        skip_optional_index_storage_clauses(parser)?;
        TableConstraint::PrimaryKey {
            name: Some(constraint_name),
            columns,
            is_clustered,
        }
    } else if parser.at_keyword(Keyword::Unique) {
        let _ = parser.next();
        let mut is_clustered = false;
        if parser.at_keyword(Keyword::Clustered) {
            let _ = parser.next();
            is_clustered = true;
        } else if parser.at_keyword(Keyword::Nonclustered) {
            let _ = parser.next();
            is_clustered = false;
        }
        parser.expect_lparen()?;
        let columns =
            crate::parser::parse::expressions::parse_comma_list(parser, parse_index_column)?;
        parser.expect_rparen()?;
        skip_optional_index_storage_clauses(parser)?;
        TableConstraint::Unique {
            name: Some(constraint_name),
            columns,
            is_clustered,
        }
    } else if parser.at_keyword(Keyword::Foreign) {
        let _ = parser.next();
        parser.expect_keyword(Keyword::Key)?;
        parser.expect_lparen()?;
        let columns =
            crate::parser::parse::expressions::parse_comma_list(parser, |p| match p.next() {
                Some(Token::Identifier(id)) => Ok(id.clone()),
                Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                _ => p.backtrack(Expected::Description("column name")),
            })?;
        parser.expect_rparen()?;
        parser.expect_keyword(Keyword::References)?;
        let ref_table = super::parse_multipart_name(parser)?;
        let mut ref_columns = Vec::new();
        if matches!(parser.peek(), Some(Token::LParen)) {
            let _ = parser.next();
            ref_columns =
                crate::parser::parse::expressions::parse_comma_list(parser, |p| match p.next() {
                    Some(Token::Identifier(id)) => Ok(id.clone()),
                    Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                    _ => p.backtrack(Expected::Description("column name")),
                })?;
            parser.expect_rparen()?;
        }
        let mut on_delete = None;
        let mut on_update = None;
        while let Some(Token::Keyword(kw)) = parser.peek() {
            if *kw == Keyword::On {
                let _ = parser.next();
                if parser.at_keyword(Keyword::Delete) {
                    let _ = parser.next();
                    on_delete = Some(parse_referential_action(parser)?);
                } else if parser.at_keyword(Keyword::Update) {
                    let _ = parser.next();
                    on_update = Some(parse_referential_action(parser)?);
                }
            } else {
                break;
            }
        }
        skip_optional_not_for_replication(parser);
        TableConstraint::ForeignKey {
            name: Some(constraint_name),
            columns,
            ref_table,
            ref_columns,
            on_delete,
            on_update,
        }
    } else if parser.at_keyword(Keyword::Check) {
        let _ = parser.next();
        skip_optional_not_for_replication(parser);
        parser.expect_lparen()?;
        let expr = crate::parser::parse::expressions::parse_expr(parser)?;
        parser.expect_rparen()?;
        TableConstraint::Check {
            name: Some(constraint_name),
            expr,
        }
    } else if parser.at_keyword(Keyword::Default) {
        let _ = parser.next();
        let expr = crate::parser::parse::expressions::parse_expr(parser)?;
        parser.expect_keyword(Keyword::For)?;
        let column = match parser.next() {
            Some(Token::Identifier(id)) => id.clone(),
            Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
            _ => return parser.backtrack(Expected::Description("column name")),
        };
        TableConstraint::Default {
            name: Some(constraint_name),
            column,
            expr,
        }
    } else {
        return parser.backtrack(Expected::Description("constraint type"));
    };
    Ok(constraint)
}
