use crate::ast::Expr;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::types::DataType;

pub(crate) fn xml_conversion_not_allowed(from_type: &str) -> DbError {
    DbError::Custom {
        class: 16,
        number: 529,
        message: format!(
            "Explicit conversion from data type {} to xml is not allowed.",
            from_type
        ),
    }
}

pub(crate) fn implicit_xml_conversion_not_allowed(to_type: &str) -> DbError {
    DbError::Custom {
        class: 16,
        number: 257,
        message: format!(
            "Implicit conversion from data type xml to {} is not allowed. Use the CONVERT function to run this query.",
            to_type
        ),
    }
}

fn data_type_sql_name(ty: &DataType) -> &'static str {
    match ty {
        DataType::Bit => "bit",
        DataType::TinyInt => "tinyint",
        DataType::SmallInt => "smallint",
        DataType::Int => "int",
        DataType::BigInt => "bigint",
        DataType::Float => "float",
        DataType::Decimal { .. } => "decimal",
        DataType::Money => "money",
        DataType::SmallMoney => "smallmoney",
        DataType::Char { .. } => "char",
        DataType::VarChar { .. } => "varchar",
        DataType::NChar { .. } => "nchar",
        DataType::NVarChar { .. } => "nvarchar",
        DataType::Binary { .. } => "binary",
        DataType::VarBinary { .. } => "varbinary",
        DataType::Date => "date",
        DataType::Time => "time",
        DataType::DateTime => "datetime",
        DataType::DateTime2 => "datetime2",
        DataType::SmallDateTime => "smalldatetime",
        DataType::DateTimeOffset => "datetimeoffset",
        DataType::UniqueIdentifier => "uniqueidentifier",
        DataType::SqlVariant => "sql_variant",
        DataType::Xml => "xml",
        DataType::Vector { .. } => "vector",
    }
}

/// Rejects direct implicit conversion from an `XML`-typed variable to a non-`XML` target type
/// (SQL Server error 257). Explicit `CAST(@x AS ...)` / `CONVERT(..., @x)` are unaffected.
pub(crate) fn check_implicit_xml_var_conversion(
    expr: &Expr,
    target_ty: &DataType,
    ctx: &ExecutionContext<'_>,
) -> Result<(), DbError> {
    if matches!(target_ty, DataType::Xml | DataType::SqlVariant) {
        return Ok(());
    }
    if let Expr::Identifier(name) = expr {
        if name.starts_with('@') {
            if let Some((DataType::Xml, _)) = ctx.session.get_var(name) {
                return Err(implicit_xml_conversion_not_allowed(data_type_sql_name(
                    target_ty,
                )));
            }
        }
    }
    Ok(())
}

fn xml_parse_err(reason: impl Into<String>) -> DbError {
    DbError::Custom {
        class: 16,
        number: 9400,
        message: format!("XML parsing: {}", reason.into()),
    }
}

fn is_name_start_char(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == ':'
}

fn is_name_char(c: char) -> bool {
    is_name_start_char(c) || c.is_ascii_digit() || c == '-' || c == '.'
}

fn validate_entity_ref(rest: &str) -> Result<usize, DbError> {
    // `rest` starts right after '&'.
    let Some(semi) = rest.find(';') else {
        return Err(xml_parse_err("semicolon expected in entity reference"));
    };
    let entity = &rest[..semi];
    if entity.is_empty() {
        return Err(xml_parse_err("empty entity reference"));
    }
    if let Some(num) = entity.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            if hex.is_empty() {
                return Err(xml_parse_err("invalid hexadecimal character reference"));
            }
            u32::from_str_radix(hex, 16)
                .map_err(|_| xml_parse_err("invalid hexadecimal character reference"))?
        } else {
            if num.is_empty() {
                return Err(xml_parse_err("invalid decimal character reference"));
            }
            num.parse::<u32>()
                .map_err(|_| xml_parse_err("invalid decimal character reference"))?
        };
        // XML 1.0 valid Char range (NUL and surrogates disallowed).
        let valid = matches!(
            code,
            0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
        );
        if !valid {
            return Err(xml_parse_err("illegal xml character"));
        }
        return Ok(semi + 1);
    }
    match entity {
        "amp" | "lt" | "gt" | "quot" | "apos" => Ok(semi + 1),
        _ => Err(xml_parse_err(format!(
            "reference to undefined entity '{}'",
            entity
        ))),
    }
}

