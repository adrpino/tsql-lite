use std::sync::Arc;

use crate::ast::TableRef;
use crate::catalog::{Catalog, ColumnDef, TableDef};
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;
use crate::executor::model::BoundTable;
use crate::executor::{type_mapping, value_ops};
use crate::parser::parse_expr_subquery_aware;
use crate::storage::{Storage, StoredRow};
use crate::types::{DataType, Value};

#[derive(Debug, Clone)]
struct XmlNode {
    tag: String,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<XmlNode>,
}

fn unescape_xml(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

struct XmlCursor<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> XmlCursor<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.src[self.pos..]
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.rest().as_bytes().first() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_prolog_and_comments(&mut self) {
        loop {
            self.skip_ws();
            let r = self.rest();
            if r.starts_with("<?") {
                if let Some(end) = r.find("?>") {
                    self.pos += end + 2;
                    continue;
                }
            } else if r.starts_with("<!--") {
                if let Some(end) = r.find("-->") {
                    self.pos += end + 3;
                    continue;
                }
            } else if r.starts_with("<!DOCTYPE") {
                if let Some(end) = r.find('>') {
                    self.pos += end + 1;
                    continue;
                }
            }
            break;
        }
    }

    fn read_name(&mut self) -> &'a str {
        let start = self.pos;
        for (i, ch) in self.rest().char_indices() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.') {
                self.pos = start + i + ch.len_utf8();
            } else {
                break;
            }
        }
        &self.src[start..self.pos]
    }

    fn parse_element(&mut self) -> Result<XmlNode, DbError> {
        self.skip_prolog_and_comments();
        if !self.rest().starts_with('<') {
            return Err(DbError::Execution("OPENXML: malformed XML document".into()));
        }
        self.pos += 1; // '<'
        let tag = self.read_name().to_string();
        if tag.is_empty() {
            return Err(DbError::Execution("OPENXML: empty XML tag name".into()));
        }
        let mut attrs = Vec::new();
        loop {
            self.skip_ws();
            let r = self.rest();
            if r.starts_with("/>") {
                self.pos += 2;
                return Ok(XmlNode {
                    tag,
                    attrs,
                    text: String::new(),
                    children: Vec::new(),
                });
            }
            if r.starts_with('>') {
                self.pos += 1;
                break;
            }
            if r.is_empty() {
                return Err(DbError::Execution("OPENXML: unclosed start tag".into()));
            }
            let attr_name = self.read_name().to_string();
            if attr_name.is_empty() {
                return Err(DbError::Execution("OPENXML: invalid XML attribute".into()));
            }
            self.skip_ws();
            if !self.rest().starts_with('=') {
                return Err(DbError::Execution(
                    "OPENXML: expected '=' after attribute name".into(),
                ));
            }
            self.pos += 1;
            self.skip_ws();
            let quote = match self.rest().chars().next() {
                Some(q @ ('"' | '\'')) => {
                    self.pos += 1;
                    q
                }
                _ => {
                    return Err(DbError::Execution(
                        "OPENXML: expected quoted attribute value".into(),
                    ))
                }
            };
            let val_start = self.pos;
            let Some(rel_end) = self.rest().find(quote) else {
                return Err(DbError::Execution(
                    "OPENXML: unterminated attribute value".into(),
                ));
            };
            let raw_val = &self.src[val_start..val_start + rel_end];
            self.pos = val_start + rel_end + 1;
            attrs.push((attr_name, unescape_xml(raw_val)));
        }

        let mut text = String::new();
        let mut children = Vec::new();
        loop {
            let r = self.rest();
            if r.is_empty() {
                return Err(DbError::Execution(format!(
                    "OPENXML: missing closing tag </{}>",
                    tag
                )));
            }
            if r.starts_with("</") {
                self.pos += 2;
                let close_tag = self.read_name();
                self.skip_ws();
                if self.rest().starts_with('>') {
                    self.pos += 1;
                }
                if !close_tag.eq_ignore_ascii_case(&tag) {
                    return Err(DbError::Execution(format!(
                        "OPENXML: mismatched closing tag </{}> for <{}>",
                        close_tag, tag
                    )));
                }
                break;
            } else if r.starts_with("<!--") {
                if let Some(end) = r.find("-->") {
                    self.pos += end + 3;
                } else {
                    return Err(DbError::Execution("OPENXML: unclosed XML comment".into()));
                }
            } else if let Some(after) = r.strip_prefix("<![CDATA[") {
                if let Some(end) = after.find("]]>") {
                    text.push_str(&after[..end]);
                    self.pos += 9 + end + 3;
                } else {
                    return Err(DbError::Execution("OPENXML: unclosed CDATA section".into()));
                }
            } else if r.starts_with('<') {
                children.push(self.parse_element()?);
            } else if let Some(next_lt) = r.find('<') {
                text.push_str(&unescape_xml(&r[..next_lt]));
                self.pos += next_lt;
            } else {
                text.push_str(&unescape_xml(r));
                self.pos = self.src.len();
            }
        }

        Ok(XmlNode {
            tag,
            attrs,
            text,
            children,
        })
    }
}

