use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

fn wrap_parenthesized_set_op(inner: SelectStmt) -> SelectStmt {
    if inner.set_op.is_none() {
        return inner;
    }
    SelectStmt {
        distinct: false,
        top: None,
        projection: vec![SelectItem {
            expr: Expr::Wildcard,
            alias: None,
        }],
        into_table: None,
        from: Some(TableRef {
            factor: TableFactor::Derived(Box::new(inner)),
            alias: Some("__paren_sub".to_string()),
            pivot: None,
            unpivot: None,
            hints: Vec::new(),
        }),
        joins: Vec::new(),
        applies: Vec::new(),
        selection: None,
        group_by: Vec::new(),
        having: None,
        order_by: Vec::new(),
        offset: None,
        fetch: None,
        set_op: None,
        extra: None,
    }
}

pub fn parse_select(parser: &mut Parser) -> ParseResult<SelectStmt> {
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        let mut current = parse_select(parser)?;
        parser.expect_rparen()?;
        if matches!(
            parser.peek(),
            Some(Token::Keyword(
                Keyword::Union | Keyword::Intersect | Keyword::Except
            ))
        ) {
            current = wrap_parenthesized_set_op(current);
            parse_set_op_tail(parser, &mut current)?;
        }
        if let Some(for_xml) = parse_for_xml_clause(parser)? {
            current.extra.get_or_insert_with(Default::default).for_xml = Some(for_xml);
        } else if let Some(for_json) = parse_for_json_clause(parser)? {
            current.extra.get_or_insert_with(Default::default).for_json = Some(for_json);
        }
        skip_query_option_clause(parser)?;
        return Ok(current);
    }
    parser.expect_keyword(Keyword::Select)?;
    parse_select_body(parser)
}

pub fn parse_select_body(parser: &mut Parser) -> ParseResult<SelectStmt> {
    let mut current = parse_single_select_body(parser)?;
    parse_set_op_tail(parser, &mut current)?;

    if let Some(for_xml) = parse_for_xml_clause(parser)? {
        current.extra.get_or_insert_with(Default::default).for_xml = Some(for_xml);
    } else if let Some(for_json) = parse_for_json_clause(parser)? {
        current.extra.get_or_insert_with(Default::default).for_json = Some(for_json);
    }

    skip_query_option_clause(parser)?;
    Ok(current)
}

fn parse_set_op_tail(parser: &mut Parser, current: &mut SelectStmt) -> ParseResult<()> {
    while let Some(Token::Keyword(k)) = parser.peek() {
        let kind = match *k {
            Keyword::Union => {
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::Keyword(Keyword::All))) {
                    let _ = parser.next();
                    SetOpKind::UnionAll
                } else {
                    SetOpKind::Union
                }
            }
            Keyword::Intersect => {
                let _ = parser.next();
                SetOpKind::Intersect
            }
            Keyword::Except => {
                let _ = parser.next();
                SetOpKind::Except
            }
            _ => break,
        };

        let right = parse_single_select(parser)?;

        let mut target = &mut *current;
        while let Some(ref mut op) = target.set_op {
            target = &mut op.right;
        }
        target.set_op = Some(Box::new(SetOp { kind, right }));
    }
    Ok(())
}

fn parse_for_xml_clause(parser: &mut Parser) -> ParseResult<Option<ForXmlClause>> {
    if !parser.at_keyword(Keyword::For) {
        return Ok(None);
    }
    let is_xml = match parser.peek_at(1) {
        Some(Token::Identifier(id)) => id.eq_ignore_ascii_case("XML"),
        Some(Token::Keyword(k)) => k.as_sql().eq_ignore_ascii_case("XML"),
        _ => false,
    };
    if !is_xml {
        return Ok(None);
    }
    let _ = parser.next(); // FOR
    let _ = parser.next(); // XML

    let mode = match parser.next() {
        Some(Token::Identifier(id)) => id.to_ascii_uppercase(),
        Some(Token::Keyword(k)) => k.as_sql().to_ascii_uppercase(),
        _ => {
            return parser.backtrack(Expected::Description(
                "XML mode (PATH, RAW, AUTO, EXPLICIT)",
            ))
        }
    };

    let mut path = None;
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        if let Some(Token::String(s)) | Some(Token::NString(s)) = parser.peek() {
            path = Some(s.clone());
            let _ = parser.next();
        }
        parser.expect_rparen()?;
    }

    let mut root = None;
    let mut type_directive = false;
    while matches!(parser.peek(), Some(Token::Comma)) {
        let _ = parser.next();
        let dir = match parser.peek() {
            Some(Token::Identifier(id)) => id.to_ascii_uppercase(),
            Some(Token::Keyword(k)) => k.as_sql().to_ascii_uppercase(),
            _ => break,
        };
        let _ = parser.next();
        match dir.as_str() {
            "TYPE" => type_directive = true,
            "ROOT" => {
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::String(s)) | Some(Token::NString(s)) = parser.peek() {
                        root = Some(s.clone());
                        let _ = parser.next();
                    }
                    parser.expect_rparen()?;
                } else {
                    root = Some("root".to_string());
                }
            }
            "ELEMENTS" => {
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("XSINIL") || id.eq_ignore_ascii_case("ABSENT"))
                {
                    let _ = parser.next();
                }
            }
            "BINARY" => {
                if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("BASE64"))
                {
                    let _ = parser.next();
                }
            }
            _ => {
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
    }

    Ok(Some(ForXmlClause {
        mode,
        path,
        root,
        type_directive,
    }))
}

