use crate::parser::ast::*;
use crate::parser::error::{Expected, ParseResult};
use crate::parser::state::Parser;
use crate::parser::token::Keyword;

fn consume_optional_single_int_param(parser: &mut Parser) -> ParseResult<()> {
    if matches!(parser.peek(), Some(Token::LParen)) {
        let _ = parser.next();
        match parser.next() {
            Some(Token::Number { .. }) => {}
            _ => return parser.backtrack(Expected::Description("number")),
        }
        parser.expect_rparen()?;
    }
    Ok(())
}

pub fn parse_data_type(parser: &mut Parser) -> ParseResult<DataType> {
    match parser.next() {
        Some(Token::Identifier(id)) => {
            let upper = id.to_uppercase();
            match upper.as_str() {
                "INT" | "INTEGER" => Ok(DataType::Int),
                "BIGINT" => Ok(DataType::BigInt),
                "SMALLINT" => Ok(DataType::SmallInt),
                "TINYINT" => Ok(DataType::TinyInt),
                "BIT" => Ok(DataType::Bit),
                "FLOAT" => {
                    consume_optional_single_int_param(parser)?;
                    Ok(DataType::Float)
                }
                "REAL" => Ok(DataType::Real),
                "DECIMAL" | "DEC" | "NUMERIC" => {
                    let mut p = 18;
                    let mut s = 0;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: val, .. }) = parser.next() {
                            p = *val as u8;
                        }
                        if matches!(parser.peek(), Some(Token::Comma)) {
                            let _ = parser.next();
                            if let Some(Token::Number { value: val, .. }) = parser.next() {
                                s = *val as u8;
                            }
                        }
                        parser.expect_rparen()?;
                    }
                    if upper == "DECIMAL" || upper == "DEC" {
                        Ok(DataType::Decimal(p, s))
                    } else {
                        Ok(DataType::Numeric(p, s))
                    }
                }
                "CHARACTER" | "CHAR" => {
                    let is_char = upper == "CHAR";
                    let mut is_varying = false;
                    let mut size = None;
                    if parser.at_keyword(Keyword::Varying) {
                        let _ = parser.next();
                        is_varying = true;
                    }
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        } else if parser.at_keyword(Keyword::Max) {
                            let _ = parser.next();
                            size = None;
                        }
                        parser.expect_rparen()?;
                    }
                    if !is_varying && parser.at_keyword(Keyword::Varying) {
                        let _ = parser.next();
                        is_varying = true;
                    }
                    if is_varying {
                        Ok(DataType::VarChar(size))
                    } else if is_char {
                        Ok(DataType::Char(size))
                    } else {
                        Ok(DataType::Char(size))
                    }
                }
                "VARCHAR" => {
                    let mut size = None;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        } else if parser.at_keyword(Keyword::Max) {
                            let _ = parser.next();
                            size = None;
                        }
                        parser.expect_rparen()?;
                    }
                    Ok(DataType::VarChar(size))
                }
                "DOUBLE" => {
                    parser.expect_keyword(Keyword::Precision)?;
                    Ok(DataType::Float)
                }
                "NATIONAL" => {
                    if parser.at_keyword(Keyword::Character) {
                        let _ = parser.next();
                    } else if parser.at_keyword(Keyword::Char) {
                        let _ = parser.next();
                    } else if parser.at_keyword(Keyword::Varchar) {
                        let _ = parser.next();
                        let mut size = None;
                        if matches!(parser.peek(), Some(Token::LParen)) {
                            let _ = parser.next();
                            if let Some(Token::Number { value: s, .. }) = parser.next() {
                                size = Some(*s as u32);
                            }
                            parser.expect_rparen()?;
                        }
                        return Ok(DataType::NationalVarChar(size));
                    } else {
                        return parser.backtrack(Expected::Description("CHARACTER or VARCHAR"));
                    }
                    let mut is_varying = false;
                    if parser.at_keyword(Keyword::Varying) {
                        let _ = parser.next();
                        is_varying = true;
                    }
                    let mut size = None;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        }
                        parser.expect_rparen()?;
                    }
                    if is_varying {
                        Ok(DataType::NationalVarChar(size))
                    } else {
                        Ok(DataType::NationalChar(size))
                    }
                }
                "NCHAR" => {
                    let mut size = None;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        } else if parser.at_keyword(Keyword::Max) {
                            let _ = parser.next();
                            size = None;
                        }
                        parser.expect_rparen()?;
                    }
                    Ok(DataType::NationalChar(size))
                }
                "NVARCHAR" => {
                    let mut size = None;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        } else if parser.at_keyword(Keyword::Max) {
                            let _ = parser.next();
                            size = None;
                        }
                        parser.expect_rparen()?;
                    }
                    Ok(DataType::NationalVarChar(size))
                }
                "BINARY" => {
                    let mut size = None;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        }
                        parser.expect_rparen()?;
                    }
                    Ok(DataType::Binary(size))
                }
                "VARBINARY" => {
                    let mut size = None;
                    if matches!(parser.peek(), Some(Token::LParen)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: s, .. }) = parser.next() {
                            size = Some(*s as u32);
                        }
                        parser.expect_rparen()?;
                    }
                    Ok(DataType::VarBinary(size))
                }
                "VECTOR" => {
                    parser.expect_lparen()?;
                    let dimensions = if let Some(Token::Number { value: d, .. }) = parser.next() {
                        if *d < 1.0
                            || *d > crate::types::VECTOR_MAX_DIMENSIONS as f64
                            || d.fract() != 0.0
                        {
                            return parser.backtrack(Expected::Description(
                                "VECTOR dimension count between 1 and 1998",
                            ));
                        }
                        *d as u16
                    } else {
                        return parser.backtrack(Expected::Description("number"));
                    };
                    if dimensions == 0 || dimensions > crate::types::VECTOR_MAX_DIMENSIONS {
                        return parser.backtrack(Expected::Description(
                            "VECTOR dimension count between 1 and 1998",
                        ));
                    }
                    if matches!(parser.peek(), Some(Token::Comma)) {
                        let _ = parser.next();
                        let base_type = match parser.next() {
                            Some(Token::Identifier(id)) => id.to_uppercase(),
                            Some(Token::Keyword(k)) => k.as_ref().to_uppercase(),
                            _ => return parser.backtrack(Expected::Description("identifier")),
                        };
                        if base_type != "FLOAT32" {
                            return parser.backtrack(Expected::Description(
                                "VECTOR currently only supports FLOAT32",
                            ));
                        }
                    }
                    parser.expect_rparen()?;
                    Ok(DataType::Vector(dimensions))
                }
                "MONEY" => Ok(DataType::Money),
                "SMALLMONEY" => Ok(DataType::SmallMoney),
                "UNIQUEIDENTIFIER" => Ok(DataType::UniqueIdentifier),
                "SYSNAME" => Ok(DataType::NVarChar(Some(128))),
                "DATE" => Ok(DataType::Date),
                "DATETIME" => Ok(DataType::DateTime),
                "SMALLDATETIME" => Ok(DataType::SmallDateTime),
                "DATETIME2" => {
                    consume_optional_single_int_param(parser)?;
                    Ok(DataType::DateTime2)
                }
                "DATETIMEOFFSET" => {
                    consume_optional_single_int_param(parser)?;
                    Ok(DataType::DateTimeOffset)
                }
                "TIME" => {
                    consume_optional_single_int_param(parser)?;
                    Ok(DataType::Time)
                }
                "XML" => Ok(DataType::Xml),
                "TEXT" => Ok(DataType::Text),
                "NTEXT" => Ok(DataType::NText),
                "IMAGE" => Ok(DataType::Image),
                "SQL_VARIANT" => Ok(DataType::SqlVariant),
                _ => {
                    let mut parts = vec![id.clone()];
                    while matches!(parser.peek(), Some(Token::Dot)) {
                        let _ = parser.next();
                        match parser.next() {
                            Some(Token::Identifier(next_id)) => parts.push(next_id.clone()),
                            Some(Token::Keyword(k)) => parts.push(k.as_ref().to_string()),
                            _ => return parser.backtrack(Expected::Description("identifier")),
                        }
                    }
                    Ok(DataType::Custom(
                        parts
                            .iter()
                            .map(|p| p.as_ref())
                            .collect::<Vec<_>>()
                            .join("."),
                    ))
                }
            }
        }
        Some(Token::Keyword(kw)) => match *kw {
            Keyword::Int => Ok(DataType::Int),
            Keyword::BigInt => Ok(DataType::BigInt),
            Keyword::SmallInt => Ok(DataType::SmallInt),
            Keyword::TinyInt => Ok(DataType::TinyInt),
            Keyword::Bit => Ok(DataType::Bit),
            Keyword::Float => {
                consume_optional_single_int_param(parser)?;
                Ok(DataType::Float)
            }
            Keyword::Real => Ok(DataType::Real),
            Keyword::Decimal | Keyword::Numeric => {
                let is_decimal = matches!(kw, Keyword::Decimal);
                let mut p = 18;
                let mut s = 0;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: val, .. }) = parser.next() {
                        p = *val as u8;
                    }
                    if matches!(parser.peek(), Some(Token::Comma)) {
                        let _ = parser.next();
                        if let Some(Token::Number { value: val, .. }) = parser.next() {
                            s = *val as u8;
                        }
                    }
                    parser.expect_rparen()?;
                }
                if is_decimal {
                    Ok(DataType::Decimal(p, s))
                } else {
                    Ok(DataType::Numeric(p, s))
                }
            }
            Keyword::Char | Keyword::Character => {
                let mut is_varying = false;
                let mut size = None;
                if parser.at_keyword(Keyword::Varying) {
                    let _ = parser.next();
                    is_varying = true;
                }
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: s, .. }) = parser.next() {
                        size = Some(*s as u32);
                    } else if parser.at_keyword(Keyword::Max) {
                        let _ = parser.next();
                        size = None;
                    }
                    parser.expect_rparen()?;
                }
                if !is_varying && parser.at_keyword(Keyword::Varying) {
                    let _ = parser.next();
                    is_varying = true;
                }
                if is_varying {
                    Ok(DataType::VarChar(size))
                } else {
                    Ok(DataType::Char(size))
                }
            }
            Keyword::Varchar => {
                let mut size = None;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: s, .. }) = parser.next() {
                        size = Some(*s as u32);
                    } else if parser.at_keyword(Keyword::Max) {
                        let _ = parser.next();
                        size = None;
                    }
                    parser.expect_rparen()?;
                }
                Ok(DataType::VarChar(size))
            }
            Keyword::NChar | Keyword::National => {
                let mut is_varying = false;
                if *kw == Keyword::National {
                    if parser.at_keyword(Keyword::Character) {
                        let _ = parser.next();
                    } else if parser.at_keyword(Keyword::Char) {
                        let _ = parser.next();
                    } else if parser.at_keyword(Keyword::Varchar) {
                        let _ = parser.next();
                        is_varying = true;
                    } else {
                        return parser.backtrack(Expected::Description("CHARACTER or VARCHAR"));
                    }
                }
                if parser.at_keyword(Keyword::Varying) {
                    let _ = parser.next();
                    is_varying = true;
                }
                let mut size = None;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: s, .. }) = parser.next() {
                        size = Some(*s as u32);
                    } else if parser.at_keyword(Keyword::Max) {
                        let _ = parser.next();
                        size = None;
                    }
                    parser.expect_rparen()?;
                }
                if !is_varying && parser.at_keyword(Keyword::Varying) {
                    let _ = parser.next();
                    is_varying = true;
                }
                if is_varying {
                    Ok(DataType::NationalVarChar(size))
                } else {
                    Ok(DataType::NationalChar(size))
                }
            }
            Keyword::Nvarchar => {
                let mut size = None;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: s, .. }) = parser.next() {
                        size = Some(*s as u32);
                    } else if parser.at_keyword(Keyword::Max) {
                        let _ = parser.next();
                        size = None;
                    }
                    parser.expect_rparen()?;
                }
                Ok(DataType::NationalVarChar(size))
            }
            Keyword::Binary => {
                let mut size = None;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: s, .. }) = parser.next() {
                        size = Some(*s as u32);
                    }
                    parser.expect_rparen()?;
                }
                Ok(DataType::Binary(size))
            }
            Keyword::Varbinary => {
                let mut size = None;
                if matches!(parser.peek(), Some(Token::LParen)) {
                    let _ = parser.next();
                    if let Some(Token::Number { value: s, .. }) = parser.next() {
                        size = Some(*s as u32);
                    }
                    parser.expect_rparen()?;
                }
                Ok(DataType::VarBinary(size))
            }
            Keyword::Money => Ok(DataType::Money),
            Keyword::SmallMoney => Ok(DataType::SmallMoney),
            Keyword::UniqueIdentifier => Ok(DataType::UniqueIdentifier),
            Keyword::SysName => Ok(DataType::NVarChar(Some(128))),
            Keyword::Date => Ok(DataType::Date),
            Keyword::DateTime => Ok(DataType::DateTime),
            Keyword::SmallDateTime => Ok(DataType::SmallDateTime),
            Keyword::DateTime2 => {
                consume_optional_single_int_param(parser)?;
                Ok(DataType::DateTime2)
            }
            Keyword::DateTimeOffset => {
                consume_optional_single_int_param(parser)?;
                Ok(DataType::DateTimeOffset)
            }
            Keyword::Time => {
                consume_optional_single_int_param(parser)?;
                Ok(DataType::Time)
            }
            Keyword::Xml => Ok(DataType::Xml),
            Keyword::Text => Ok(DataType::Text),
            Keyword::NText => Ok(DataType::NText),
            Keyword::Image => Ok(DataType::Image),
            Keyword::SqlVariant | Keyword::Cursor => Ok(DataType::SqlVariant),
            _ => Ok(DataType::Custom(kw.as_ref().to_string())),
        },
        _ => parser.backtrack(Expected::Description("data type")),
    }
}