pub(crate) fn validate_xml_document(xml: &str) -> Result<(), DbError> {
    let mut cur = XmlCursor::new(xml);
    let _ = cur.parse_element()?;
    Ok(())
}

fn node_all_text(node: &XmlNode) -> String {
    if node.children.is_empty() {
        return node.text.trim().to_string();
    }
    let mut out = node.text.clone();
    for ch in &node.children {
        out.push_str(&node_all_text(ch));
    }
    out.trim().to_string()
}

fn strip_tag_prefix(name: &str) -> &str {
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
}

fn tag_matches(node_tag: &str, seg: &str) -> bool {
    seg == "*"
        || node_tag == seg
        || strip_tag_prefix(node_tag) == seg
        || node_tag.eq_ignore_ascii_case(seg)
}

fn select_nodes_xpath<'a>(root: &'a XmlNode, xpath: &str) -> Vec<(Vec<&'a XmlNode>, &'a XmlNode)> {
    let mut results = Vec::new();
    for branch in xpath.split('|') {
        let p = branch.trim();
        if p.is_empty() {
            continue;
        }
        let mut current: Vec<(Vec<&'a XmlNode>, &'a XmlNode)> = Vec::new();
        if let Some(rest) = p.strip_prefix("//") {
            let mut segs = rest.split('/').filter(|s| !s.is_empty());
            if let Some(first) = segs.next() {
                collect_descendants(root, Vec::new(), first, &mut current);
                for seg in segs {
                    current = step_nodes(current, seg);
                }
            }
        } else {
            let trimmed = p.trim_start_matches('/');
            let mut segs = trimmed.split('/').filter(|s| !s.is_empty());
            if let Some(first) = segs.next() {
                if tag_matches(&root.tag, first) {
                    current.push((Vec::new(), root));
                }
                for seg in segs {
                    current = step_nodes(current, seg);
                }
            }
        }
        results.extend(current);
    }
    results
}

fn collect_descendants<'a>(
    node: &'a XmlNode,
    ancestors: Vec<&'a XmlNode>,
    seg: &str,
    out: &mut Vec<(Vec<&'a XmlNode>, &'a XmlNode)>,
) {
    if tag_matches(&node.tag, seg) {
        out.push((ancestors.clone(), node));
    }
    let mut next_anc = ancestors;
    next_anc.push(node);
    for child in &node.children {
        collect_descendants(child, next_anc.clone(), seg, out);
    }
}

fn step_nodes<'a>(
    current: Vec<(Vec<&'a XmlNode>, &'a XmlNode)>,
    seg: &str,
) -> Vec<(Vec<&'a XmlNode>, &'a XmlNode)> {
    let mut next = Vec::new();
    for (ancestors, node) in current {
        if seg == "." {
            next.push((ancestors, node));
        } else if seg == ".." {
            if let Some((parent, rest)) = ancestors.split_last() {
                next.push((rest.to_vec(), *parent));
            }
        } else {
            let mut next_anc = ancestors;
            next_anc.push(node);
            for child in &node.children {
                if tag_matches(&child.tag, seg) {
                    next.push((next_anc.clone(), child));
                }
            }
        }
    }
    next
}