fn parse_for_json_clause(parser: &mut Parser) -> ParseResult<Option<ForJsonClause>> {
    if !parser.at_keyword(Keyword::For) {
        return Ok(None);
    }
    let is_json = match parser.peek_at(1) {
        Some(Token::Identifier(id)) => id.eq_ignore_ascii_case("JSON"),
        Some(Token::Keyword(k)) => k.as_sql().eq_ignore_ascii_case("JSON"),
        _ => false,
    };
    if !is_json {
        return Ok(None);
    }
    let _ = parser.next(); // FOR
    let _ = parser.next(); // JSON

    let mode = match parser.next() {
        Some(Token::Identifier(id)) => id.to_ascii_uppercase(),
        Some(Token::Keyword(k)) => k.as_sql().to_ascii_uppercase(),
        _ => return parser.backtrack(Expected::Description("JSON mode (PATH, AUTO)")),
    };
    if mode != "PATH" && mode != "AUTO" {
        return parser.backtrack(Expected::Description("JSON mode (PATH, AUTO)"));
    }

    let mut root = None;
    let mut include_null_values = false;
    let mut without_array_wrapper = false;

    while matches!(parser.peek(), Some(Token::Comma)) {
        let _ = parser.next();
        let dir = match parser.peek() {
            Some(Token::Identifier(id)) => id.to_ascii_uppercase(),
            Some(Token::Keyword(k)) => k.as_sql().to_ascii_uppercase(),
            _ => break,
        };
        let _ = parser.next();
        match dir.as_str() {
            "INCLUDE_NULL_VALUES" => include_null_values = true,
            "WITHOUT_ARRAY_WRAPPER" => without_array_wrapper = true,
            "ROOT" => {
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::String(s)) | Some(Token::NString(s)) = parser.peek() {
                        root = Some(s.clone());
                        let _ = parser.next();
                    }
                    parser.expect_rparen()?;
                } else {
                    root = Some("root".to_string());
                }
            }
            _ => {
                return parser.backtrack(Expected::Description(
                    "FOR JSON option (ROOT, INCLUDE_NULL_VALUES, WITHOUT_ARRAY_WRAPPER)",
                ));
            }
        }
    }

    Ok(Some(ForJsonClause {
        mode,
        root,
        include_null_values,
        without_array_wrapper,
    }))
}

pub(crate) fn skip_query_option_clause(parser: &mut Parser) -> ParseResult<()> {
    let is_option = match parser.peek() {
        Some(Token::Identifier(id)) => id.eq_ignore_ascii_case("OPTION"),
        Some(Token::Keyword(k)) => k.as_sql().eq_ignore_ascii_case("OPTION"),
        _ => false,
    };
    if is_option && matches!(parser.peek_at(1), Some(Token::LParen)) {
        let _ = parser.next(); // OPTION
        let _ = parser.next(); // (
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
    Ok(())
}

fn parse_single_select(parser: &mut Parser) -> ParseResult<SelectStmt> {
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        let stmt = parse_select(parser)?;
        parser.expect_rparen()?;
        return Ok(wrap_parenthesized_set_op(stmt));
    }
    parser.expect_keyword(Keyword::Select)?;
    parse_single_select_body(parser)
}

