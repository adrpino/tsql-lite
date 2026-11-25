use std::cmp::Ordering;

use crate::ast::Expr;
use crate::types::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CollationFlags {
    pub case_sensitive: bool,
    pub accent_sensitive: bool,
}

impl Default for CollationFlags {
    fn default() -> Self {
        // SQL Server default collation (SQL_Latin1_General_CP1_CI_AS): case-insensitive, accent-sensitive.
        Self {
            case_sensitive: false,
            accent_sensitive: true,
        }
    }
}

impl CollationFlags {
    pub(crate) fn parse_collation_name(name: &str) -> Self {
        let upper = name.to_ascii_uppercase();
        if upper.contains("_BIN") || upper.ends_with("_BIN2") {
            return Self {
                case_sensitive: true,
                accent_sensitive: true,
            };
        }
        let case_sensitive = upper.contains("_CS_") || upper.ends_with("_CS");
        let accent_sensitive = !(upper.contains("_AI_") || upper.ends_with("_AI"));
        Self {
            case_sensitive,
            accent_sensitive,
        }
    }
}

pub(crate) fn expr_collation(expr: &Expr) -> Option<CollationFlags> {
    if let Expr::FunctionCall { name, args, .. } = expr {
        if name == "__COLLATE" {
            if let Some(Expr::String(col)) = args.get(1) {
                return Some(CollationFlags::parse_collation_name(col));
            }
        }
    }
    None
}

fn is_combining_diacritic(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}')
}

/// Strips Latin diacritical marks while preserving letter case.
pub(crate) fn strip_latin_accent(c: char) -> char {
    match c {
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => 'A',
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
        'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => 'C',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'Ď' | 'Đ' => 'D',
        'ď' | 'đ' => 'd',
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => 'E',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => 'G',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'Ĥ' | 'Ħ' => 'H',
        'ĥ' | 'ħ' => 'h',
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => 'I',
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'Ĵ' => 'J',
        'ĵ' => 'j',
        'Ķ' => 'K',
        'ķ' => 'k',
        'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => 'L',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'Ñ' | 'Ń' | 'Ņ' | 'Ň' => 'N',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => 'O',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'Ŕ' | 'Ŗ' | 'Ř' => 'R',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'Ś' | 'Ŝ' | 'Ş' | 'Š' => 'S',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'Ţ' | 'Ť' | 'Ŧ' => 'T',
        'ţ' | 'ť' | 'ŧ' => 't',
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => 'U',
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'Ŵ' => 'W',
        'ŵ' => 'w',
        'Ý' | 'Ÿ' | 'Ŷ' => 'Y',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'Ź' | 'Ż' | 'Ž' => 'Z',
        'ź' | 'ż' | 'ž' => 'z',
        other => other,
    }
}

pub(crate) fn fold_str(s: &str, flags: CollationFlags, trim_trailing_spaces: bool) -> String {
    let base = if trim_trailing_spaces {
        s.trim_end_matches(' ')
    } else {
        s
    };
    if base.is_ascii() {
        return if flags.case_sensitive {
            base.to_string()
        } else {
            base.to_ascii_lowercase()
        };
    }
    let mut out = String::with_capacity(base.len());
    for ch in base.chars() {
        if !flags.accent_sensitive && is_combining_diacritic(ch) {
            continue;
        }
        let c = if flags.accent_sensitive {
            ch
        } else {
            strip_latin_accent(ch)
        };
        if flags.case_sensitive {
            out.push(c);
        } else {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
        }
    }
    out
}

fn value_as_str(v: &Value) -> Option<&str> {
    match v {
        Value::Char(s) | Value::VarChar(s) | Value::NChar(s) | Value::NVarChar(s) => Some(s),
        _ => None,
    }
}

pub(crate) fn compare_values_with_collation(
    a: &Value,
    b: &Value,
    flags: CollationFlags,
) -> Ordering {
    if let (Some(ls), Some(rs)) = (value_as_str(a), value_as_str(b)) {
        if flags.case_sensitive && flags.accent_sensitive {
            return ls.trim_end_matches(' ').cmp(rs.trim_end_matches(' '));
        }
        let lf = fold_str(ls, flags, true);
        let rf = fold_str(rs, flags, true);
        return lf.cmp(&rf);
    }
    crate::executor::value_ops::compare_values(a, b)
}