fn extract_col_value(
    ancestors: &[&XmlNode],
    node: &XmlNode,
    col_name: &str,
    col_pattern: Option<&str>,
    flags: i64,
) -> Option<String> {
    if let Some(pat) = col_pattern {
        return eval_relative_pattern(ancestors, node, pat.trim());
    }
    let mode = flags & 3;
    let try_attr = mode == 0 || mode == 1 || mode == 3;
    let try_elem = mode == 2 || mode == 3;
    if try_attr {
        if let Some((_, val)) = node
            .attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(col_name))
        {
            return Some(val.clone());
        }
    }
    if try_elem {
        if let Some(child) = node.children.iter().find(|c| tag_matches(&c.tag, col_name)) {
            return Some(node_all_text(child));
        }
    }
    None
}

fn eval_relative_pattern(ancestors: &[&XmlNode], node: &XmlNode, pat: &str) -> Option<String> {
    if pat == "." {
        return Some(node_all_text(node));
    }
    let mut cur_anc: Vec<&XmlNode> = ancestors.to_vec();
    let mut cur = node;
    let parts: Vec<&str> = pat.split('/').filter(|s| !s.is_empty()).collect();
    for (i, seg) in parts.iter().enumerate() {
        let is_last = i + 1 == parts.len();
        if *seg == "." {
            continue;
        }
        if *seg == ".." {
            let parent = cur_anc.pop()?;
            cur = parent;
            continue;
        }
        if let Some(attr) = seg.strip_prefix('@') {
            return cur
                .attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(attr))
                .map(|(_, v)| v.clone());
        }
        let child = cur.children.iter().find(|c| tag_matches(&c.tag, seg))?;
        if is_last {
            return Some(node_all_text(child));
        }
        cur_anc.push(cur);
        cur = child;
    }
    Some(node_all_text(cur))
}

struct OpenXmlColDef {
    name: String,
    data_type: DataType,
    pattern: Option<String>,
}

fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_single = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_single {
            current.push(c);
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    current.push(chars.next().unwrap());
                } else {
                    in_single = false;
                }
            }
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                current.push(c);
            }
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ch if ch == sep && depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn parse_openxml_call_and_with(raw: &str) -> Result<(String, Option<String>), DbError> {
    let open_idx = raw
        .find('(')
        .ok_or_else(|| DbError::Parse("OPENXML requires (idoc, rowpattern [, flags])".into()))?;
    let mut depth = 0usize;
    let mut in_single = false;
    let mut close_idx = None;
    let bytes = raw.as_bytes();
    let mut i = open_idx;
    while i < bytes.len() {
        let b = bytes[i];
        if in_single {
            if b == b'\'' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    i += 2;
                    continue;
                }
                in_single = false;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' => in_single = true,
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close_idx = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close =
        close_idx.ok_or_else(|| DbError::Parse("OPENXML: missing closing parenthesis".into()))?;
    let args_str = raw[open_idx + 1..close].trim().to_string();
    let tail = raw[close + 1..].trim();
    if tail.is_empty() {
        return Ok((args_str, None));
    }
    let tail_upper = tail.to_ascii_uppercase();
    if let Some(after_with) = tail_upper.strip_prefix("WITH") {
        let rest = tail[tail.len() - after_with.len()..].trim();
        let inner = rest
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| DbError::Parse("OPENXML WITH requires parenthesized schema".into()))?;
        Ok((args_str, Some(inner.trim().to_string())))
    } else {
        Err(DbError::Parse(format!("Invalid OPENXML clause: {}", tail)))
    }
}