pub fn parse_single_select_body(parser: &mut Parser) -> ParseResult<SelectStmt> {
    let mut distinct = false;
    if let Some(Token::Keyword(Keyword::Distinct)) = parser.peek() {
        let _ = parser.next();
        distinct = true;
    }

    let mut top = None;
    if let Some(Token::Keyword(Keyword::Top)) = parser.peek() {
        let _ = parser.next();
        let value = crate::parser::parse::expressions::parse_primary(parser)?;
        let mut percent = false;
        if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("PERCENT"))
        {
            let _ = parser.next();
            percent = true;
        }
        let mut with_ties = false;
        if parser.at_keyword(Keyword::With) {
            let saved = parser.save();
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("TIES"))
            {
                let _ = parser.next();
                with_ties = true;
            } else {
                parser.restore(saved);
            }
        }
        top = Some(TopSpec {
            value,
            percent,
            with_ties,
        });
    }

    let projection = parse_projection(parser)?;

    let mut into_table = None;
    if let Some(Token::Keyword(Keyword::Into)) = parser.peek() {
        let _ = parser.next();
        into_table = Some(parse_object_name(parse_multipart_name(parser)?));
    }

    let mut from = None;
    let mut joins = Vec::new();
    if let Some(tok) = parser.peek() {
        let is_from = match tok {
            Token::Keyword(Keyword::From) => true,
            Token::Identifier(id) => id.eq_ignore_ascii_case("FROM"),
            _ => false,
        };
        if is_from {
            let _ = parser.next();
            let tables =
                crate::parser::parse::expressions::parse_comma_list(parser, parse_table_ref)?;
            let mut iter = tables.into_iter();
            from = iter.next();
            for table in iter {
                joins.push(JoinClause {
                    join_type: JoinType::Cross,
                    table,
                    on: None,
                });
            }
        }
    }

    let mut applies = Vec::new();
    parse_joins_and_applies(parser, &mut joins, &mut applies)?;

    let mut selection = None;
    if let Some(Token::Keyword(Keyword::Where)) = parser.peek() {
        let _ = parser.next();
        selection = Some(crate::parser::parse::expressions::parse_expr(parser)?);
    }

    let mut group_by = Vec::new();
    let mut grouping_sets = Vec::new();
    if let Some(Token::Keyword(Keyword::Group)) = parser.peek() {
        let _ = parser.next();
        if matches!(parser.peek(), Some(Token::Keyword(Keyword::By))) {
            let _ = parser.next();
        }
        let (gb, gs) = parse_group_by_items(parser)?;
        group_by = gb;
        grouping_sets = gs;
    }

    let mut having = None;
    if let Some(Token::Keyword(Keyword::Having)) = parser.peek() {
        let _ = parser.next();
        having = Some(crate::parser::parse::expressions::parse_expr(parser)?);
    }

    let mut order_by = Vec::new();
    if let Some(Token::Keyword(Keyword::Order)) = parser.peek() {
        let _ = parser.next();
        if matches!(parser.peek(), Some(Token::Keyword(Keyword::By))) {
            let _ = parser.next();
        }
        order_by =
            crate::parser::parse::expressions::parse_comma_list(parser, parse_order_by_expr)?;
    }

    let mut offset = None;
    let mut fetch = None;
    if let Some(Token::Keyword(Keyword::Offset)) = parser.peek() {
        let _ = parser.next();
        offset = Some(crate::parser::parse::expressions::parse_expr(parser)?);
        parser.expect_keyword(Keyword::Rows)?;
        if let Some(Token::Keyword(Keyword::Fetch)) = parser.peek() {
            let _ = parser.next();
            parser.expect_keyword(Keyword::Next)?;
            fetch = Some(crate::parser::parse::expressions::parse_expr(parser)?);
            parser.expect_keyword(Keyword::Rows)?;
            parser.expect_keyword(Keyword::Only)?;
        }
    }

    let extra = if grouping_sets.is_empty() {
        None
    } else {
        Some(Box::new(SelectExtra {
            for_xml: None,
            for_json: None,
            grouping_sets,
        }))
    };

    Ok(SelectStmt {
        distinct,
        top,
        projection,
        into_table,
        from,
        joins,
        applies,
        selection,
        group_by,
        having,
        order_by,
        offset,
        fetch,
        set_op: None,
        extra,
    })
}

fn parse_projection(parser: &mut Parser) -> ParseResult<Vec<SelectItem>> {
    let mut items = Vec::new();
    loop {
        items.push(parse_select_item(parser)?);
        if let Some(Token::Comma) = parser.peek() {
            let _ = parser.next();
            continue;
        }
        if let Some(Token::Keyword(k)) = parser.peek() {
            if matches!(
                k,
                Keyword::From
                    | Keyword::Into
                    | Keyword::Where
                    | Keyword::Group
                    | Keyword::Order
                    | Keyword::Having
                    | Keyword::Union
                    | Keyword::Intersect
                    | Keyword::Except
            ) {
                break;
            }
        }
        break;
    }
    Ok(items)
}

fn starts_with_select_inside_parens(parser: &Parser) -> bool {
    let mut offset = 0usize;
    while matches!(parser.peek_at(offset), Some(Token::LParen)) {
        offset += 1;
    }
    matches!(
        parser.peek_at(offset),
        Some(Token::Keyword(Keyword::Select))
    )
}

