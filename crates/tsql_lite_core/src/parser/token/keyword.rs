use std::fmt;
use std::hash::Hash;

macro_rules! define_keywords {
    (
        $(
            $(#[$cat:meta])*
            $variant:ident => $sql:expr
        ),* $(,)?
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Keyword {
            $( $variant, )*
        }

        impl Keyword {
            #[inline]
            pub fn parse(s: &str) -> Option<Keyword> {
                const MAX_KEYWORD_LEN: usize = 24;
                let bytes = s.as_bytes();
                if bytes.len() > MAX_KEYWORD_LEN {
                    return None;
                }
                let mut buf = [0u8; MAX_KEYWORD_LEN];
                for (dst, &b) in buf.iter_mut().zip(bytes) {
                    if !b.is_ascii() {
                        return None;
                    }
                    *dst = b.to_ascii_uppercase();
                }
                let upper = std::str::from_utf8(&buf[..bytes.len()]).ok()?;
                match upper {
                    $( $sql => Some(Keyword::$variant), )*
                    _ => None,
                }
            }

            pub fn as_sql(&self) -> &'static str {
                match self {
                    $( Keyword::$variant => $sql, )*
                }
            }

            #[cfg(test)]
            pub const ALL: &'static [Keyword] = &[
                $( Keyword::$variant, )*
            ];
        }

        impl fmt::Display for Keyword {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_sql())
            }
        }

        impl AsRef<str> for Keyword {
            fn as_ref(&self) -> &str {
                self.as_sql()
            }
        }
    };
}