fn parse_openxml_with_schema(
    inner: &str,
    catalog: &dyn Catalog,
) -> Result<Vec<OpenXmlColDef>, DbError> {
    let items = split_top_level(inner, ',');
    // Single existing table name form: WITH (dbo.MyTable)
    if items.len() == 1 && !items[0].contains(' ') {
        let tbl_raw = items[0].trim();
        let (sch, name) = match tbl_raw.split_once('.') {
            Some((s, n)) => (
                s.trim().trim_matches(['[', ']']),
                n.trim().trim_matches(['[', ']']),
            ),
            None => ("dbo", tbl_raw.trim_matches(['[', ']'])),
        };
        if let Some(tbl) = catalog.find_table(sch, name) {
            return Ok(tbl
                .columns
                .iter()
                .map(|c| OpenXmlColDef {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    pattern: None,
                })
                .collect());
        }
    }

    let mut cols = Vec::with_capacity(items.len());
    for item in items {
        let mut slice = item.as_str();
        let tokens = crate::parser::lexer::lex(&mut slice, true)
            .map_err(|_| DbError::Parse("Failed to lex OPENXML WITH schema".into()))?;
        let mut p = crate::parser::state::Parser::new(tokens);
        let col_name = match p.next() {
            Some(crate::parser::ast::Token::Identifier(id)) => id.clone(),
            Some(crate::parser::ast::Token::Keyword(kw)) => kw.as_ref().to_string(),
            _ => {
                return Err(DbError::Parse(
                    "Expected column name in OPENXML WITH".into(),
                ))
            }
        };
        let parsed_ty = crate::parser::parse::expressions::parse_data_type(&mut p)
            .map_err(|e| DbError::Parse(format!("OPENXML WITH type error: {}", e)))?;
        let lowered_ty = crate::parser::lower::common::lower_data_type(parsed_ty)?;
        let data_type = type_mapping::resolve_data_type_spec(&lowered_ty, catalog);
        let pattern = match p.peek() {
            Some(crate::parser::ast::Token::String(s))
            | Some(crate::parser::ast::Token::NString(s)) => {
                let pat = s.clone();
                let _ = p.next();
                Some(pat)
            }
            _ => None,
        };
        cols.push(OpenXmlColDef {
            name: col_name,
            data_type,
            pattern,
        });
    }
    Ok(cols)
}