fn parse_group_by_items(parser: &mut Parser) -> ParseResult<(Vec<Expr>, Vec<Vec<Expr>>)> {
    let mut all_exprs: Vec<Expr> = Vec::new();
    let mut has_extended = false;
    // Each item in GROUP BY expands to a list of grouping sets; the full clause is
    // the Cartesian product of those item expansions.
    let mut item_expansions: Vec<Vec<Vec<Expr>>> = Vec::new();

    let push_unique = |all: &mut Vec<Expr>, e: &Expr| {
        if !all.contains(e) {
            all.push(e.clone());
        }
    };

    loop {
        let is_grouping_sets = matches!(
            parser.peek(),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("GROUPING")
        ) && matches!(
            parser.peek_at(1),
            Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("SETS")
        );
        let rollup_or_cube = match parser.peek() {
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("ROLLUP")
                    && matches!(parser.peek_at(1), Some(Token::LParen)) =>
            {
                Some(false)
            }
            Some(Token::Identifier(id))
                if id.eq_ignore_ascii_case("CUBE")
                    && matches!(parser.peek_at(1), Some(Token::LParen)) =>
            {
                Some(true)
            }
            _ => None,
        };

        if is_grouping_sets {
            has_extended = true;
            let _ = parser.next(); // GROUPING
            let _ = parser.next(); // SETS
            parser.expect_lparen()?;
            let mut sets: Vec<Vec<Expr>> = Vec::new();
            loop {
                if matches!(parser.peek(), Some(Token::RParen)) {
                    break;
                }
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    let set_exprs = if matches!(parser.peek(), Some(Token::RParen)) {
                        Vec::new()
                    } else {
                        crate::parser::parse::expressions::parse_comma_list(
                            parser,
                            crate::parser::parse::expressions::parse_expr,
                        )?
                    };
                    parser.expect_rparen()?;
                    for e in &set_exprs {
                        push_unique(&mut all_exprs, e);
                    }
                    sets.push(set_exprs);
                } else {
                    let e = crate::parser::parse::expressions::parse_expr(parser)?;
                    push_unique(&mut all_exprs, &e);
                    sets.push(vec![e]);
                }
                if matches!(parser.peek(), Some(Token::Comma)) {
                    let _ = parser.next();
                } else {
                    break;
                }
            }
            parser.expect_rparen()?;
            item_expansions.push(sets);
        } else if let Some(is_cube) = rollup_or_cube {
            has_extended = true;
            let _ = parser.next(); // ROLLUP | CUBE
            parser.expect_lparen()?;
            let set_exprs = if matches!(parser.peek(), Some(Token::RParen)) {
                Vec::new()
            } else {
                crate::parser::parse::expressions::parse_comma_list(
                    parser,
                    crate::parser::parse::expressions::parse_expr,
                )?
            };
            parser.expect_rparen()?;
            for e in &set_exprs {
                push_unique(&mut all_exprs, e);
            }
            let mut sets: Vec<Vec<Expr>> = Vec::new();
            if is_cube {
                // All 2^n subsets (capped at 10 columns to prevent blowup).
                let n = set_exprs.len().min(10);
                for mask in (0usize..(1usize << n)).rev() {
                    let mut subset = Vec::new();
                    for (bit, expr) in set_exprs.iter().take(n).enumerate() {
                        if (mask & (1usize << (n - 1 - bit))) != 0 {
                            subset.push(expr.clone());
                        }
                    }
                    sets.push(subset);
                }
            } else {
                // ROLLUP(a, b, c) -> (a, b, c), (a, b), (a), ()
                for len in (0..=set_exprs.len()).rev() {
                    sets.push(set_exprs[..len].to_vec());
                }
            }
            item_expansions.push(sets);
        } else {
            let e = crate::parser::parse::expressions::parse_expr(parser)?;
            push_unique(&mut all_exprs, &e);
            item_expansions.push(vec![vec![e]]);
        }

        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
        } else {
            break;
        }
    }

    let grouping_sets = if !has_extended {
        Vec::new()
    } else {
        let mut combined: Vec<Vec<Expr>> = vec![Vec::new()];
        for expansion in item_expansions {
            let mut next = Vec::with_capacity(combined.len() * expansion.len());
            for prefix in &combined {
                for suffix in &expansion {
                    let mut merged = prefix.clone();
                    for e in suffix {
                        if !merged.contains(e) {
                            merged.push(e.clone());
                        }
                    }
                    next.push(merged);
                }
            }
            combined = next;
        }
        combined
    };

    Ok((all_exprs, grouping_sets))
}