define_keywords! {
    // DML
    Select => "SELECT",
    From => "FROM",
    Where => "WHERE",
    Group => "GROUP",
    By => "BY",
    Having => "HAVING",
    Order => "ORDER",
    Join => "JOIN",
    On => "ON",
    As => "AS",
    In => "IN",
    Is => "IS",
    Null => "NULL",
    And => "AND",
    Or => "OR",
    Not => "NOT",
    Case => "CASE",
    When => "WHEN",
    Then => "THEN",
    Else => "ELSE",
    End => "END",
    Cast => "CAST",
    Convert => "CONVERT",
    Like => "LIKE",
    Escape => "ESCAPE",
    Top => "TOP",
    Distinct => "DISTINCT",
    Insert => "INSERT",
    Update => "UPDATE",
    Delete => "DELETE",
    Into => "INTO",
    Values => "VALUES",
    Exists => "EXISTS",
    Truncate => "TRUNCATE",
    Merge => "MERGE",
    Using => "USING",
    Matched => "MATCHED",
    Source => "SOURCE",
    Output => "OUTPUT",
    Out => "OUT",
    Union => "UNION",
    Intersect => "INTERSECT",
    Except => "EXCEPT",
    All => "ALL",
    Between => "BETWEEN",
    Apply => "APPLY",
    Pivot => "PIVOT",
    Unpivot => "UNPIVOT",
    Over => "OVER",
    Partition => "PARTITION",
    Unbounded => "UNBOUNDED",
    Preceding => "PRECEDING",
    Following => "FOLLOWING",
    Current => "CURRENT",
    Row => "ROW",
    TryCast => "TRY_CAST",
    TryConvert => "TRY_CONVERT",
    Coalesce => "COALESCE",
    Nullif => "NULLIF",
    Offset => "OFFSET",
    Rows => "ROWS",
    Fetch => "FETCH",
    Next => "NEXT",
    Only => "ONLY",
    With => "WITH",
    Within => "WITHIN",
    Groups => "GROUPS",
    Range => "RANGE",
    CurrentDate => "CURRENT_DATE",
    CurrentTime => "CURRENT_TIME",
    CurrentTimestamp => "CURRENT_TIMESTAMP",
    CurrentUser => "CURRENT_USER",
    SessionUser => "SESSION_USER",
    SystemUser => "SYSTEM_USER",

    // DDL
    Create => "CREATE",
    Table => "TABLE",
    Identity => "IDENTITY",
    Primary => "PRIMARY",
    Key => "KEY",
    Desc => "DESC",
    Asc => "ASC",
    Alter => "ALTER",
    Add => "ADD",
    Constraint => "CONSTRAINT",
    References => "REFERENCES",
    Default => "DEFAULT",
    Check => "CHECK",
    Unique => "UNIQUE",
    Foreign => "FOREIGN",
    Drop => "DROP",
    View => "VIEW",
    Procedure => "PROCEDURE",
    Function => "FUNCTION",
    Trigger => "TRIGGER",
    Index => "INDEX",
    Clustered => "CLUSTERED",
    Nonclustered => "NONCLUSTERED",
    Fillfactor => "FILLFACTOR",
    Schema => "SCHEMA",
    Type => "TYPE",
    Column => "COLUMN",
    Proc => "PROC",

    // Control flow
    If => "IF",
    Begin => "BEGIN",
    Declare => "DECLARE",
    Set => "SET",
    Exec => "EXEC",
    Execute => "EXECUTE",
    Print => "PRINT",
    Use => "USE",
    Go => "GO",
    While => "WHILE",
    Break => "BREAK",
    Continue => "CONTINUE",
    Return => "RETURN",
    Try => "TRY",
    Catch => "CATCH",
    RaiseError => "RAISERROR",
    Throw => "THROW",

    // Transactions
    Tran => "TRAN",
    Transaction => "TRANSACTION",
    Commit => "COMMIT",
    Rollback => "ROLLBACK",
    Save => "SAVE",
    Mark => "MARK",
    Synonym => "SYNONYM",
    Sequence => "SEQUENCE",
    Value => "VALUE",
    Increment => "INCREMENT",
    Minvalue => "MINVALUE",
    Maxvalue => "MAXVALUE",
    Cycle => "CYCLE",
    Start => "START",
    Distributed => "DISTRIBUTED",

    // Cursors
    Cursor => "CURSOR",
    Open => "OPEN",
    Close => "CLOSE",
    Deallocate => "DEALLOCATE",

    // Locking hints
    Nolock => "NOLOCK",
    Rowlock => "ROWLOCK",
    Tablock => "TABLOCK",
    Holdlock => "HOLDLOCK",
    Updlock => "UPDLOCK",
    Xlock => "XLOCK",
    Readuncommitted => "READUNCOMMITTED",
    Readcommitted => "READCOMMITTED",
    Readpast => "READPAST",
    Serializable => "SERIALIZABLE",
    Snapshot => "SNAPSHOT",
    Noexpand => "NOEXPAND",

    // Join types
    Outer => "OUTER",
    Inner => "INNER",
    Left => "LEFT",
    Right => "RIGHT",
    Full => "FULL",
    Cross => "CROSS",

    // Data types - integer
    Int => "INT",
    BigInt => "BIGINT",
    SmallInt => "SMALLINT",
    TinyInt => "TINYINT",
    Bit => "BIT",

    // Data types - floating point / decimal
    Float => "FLOAT",
    Decimal => "DECIMAL",
    Numeric => "NUMERIC",
    Real => "REAL",
    Precision => "PRECISION",
    Money => "MONEY",
    SmallMoney => "SMALLMONEY",

    // Data types - character
    Char => "CHAR",
    Character => "CHARACTER",
    NChar => "NCHAR",
    Varchar => "VARCHAR",
    Varying => "VARYING",
    National => "NATIONAL",
    Nvarchar => "NVARCHAR",
    Text => "TEXT",
    NText => "NTEXT",

    // Data types - binary
    Binary => "BINARY",
    Varbinary => "VARBINARY",
    Image => "IMAGE",

    // Data types - date/time
    Date => "DATE",
    Time => "TIME",
    DateTime => "DATETIME",
    DateTime2 => "DATETIME2",
    DateTimeOffset => "DATETIMEOFFSET",
    SmallDateTime => "SMALLDATETIME",

    // Data types - other
    UniqueIdentifier => "UNIQUEIDENTIFIER",
    Xml => "XML",
    SqlVariant => "SQL_VARIANT",
    SysName => "SYSNAME",
    Max => "MAX",
    Computed => "COMPUTED",

    // Triggers
    For => "FOR",
    After => "AFTER",
    Instead => "INSTEAD",

    // Cascade / referential
    Cascade => "CASCADE",
    Action => "ACTION",
    No => "NO",
    Prior => "PRIOR",

    // Function / routine metadata
    Returns => "RETURNS",
    Absolute => "ABSOLUTE",
    Relative => "RELATIVE",
    Routine => "ROUTINE",
    Collation => "COLLATION",
    Collate => "COLLATE",

    // Isolation level
    Isolation => "ISOLATION",
    Level => "LEVEL",
    Read => "READ",
    Uncommitted => "UNCOMMITTED",
    Committed => "COMMITTED",
    Repeatable => "REPEATABLE",
    Of => "OF",
    Off => "OFF",

    // Lock timeout
    LockTimeout => "LOCK_TIMEOUT",
    Statistics => "STATISTICS",
    Showplan => "SHOWPLAN",
    Io => "IO",
    Low => "LOW",
    Normal => "NORMAL",
    High => "HIGH",

    // SET options
    NoCount => "NOCOUNT",
    Rowcount => "ROWCOUNT",
    Textsize => "TEXTSIZE",
    IdentityInsert => "IDENTITY_INSERT",
    IdentityCol => "IDENTITYCOL",
    ContextInfo => "CONTEXT_INFO",

    // DML pseudo-tables
    Inserted => "INSERTED",
    Deleted => "DELETED",

    // Bulk
    Bulk => "BULK",

    // Misc
    Last => "LAST",
    First => "FIRST",
    Any => "ANY",
    Some => "SOME",
    User => "USER",
    Database => "DATABASE",
    Backup => "BACKUP",
    Restore => "RESTORE",
    Checkpoint => "CHECKPOINT",
    Kill => "KILL",
    Authorization => "AUTHORIZATION",
    Browse => "BROWSE",
    Shutdown => "SHUTDOWN",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_parse_all_variants_case_insensitive_and_bounds() {
        for &kw in Keyword::ALL {
            let sql = kw.as_sql();
            assert!(
                sql.len() <= 24 && sql.is_ascii(),
                "keyword {sql:?} exceeds stack buffer assumptions"
            );
            assert_eq!(Keyword::parse(sql), Some(kw));
            assert_eq!(Keyword::parse(&sql.to_ascii_lowercase()), Some(kw));
            let mixed: String = sql
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    if i % 2 == 0 {
                        c.to_ascii_uppercase()
                    } else {
                        c.to_ascii_lowercase()
                    }
                })
                .collect();
            assert_eq!(Keyword::parse(&mixed), Some(kw));
        }
        assert_eq!(Keyword::parse(""), None);
        assert_eq!(Keyword::parse("NON_EXISTENT_KEYWORD"), None);
        assert_eq!(
            Keyword::parse("VERY_LONG_IDENTIFIER_EXCEEDING_24_BYTES"),
            None
        );
        assert_eq!(Keyword::parse("SELECTÉ"), None);
        assert_eq!(Keyword::parse("café"), None);
    }
}