pub(super) fn bind_openxml(
    catalog: &dyn Catalog,
    storage: &dyn Storage,
    clock: &dyn Clock,
    tref: &TableRef,
    ctx: &mut ExecutionContext,
    raw_name: &str,
) -> Result<Option<BoundTable>, DbError> {
    let (args_str, with_inner) = parse_openxml_call_and_with(raw_name)?;
    let arg_strs = split_top_level(&args_str, ',');
    if arg_strs.len() < 2 || arg_strs.len() > 3 {
        return Err(DbError::Parse(
            "OPENXML requires 2 or 3 arguments: (idoc, rowpattern [, flags])".into(),
        ));
    }

    let hdoc_expr = parse_expr_subquery_aware(&arg_strs[0])?;
    let hdoc_val = eval_expr(&hdoc_expr, &[], ctx, catalog, storage, clock)?;
    let hdoc = hdoc_val
        .to_integer_i64()
        .ok_or_else(|| DbError::Execution("OPENXML: document handle must be an integer".into()))?
        as i32;

    let pat_expr = parse_expr_subquery_aware(&arg_strs[1])?;
    let pat_val = eval_expr(&pat_expr, &[], ctx, catalog, storage, clock)?;
    let rowpattern = pat_val.to_string_value();

    let flags = if arg_strs.len() == 3 {
        let f_expr = parse_expr_subquery_aware(&arg_strs[2])?;
        eval_expr(&f_expr, &[], ctx, catalog, storage, clock)?
            .to_integer_i64()
            .unwrap_or(1)
    } else {
        1
    };

    let xml_src = ctx
        .session
        .extras
        .xml_documents
        .get(&hdoc)
        .cloned()
        .ok_or_else(|| {
            DbError::Execution(format!(
                "Could not find prepared statement with handle {}.",
                hdoc
            ))
        })?;

    let mut cur = XmlCursor::new(&xml_src);
    let root = cur.parse_element()?;
    let matched = select_nodes_xpath(&root, &rowpattern);

    if let Some(with_sql) = with_inner {
        let col_defs = parse_openxml_with_schema(&with_sql, catalog)?;
        let columns: Vec<ColumnDef> = col_defs
            .iter()
            .enumerate()
            .map(|(i, c)| ColumnDef {
                id: (i + 1) as u32,
                name: c.name.clone(),
                data_type: c.data_type.clone(),
                nullable: true,
                primary_key: false,
                unique: false,
                identity: None,
                default: None,
                default_constraint_name: None,
                check: None,
                check_constraint_name: None,
                computed_expr: None,
                collation: None,
                is_clustered: false,
                ansi_padding_on: true,
                is_rowversion: false,
            })
            .collect();

        let mut rows = Vec::with_capacity(matched.len());
        for (ancestors, node) in matched {
            let mut vals = Vec::with_capacity(col_defs.len());
            for c in &col_defs {
                let raw = extract_col_value(&ancestors, node, &c.name, c.pattern.as_deref(), flags);
                let val = match raw {
                    None => Value::Null,
                    Some(s) => value_ops::coerce_value_to_type_with_dateformat(
                        Value::NVarChar(s),
                        &c.data_type,
                        &ctx.options.dateformat,
                    )?,
                };
                vals.push(val);
            }
            rows.push(StoredRow {
                values: vals,
                deleted: false,
            });
        }

        let table_def = TableDef {
            id: 0,
            schema_id: 1,
            schema_name: "dbo".to_string(),
            name: "OPENXML".to_string(),
            columns,
            check_constraints: vec![],
            foreign_keys: vec![],
            unique_constraints: vec![],
        };
        return Ok(Some(BoundTable {
            alias: Arc::from(tref.alias.as_deref().unwrap_or("OPENXML")),
            table: Arc::new(table_def),
            virtual_rows: Some(rows),
        }));
    }

    // Default edge-table output when WITH is omitted
    let edge_cols = vec![
        ("id", DataType::BigInt),
        ("parentid", DataType::BigInt),
        ("nodetype", DataType::Int),
        ("localname", DataType::NVarChar { max_len: 4000 }),
        ("text", DataType::NVarChar { max_len: 4000 }),
    ];
    let columns: Vec<ColumnDef> = edge_cols
        .into_iter()
        .enumerate()
        .map(|(i, (name, dt))| ColumnDef {
            id: (i + 1) as u32,
            name: name.to_string(),
            data_type: dt,
            nullable: true,
            primary_key: false,
            unique: false,
            identity: None,
            default: None,
            default_constraint_name: None,
            check: None,
            check_constraint_name: None,
            computed_expr: None,
            collation: None,
            is_clustered: false,
            ansi_padding_on: true,
            is_rowversion: false,
        })
        .collect();
    let mut rows = Vec::new();
    let mut next_id = 1i64;
    for (_, node) in matched {
        let elem_id = next_id;
        next_id += 1;
        rows.push(StoredRow {
            values: vec![
                Value::BigInt(elem_id),
                Value::Null,
                Value::Int(1),
                Value::NVarChar(node.tag.clone()),
                if node.text.trim().is_empty() {
                    Value::Null
                } else {
                    Value::NVarChar(node.text.trim().to_string())
                },
            ],
            deleted: false,
        });
        for (k, v) in &node.attrs {
            let attr_id = next_id;
            next_id += 1;
            rows.push(StoredRow {
                values: vec![
                    Value::BigInt(attr_id),
                    Value::BigInt(elem_id),
                    Value::Int(2),
                    Value::NVarChar(k.clone()),
                    Value::NVarChar(v.clone()),
                ],
                deleted: false,
            });
        }
    }
    let table_def = TableDef {
        id: 0,
        schema_id: 1,
        schema_name: "dbo".to_string(),
        name: "OPENXML".to_string(),
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
        unique_constraints: vec![],
    };
    Ok(Some(BoundTable {
        alias: Arc::from(tref.alias.as_deref().unwrap_or("OPENXML")),
        table: Arc::new(table_def),
        virtual_rows: Some(rows),
    }))
}