pub fn parse_table_ref(parser: &mut Parser) -> ParseResult<TableRef> {
    let (factor, alias, hints) = match parser.peek() {
        Some(Token::LParen) => {
            let _ = parser.next();
            if parser.at_keyword(Keyword::Values) {
                let _ = parser.next();
                let rows = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                    p.expect_lparen()?;
                    let vals = crate::parser::parse::expressions::parse_comma_list(
                        p,
                        crate::parser::parse::expressions::parse_expr,
                    )?;
                    p.expect_rparen()?;
                    Ok(vals)
                })?;
                parser.expect_rparen()?;
                let alias = parse_required_alias(parser)?;
                let mut columns = Vec::new();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    columns = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                        match p.next() {
                            Some(Token::Identifier(id)) => Ok(id.clone()),
                            Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                            _ => p.backtrack(Expected::Description("column name")),
                        }
                    })?;
                    parser.expect_rparen()?;
                }
                (
                    TableFactor::Values { rows, columns },
                    Some(alias),
                    Vec::new(),
                )
            } else if starts_with_select_inside_parens(parser) {
                let mut subquery = Box::new(parse_select(parser)?);
                parser.expect_rparen()?;
                let alias = parse_required_alias(parser)?;
                // Optional derived-table column alias list: (SELECT ...) AS S (c1, c2, ...)
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    let col_aliases =
                        crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                            match p.next() {
                                Some(Token::Identifier(id)) => Ok(id.clone()),
                                Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                                _ => p.backtrack(Expected::Description("column name")),
                            }
                        })?;
                    parser.expect_rparen()?;
                    for (item, col_alias) in
                        subquery.projection.iter_mut().zip(col_aliases.into_iter())
                    {
                        item.alias = Some(col_alias);
                    }
                }
                (TableFactor::Derived(subquery), Some(alias), Vec::new())
            } else {
                let base = parse_table_ref(parser)?;
                let mut joins = Vec::new();
                while !matches!(parser.peek(), Some(Token::RParen)) {
                    match parse_join_clause(parser)? {
                        Some(join) => joins.push(join),
                        None => break,
                    }
                }
                parser.expect_rparen()?;
                let alias = parse_optional_alias(parser);
                (
                    TableFactor::JoinedGroup {
                        base: Box::new(base),
                        joins,
                    },
                    alias,
                    Vec::new(),
                )
            }
        }
        Some(Token::Identifier(_))
        | Some(Token::Keyword(_))
        | Some(Token::Variable(_))
        | Some(Token::Dot) => {
            let name = parse_multipart_name(parser)?;
            if matches!(parser.peek(), Some(Token::LParen)) {
                let _ = parser.next();
                let args = if matches!(parser.peek(), Some(Token::RParen)) {
                    Vec::new()
                } else {
                    crate::parser::parse::expressions::parse_comma_list(
                        parser,
                        crate::parser::parse::expressions::parse_expr,
                    )?
                };
                parser.expect_rparen()?;
                // Optional OPENJSON(...) WITH (col type ['$.path'] [AS JSON], ...) schema
                let mut with_schema = Vec::new();
                if parser.at_keyword(Keyword::With)
                    && matches!(parser.peek_at(1), Some(Token::LParen))
                {
                    let _ = parser.next(); // WITH
                    let _ = parser.next(); // (
                    with_schema =
                        crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                            let col_name = match p.next() {
                                Some(Token::Identifier(id)) => id.clone(),
                                Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
                                _ => return p.backtrack(Expected::Description("column name")),
                            };
                            let data_type = crate::parser::parse::expressions::parse_data_type(p)?;
                            let path = match p.peek() {
                                Some(Token::String(s)) | Some(Token::NString(s)) => {
                                    let s = s.clone();
                                    let _ = p.next();
                                    Some(s)
                                }
                                _ => None,
                            };
                            let mut as_json = false;
                            if p.at_keyword(Keyword::As)
                                && matches!(
                                    p.peek_at(1),
                                    Some(Token::Identifier(id)) if id.eq_ignore_ascii_case("JSON")
                                )
                            {
                                let _ = p.next();
                                let _ = p.next();
                                as_json = true;
                            }
                            Ok(OpenJsonColumn {
                                name: col_name,
                                data_type,
                                path,
                                as_json,
                            })
                        })?;
                    parser.expect_rparen()?;
                }
                let alias = parse_optional_alias(parser);
                // Optional table-valued function/method column alias list: AS Split(a, b)
                let mut column_aliases = Vec::new();
                if alias.is_some() && matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    column_aliases =
                        crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                            match p.next() {
                                Some(Token::Identifier(id)) => Ok(id.clone()),
                                Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                                _ => p.backtrack(Expected::Description("column name")),
                            }
                        })?;
                    parser.expect_rparen()?;
                }
                let factor = TableFactor::TableValuedFunction {
                    name,
                    args,
                    alias: alias.clone(),
                    with_schema,
                    column_aliases,
                };
                return Ok(TableRef {
                    factor,
                    alias,
                    pivot: None,
                    unpivot: None,
                    hints: Vec::new(),
                });
            }
            let mut alias = parse_optional_alias(parser);
            let mut hints = Vec::new();
            if let Some(Token::Keyword(Keyword::With)) = parser.peek() {
                let saved = parser.save();
                let _ = parser.next();
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    hints = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                        let hint_name = if let Some(tok) = p.next() {
                            match tok {
                                Token::Identifier(id) => id.clone(),
                                Token::Keyword(kw) => kw.as_ref().to_string(),
                                _ => return p.backtrack(Expected::Description("identifier")),
                            }
                        } else {
                            return p.backtrack(Expected::Description("identifier"));
                        };
                        // Parameterized table hints: `WITH (INDEX(ix_name))`, `WITH (INDEX = ix_name)`
                        if matches!(p.peek(), Some(Token::Operator(op)) if op == "=") {
                            let _ = p.next();
                            if matches!(p.peek(), Some(Token::LParen)) {
                                let _ = p.next();
                                let mut depth = 1usize;
                                while depth > 0 {
                                    match p.next() {
                                        Some(Token::LParen) => depth += 1,
                                        Some(Token::RParen) => depth -= 1,
                                        Some(_) => {}
                                        None => return p.backtrack(Expected::Description(")")),
                                    }
                                }
                            } else {
                                let _ = p.next();
                            }
                        } else if matches!(p.peek(), Some(Token::LParen)) {
                            let _ = p.next();
                            let mut depth = 1usize;
                            while depth > 0 {
                                match p.next() {
                                    Some(Token::LParen) => depth += 1,
                                    Some(Token::RParen) => depth -= 1,
                                    Some(_) => {}
                                    None => return p.backtrack(Expected::Description(")")),
                                }
                            }
                        }
                        Ok(hint_name)
                    })?;
                    parser.expect_rparen()?;
                } else {
                    parser.restore(saved);
                }
            }
            if alias.is_none() {
                alias = parse_optional_alias(parser);
            }
            // Legacy T-SQL table hints without `WITH`: `FROM tbl alias (NOLOCK)`
            if hints.is_empty() && matches!(parser.peek(), Some(Token::LParen)) {
                let is_table_hint = match parser.peek_at(1) {
                    Some(Token::Identifier(id)) => matches!(
                        id.to_ascii_uppercase().as_str(),
                        "NOLOCK"
                            | "READUNCOMMITTED"
                            | "READCOMMITTED"
                            | "READCOMMITTEDLOCK"
                            | "REPEATABLEREAD"
                            | "SERIALIZABLE"
                            | "HOLDLOCK"
                            | "UPDLOCK"
                            | "XLOCK"
                            | "ROWLOCK"
                            | "PAGLOCK"
                            | "TABLOCK"
                            | "TABLOCKX"
                            | "READPAST"
                            | "NOWAIT"
                            | "FASTFIRSTROW"
                            | "FORCESEEK"
                            | "FORCESCAN"
                            | "NOEXPAND"
                            | "SNAPSHOT"
                    ),
                    Some(Token::Keyword(
                        Keyword::Nolock
                        | Keyword::Rowlock
                        | Keyword::Tablock
                        | Keyword::Holdlock
                        | Keyword::Updlock
                        | Keyword::Xlock
                        | Keyword::Readuncommitted
                        | Keyword::Readcommitted
                        | Keyword::Readpast
                        | Keyword::Index
                        | Keyword::Read
                        | Keyword::Serializable
                        | Keyword::Repeatable
                        | Keyword::Snapshot,
                    )) => true,
                    _ => false,
                };
                if is_table_hint {
                    let _ = parser.next(); // (
                    hints = crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                        let hint_name = match p.next() {
                            Some(Token::Identifier(id)) => id.clone(),
                            Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
                            _ => return p.backtrack(Expected::Description("table hint")),
                        };
                        if matches!(p.peek(), Some(Token::Operator(op)) if op == "=") {
                            let _ = p.next();
                            let _ = p.next();
                        } else if matches!(p.peek(), Some(Token::LParen)) {
                            let _ = p.next();
                            let mut depth = 1usize;
                            while depth > 0 {
                                match p.next() {
                                    Some(Token::LParen) => depth += 1,
                                    Some(Token::RParen) => depth -= 1,
                                    Some(_) => {}
                                    None => return p.backtrack(Expected::Description(")")),
                                }
                            }
                        }
                        Ok(hint_name)
                    })?;
                    parser.expect_rparen()?;
                }
            }
            (TableFactor::Named(parse_object_name(name)), alias, hints)
        }
        _ => return parser.backtrack(Expected::Description("table reference")),
    };

    let mut table = TableRef {
        factor,
        alias,
        pivot: None,
        unpivot: None,
        hints,
    };

    loop {
        match parser.peek() {
            Some(Token::Keyword(Keyword::Pivot)) => {
                let _ = parser.next();
                parser.expect_lparen()?;
                let aggregate_func = match parser.next() {
                    Some(Token::Identifier(id)) => id.clone(),
                    Some(Token::Keyword(kw)) => kw.as_ref().to_string(),
                    _ => return parser.backtrack(Expected::Description("identifier")),
                };
                parser.expect_lparen()?;
                let aggregate_col = parse_multipart_name(parser)?
                    .pop()
                    .ok_or_else(|| parser.error(Expected::Description("identifier")))?;
                parser.expect_rparen()?;
                parser.expect_keyword(Keyword::For)?;
                let pivot_col = parse_multipart_name(parser)?
                    .pop()
                    .ok_or_else(|| parser.error(Expected::Description("identifier")))?;
                parser.expect_keyword(Keyword::In)?;
                parser.expect_lparen()?;
                let pivot_values =
                    crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                        match p.next() {
                            Some(Token::Identifier(id)) => Ok(id.clone()),
                            Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                            _ => p.backtrack(Expected::Description("identifier")),
                        }
                    })?;
                parser.expect_rparen()?;
                parser.expect_rparen()?;
                let alias = parse_required_alias(parser)?;
                table.pivot = Some(Box::new(PivotSpec {
                    aggregate_func,
                    aggregate_col,
                    pivot_col,
                    pivot_values,
                    alias: Some(alias.clone()),
                }));
                table.alias = Some(alias);
            }
            Some(Token::Keyword(Keyword::Unpivot)) => {
                let _ = parser.next();
                parser.expect_lparen()?;
                let value_col = parse_multipart_name(parser)?
                    .pop()
                    .ok_or_else(|| parser.error(Expected::Description("identifier")))?;
                parser.expect_keyword(Keyword::For)?;
                let pivot_col = parse_multipart_name(parser)?
                    .pop()
                    .ok_or_else(|| parser.error(Expected::Description("identifier")))?;
                parser.expect_keyword(Keyword::In)?;
                parser.expect_lparen()?;
                let column_list =
                    crate::parser::parse::expressions::parse_comma_list(parser, |p| {
                        match p.next() {
                            Some(Token::Identifier(id)) => Ok(id.clone()),
                            Some(Token::Keyword(kw)) => Ok(kw.as_ref().to_string()),
                            _ => p.backtrack(Expected::Description("identifier")),
                        }
                    })?;
                parser.expect_rparen()?;
                parser.expect_rparen()?;
                let alias = parse_required_alias(parser)?;
                let source_alias = table.alias.clone();
                table.unpivot = Some(Box::new(UnpivotSpec {
                    value_col,
                    pivot_col,
                    column_list,
                    alias: Some(alias.clone()),
                    source_alias,
                }));
                table.alias = Some(alias);
            }
            _ => break,
        }
    }

    Ok(table)
}