fn scan_name(s: &str, start: usize) -> Option<usize> {
    let mut chars = s[start..].char_indices();
    let (_, first) = chars.next()?;
    if !is_name_start_char(first) {
        return None;
    }
    let mut end = start + first.len_utf8();
    for (rel, ch) in chars {
        if is_name_char(ch) {
            end = start + rel + ch.len_utf8();
        } else {
            break;
        }
    }
    Some(end)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// Validates an XML 1.0 document or content fragment following SQL Server `XML` data type rules.
///
/// SQL Server allows empty strings, plain text fragments, and multi-root fragments (`<a/><b/>`),
/// while rejecting unclosed/mismatched tags, unescaped `<`/`&`, malformed/duplicate attributes,
/// and unterminated comments, CDATA sections, or processing instructions.
pub(crate) fn validate_xml_fragment(s: &str) -> Result<(), DbError> {
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut stack: Vec<&str> = Vec::new();

    while i < len {
        match bytes[i] {
            b'&' => {
                let consumed = validate_entity_ref(&s[i + 1..])?;
                i += 1 + consumed;
            }
            b']' if s[i..].starts_with("]]>") => {
                return Err(xml_parse_err("']]>' is not allowed in character data"));
            }
            b'<' => {
                if s[i..].starts_with("<!--") {
                    let body_start = i + 4;
                    let Some(rel_end) = s[body_start..].find("-->") else {
                        return Err(xml_parse_err("unexpected end of input in comment"));
                    };
                    let body = &s[body_start..body_start + rel_end];
                    if body.contains("--") || body.ends_with('-') {
                        return Err(xml_parse_err("'--' is not allowed inside XML comment"));
                    }
                    i = body_start + rel_end + 3;
                } else if s[i..].starts_with("<![CDATA[") {
                    let body_start = i + 9;
                    let Some(rel_end) = s[body_start..].find("]]>") else {
                        return Err(xml_parse_err("unexpected end of input in CDATA section"));
                    };
                    i = body_start + rel_end + 3;
                } else if s[i..].starts_with("<?") {
                    let name_start = i + 2;
                    let Some(name_end) = scan_name(s, name_start) else {
                        return Err(xml_parse_err("invalid processing instruction target"));
                    };
                    let Some(rel_end) = s[name_end..].find("?>") else {
                        return Err(xml_parse_err(
                            "unexpected end of input in processing instruction",
                        ));
                    };
                    i = name_end + rel_end + 2;
                } else if s[i..].starts_with("<!") {
                    // DOCTYPE or markup declaration — reject if unclosed.
                    let Some(rel_end) = s[i + 2..].find('>') else {
                        return Err(xml_parse_err("unexpected end of input in declaration"));
                    };
                    i = i + 2 + rel_end + 1;
                } else if i + 1 < len && bytes[i + 1] == b'/' {
                    // Closing tag </name>
                    let name_start = i + 2;
                    let Some(name_end) = scan_name(s, name_start) else {
                        return Err(xml_parse_err("expected element name in end tag"));
                    };
                    let tag_name = &s[name_start..name_end];
                    let j = skip_ws(bytes, name_end);
                    if j >= len || bytes[j] != b'>' {
                        return Err(xml_parse_err("expected '>' in end tag"));
                    }
                    match stack.pop() {
                        Some(open) if open == tag_name => {}
                        Some(open) => {
                            return Err(xml_parse_err(format!(
                                "end tag '{}' does not match start tag '{}'",
                                tag_name, open
                            )));
                        }
                        None => {
                            return Err(xml_parse_err(format!(
                                "unexpected end tag '{}'",
                                tag_name
                            )));
                        }
                    }
                    i = j + 1;
                } else {
                    // Opening or self-closing tag <name attr="val" ...>
                    let name_start = i + 1;
                    let Some(name_end) = scan_name(s, name_start) else {
                        return Err(xml_parse_err("illegal name character"));
                    };
                    let tag_name = &s[name_start..name_end];
                    let mut j = name_end;
                    let mut seen_attrs: Vec<&str> = Vec::new();
                    let mut self_closing = false;

                    loop {
                        let before_ws = j;
                        j = skip_ws(bytes, j);
                        if j >= len {
                            return Err(xml_parse_err("unexpected end of input in start tag"));
                        }
                        if bytes[j] == b'>' {
                            j += 1;
                            break;
                        }
                        if bytes[j] == b'/' {
                            if j + 1 < len && bytes[j + 1] == b'>' {
                                self_closing = true;
                                j += 2;
                                break;
                            }
                            return Err(xml_parse_err("expected '>' after '/'"));
                        }
                        if j == before_ws {
                            return Err(xml_parse_err("whitespace expected before attribute"));
                        }
                        let Some(attr_end) = scan_name(s, j) else {
                            return Err(xml_parse_err("invalid attribute name"));
                        };
                        let attr_name = &s[j..attr_end];
                        if seen_attrs.contains(&attr_name) {
                            return Err(xml_parse_err(format!(
                                "duplicate attribute '{}'",
                                attr_name
                            )));
                        }
                        seen_attrs.push(attr_name);
                        j = skip_ws(bytes, attr_end);
                        if j >= len || bytes[j] != b'=' {
                            return Err(xml_parse_err("expected '=' after attribute name"));
                        }
                        j = skip_ws(bytes, j + 1);
                        if j >= len || (bytes[j] != b'"' && bytes[j] != b'\'') {
                            return Err(xml_parse_err(
                                "expected quoted string for attribute value",
                            ));
                        }
                        let quote = bytes[j];
                        j += 1;
                        let mut closed = false;
                        while j < len {
                            let b = bytes[j];
                            if b == quote {
                                j += 1;
                                closed = true;
                                break;
                            }
                            if b == b'<' {
                                return Err(xml_parse_err("'<' is not allowed in attribute value"));
                            }
                            if b == b'&' {
                                let consumed = validate_entity_ref(&s[j + 1..])?;
                                j += 1 + consumed;
                            } else {
                                j += 1;
                            }
                        }
                        if !closed {
                            return Err(xml_parse_err(
                                "unexpected end of input in attribute value",
                            ));
                        }
                    }

                    if !self_closing {
                        stack.push(tag_name);
                    }
                    i = j;
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    if let Some(unclosed) = stack.last() {
        return Err(xml_parse_err(format!(
            "unexpected end of input, unclosed element '{}'",
            unclosed
        )));
    }
    Ok(())
}