pub fn parse_join_clause(parser: &mut Parser) -> ParseResult<Option<JoinClause>> {
    let Some(Token::Keyword(k)) = parser.peek() else {
        return Ok(None);
    };

    if matches!(
        k,
        Keyword::Cross | Keyword::Left | Keyword::Right | Keyword::Full
    ) && matches!(parser.peek_at(1), Some(Token::Keyword(Keyword::Apply)))
    {
        return Ok(None);
    }

    let join_type = match *k {
        Keyword::Join => {
            let _ = parser.next();
            JoinType::Inner
        }
        Keyword::Inner | Keyword::Left | Keyword::Right | Keyword::Full | Keyword::Cross => {
            let jt = match *k {
                Keyword::Inner => JoinType::Inner,
                Keyword::Left => JoinType::Left,
                Keyword::Right => JoinType::Right,
                Keyword::Full => JoinType::Full,
                Keyword::Cross => JoinType::Cross,
                _ => unreachable!(),
            };
            let _ = parser.next();
            if matches!(parser.peek(), Some(Token::Keyword(Keyword::Outer))) {
                let _ = parser.next();
            }
            parser.expect_keyword(Keyword::Join)?;
            jt
        }
        _ => return Ok(None),
    };

    let mut table = parse_table_ref(parser)?;
    let on = if join_type != JoinType::Cross {
        // ANSI nested join without parentheses: FROM A JOIN B JOIN C ON B.k = C.k ON A.k = B.k
        let mut nested_joins = Vec::new();
        while !parser.at_keyword(Keyword::On) {
            match parse_join_clause(parser)? {
                Some(inner) => nested_joins.push(inner),
                None => break,
            }
        }
        if !nested_joins.is_empty() {
            table = TableRef {
                factor: TableFactor::JoinedGroup {
                    base: Box::new(table),
                    joins: nested_joins,
                },
                alias: None,
                pivot: None,
                unpivot: None,
                hints: Vec::new(),
            };
        }
        parser.expect_keyword(Keyword::On)?;
        Some(crate::parser::parse::expressions::parse_expr(parser)?)
    } else {
        None
    };
    Ok(Some(JoinClause {
        join_type,
        table,
        on,
    }))
}

fn parse_optional_alias(parser: &mut Parser) -> Option<String> {
    let saved = parser.save();
    match parser.peek() {
        Some(Token::Keyword(Keyword::As)) => {
            let _ = parser.next();
            match parser.next() {
                Some(Token::Identifier(alias)) => Some(alias.clone()),
                Some(Token::String(alias)) => Some(alias.clone()),
                Some(Token::Keyword(kw)) => Some(kw.as_ref().to_string()),
                _ => {
                    parser.restore(saved);
                    None
                }
            }
        }
        Some(Token::Keyword(k))
            if !crate::parser::parse::expressions::is_stop_keyword(k.as_sql()) =>
        {
            match parser.next() {
                Some(Token::Identifier(alias)) => Some(alias.clone()),
                Some(Token::Keyword(kw)) => Some(kw.as_ref().to_string()),
                Some(Token::String(alias)) => Some(alias.clone()),
                Some(_) | None => {
                    parser.restore(saved);
                    None
                }
            }
        }
        Some(Token::Identifier(alias))
            if !crate::parser::parse::expressions::is_stop_keyword(alias)
                && !(alias.eq_ignore_ascii_case("OPTION")
                    && matches!(parser.peek_at(1), Some(Token::LParen))) =>
        {
            let alias = alias.clone();
            let _ = parser.next();
            Some(alias)
        }
        Some(Token::String(alias)) => {
            let alias = alias.clone();
            let _ = parser.next();
            Some(alias)
        }
        _ => {
            parser.restore(saved);
            None
        }
    }
}

fn parse_required_alias(parser: &mut Parser) -> ParseResult<String> {
    parse_optional_alias(parser).ok_or_else(|| parser.error(Expected::Description("alias")))
}

pub fn parse_select_item(parser: &mut Parser) -> ParseResult<SelectItem> {
    let expr = crate::parser::parse::expressions::parse_expr(parser)?;
    let alias = if let Some(Token::Keyword(k)) = parser.peek() {
        if *k == Keyword::As {
            let _ = parser.next();
            match parser.next() {
                Some(Token::Identifier(alias)) => Some(alias.clone()),
                Some(Token::String(alias)) => Some(alias.clone()),
                Some(Token::Keyword(kw)) => Some(kw.as_ref().to_string()),
                _ => return parser.backtrack(Expected::Description("alias")),
            }
        } else if !crate::parser::parse::expressions::is_stop_keyword(k.as_sql()) {
            match parser.next() {
                Some(Token::Identifier(id)) => Some(id.clone()),
                Some(Token::Keyword(kw)) => Some(kw.as_ref().to_string()),
                Some(_) | None => None,
            }
        } else {
            None
        }
    } else if let Some(Token::Identifier(alias)) = parser.peek() {
        // Unquoted SQL keywords are lexed as Token::Keyword, so any Token::Identifier whose
        // text matches a SQL keyword (e.g. [Group], [Order]) was explicitly delimited and is a
        // valid column alias. Still guard contextual OPTION(...) query hints.
        if !(alias.eq_ignore_ascii_case("OPTION")
            && matches!(parser.peek_at(1), Some(Token::LParen)))
        {
            let alias = alias.clone();
            let _ = parser.next();
            Some(alias)
        } else {
            None
        }
    } else if let Some(Token::String(alias)) = parser.peek() {
        let alias = alias.clone();
        let _ = parser.next();
        Some(alias)
    } else {
        None
    };
    Ok(SelectItem { expr, alias })
}

pub fn parse_order_by_expr(parser: &mut Parser) -> ParseResult<OrderByExpr> {
    let expr = crate::parser::parse::expressions::parse_expr(parser)?;
    let mut asc = true;
    if let Some(Token::Keyword(Keyword::Desc)) = parser.peek() {
        let _ = parser.next();
        asc = false;
    } else if let Some(Token::Keyword(Keyword::Asc)) = parser.peek() {
        let _ = parser.next();
        asc = true;
    }
    Ok(OrderByExpr { expr, asc })
}

pub fn parse_multipart_name(parser: &mut Parser) -> ParseResult<Vec<String>> {
    let mut parts = Vec::new();
    // Optional leading dot (e.g. `.[dbo].[tbl]` meaning current database)
    if matches!(parser.peek(), Some(Token::Dot)) {
        let _ = parser.next();
    }
    if let Some(tok) = parser.next() {
        match tok {
            Token::Identifier(id) | Token::Variable(id) => parts.push(id.clone()),
            Token::Keyword(k) => parts.push(k.as_ref().to_string()),
            _ => return parser.backtrack(Expected::Description("identifier")),
        }
    } else {
        return parser.backtrack(Expected::Description("identifier"));
    }
    while matches!(parser.peek(), Some(Token::Dot)) {
        let _ = parser.next();
        // Support omitted schema in 3-part names (e.g. `db..tbl`)
        if matches!(parser.peek(), Some(Token::Dot)) {
            parts.push(String::new());
            continue;
        }
        if let Some(tok) = parser.next() {
            match tok {
                Token::Identifier(id) | Token::Variable(id) => parts.push(id.clone()),
                Token::Keyword(k) => parts.push(k.as_ref().to_string()),
                _ => return parser.backtrack(Expected::Description("identifier")),
            }
        } else {
            return parser.backtrack(Expected::Description("identifier"));
        }
    }
    Ok(parts)
}

pub fn parse_object_name(mut parts: Vec<String>) -> ObjectName {
    let non_empty = |s: String| if s.is_empty() { None } else { Some(s) };
    match parts.len() {
        0 => ObjectName {
            database: None,
            schema: None,
            name: "".to_string(),
        },
        1 => ObjectName {
            database: None,
            schema: None,
            name: parts.remove(0),
        },
        2 => {
            let name = parts.pop().unwrap_or_default();
            let schema = non_empty(parts.pop().unwrap_or_default());
            ObjectName {
                database: None,
                schema,
                name,
            }
        }
        _ => {
            let name = parts.pop().unwrap_or_default();
            let schema = non_empty(parts.pop().unwrap_or_default());
            let database = non_empty(parts.pop().unwrap_or_default());
            ObjectName {
                database,
                schema,
                name,
            }
        }
    }
}

pub(crate) fn parse_joins_and_applies(
    parser: &mut Parser,
    joins: &mut Vec<JoinClause>,
    applies: &mut Vec<ApplyClause>,
) -> ParseResult<()> {
    loop {
        if matches!(parser.peek(), Some(Token::Comma)) {
            let _ = parser.next();
            let table = parse_table_ref(parser)?;
            if applies.is_empty() {
                joins.push(JoinClause {
                    join_type: JoinType::Cross,
                    table,
                    on: None,
                });
            } else {
                applies.push(ApplyClause {
                    apply_type: ApplyType::Cross,
                    table,
                });
            }
            continue;
        }
        if let Some(join) = parse_join_clause(parser)? {
            if applies.is_empty() {
                joins.push(join);
            } else {
                // Desugar a JOIN appearing after CROSS/OUTER APPLY into a correlated APPLY
                // so left-to-right visibility of earlier APPLY aliases is preserved.
                let apply_type = match join.join_type {
                    JoinType::Left => ApplyType::Outer,
                    _ => ApplyType::Cross,
                };
                let join_alias = join
                    .table
                    .alias
                    .clone()
                    .or_else(|| join.table.name_as_object().map(|o| o.name.clone()));
                let sub = SelectStmt {
                    distinct: false,
                    top: None,
                    projection: vec![SelectItem {
                        expr: Expr::Wildcard,
                        alias: None,
                    }],
                    into_table: None,
                    from: Some(join.table),
                    joins: Vec::new(),
                    applies: Vec::new(),
                    selection: join.on,
                    group_by: Vec::new(),
                    having: None,
                    order_by: Vec::new(),
                    offset: None,
                    fetch: None,
                    set_op: None,
                    extra: None,
                };
                applies.push(ApplyClause {
                    apply_type,
                    table: TableRef {
                        factor: TableFactor::Derived(Box::new(sub)),
                        alias: join_alias,
                        pivot: None,
                        unpivot: None,
                        hints: Vec::new(),
                    },
                });
            }
            continue;
        }
        if let Some(Token::Keyword(k)) = parser.peek() {
            let apply_type = match *k {
                Keyword::Cross => {
                    let saved = parser.save();
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Apply))) {
                        let _ = parser.next();
                        Some(ApplyType::Cross)
                    } else {
                        parser.restore(saved);
                        None
                    }
                }
                Keyword::Outer => {
                    let saved = parser.save();
                    let _ = parser.next();
                    if matches!(parser.peek(), Some(Token::Keyword(Keyword::Apply))) {
                        let _ = parser.next();
                        Some(ApplyType::Outer)
                    } else {
                        parser.restore(saved);
                        None
                    }
                }
                _ => None,
            };
            if let Some(apply_type) = apply_type {
                let table = parse_table_ref(parser)?;
                applies.push(ApplyClause { apply_type, table });
                continue;
            }
        }
        break;
    }
    Ok(())
}
