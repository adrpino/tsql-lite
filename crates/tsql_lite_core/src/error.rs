use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    Parse,
    Semantic,
    Execution,
    Storage,
}

#[derive(Debug, Clone)]
pub enum DbError {
    Parse(String),
    Semantic(String),
    Execution(String),
    Storage(String),
    Deadlock(String),

    /// A custom error with an explicit SQL Server–style class and number.
    /// Allows callers to raise domain-specific errors (e.g. timeout, permission)
    /// without modifying the DbError enum itself.
    Custom {
        class: u8,
        number: i32,
        message: String,
    },

    /// A user-raised SQL error (RAISERROR / THROW) preserving exact number, severity, and state.
    Raised {
        number: i32,
        severity: u8,
        state: u8,
        message: String,
    },

    // -- Strongly-typed semantic errors --
    SchemaNotFound { schema: String },
    TableNotFound { schema: String, table: String },
    ColumnNotFound { column: String },
    ColumnNotFoundQualified { table: String, column: String },
    TypeMismatch { expected: String, found: String },
    IndexNotFound { table: String, index: String },
    PrimaryKeyNotFound { table: String },
    ConstraintNotFound { table: String, constraint: String },
    DatabaseNotFound { database: String },
    ObjectNotFound { object: String },
    DuplicateColumn { column: String },
    DuplicateTable { schema: String, table: String },
    InvalidIdentifier { identifier: String },
    TriggerNotFound { schema: String, trigger: String },
    DuplicateTrigger { schema: String, trigger: String },
    ViewNotFound { schema: String, view: String },
    DuplicateView { schema: String, view: String },
    TypeNotFound { schema: String, type_name: String },
    DuplicateType { schema: String, type_name: String },
    DuplicateSchema { schema: String },
    CursorNotDeclared { cursor: String },
    CursorHasNoQuery { cursor: String },
    DivideByZero,
    ConversionFailed {
        from_type: String,
        value: String,
        to_type: String,
    },
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Parse(s) => write!(f, "Incorrect syntax near '{s}'."),
            DbError::Semantic(s) => f.write_str(s),
            DbError::Execution(s) => write!(f, "execution error: {s}"),
            DbError::Storage(s) => f.write_str(s),
            DbError::Deadlock(_) => f.write_str(
                "Transaction (Process ID 0) was deadlocked on resources with another process and has been chosen as the deadlock victim. Rerun the transaction.",
            ),
            DbError::Custom { message, .. } => f.write_str(message),
            DbError::Raised { message, .. } => f.write_str(message),
            DbError::SchemaNotFound { schema } => write!(f, "Invalid schema name '{schema}'."),
            DbError::TableNotFound { table, .. } => write!(f, "Invalid object name '{table}'."),
            DbError::ColumnNotFound { column } => write!(f, "Invalid column name '{column}'."),
            DbError::ColumnNotFoundQualified { column, .. } => {
                write!(f, "Invalid column name '{column}'.")
            }
            DbError::TypeMismatch { expected, found } => {
                write!(f, "Type mismatch: expected {expected}, found {found}")
            }
            DbError::IndexNotFound { table, index } => {
                write!(f, "Index '{index}' not found on table '{table}'.")
            }
            DbError::PrimaryKeyNotFound { table } => {
                write!(f, "Primary key not found on table '{table}'.")
            }
            DbError::ConstraintNotFound { table, constraint } => {
                write!(f, "Constraint '{constraint}' not found on table '{table}'.")
            }
            DbError::DatabaseNotFound { database } => write!(
                f,
                "Database '{database}' does not exist. Make sure that the name is entered correctly and try again."
            ),
            DbError::ObjectNotFound { object } => write!(f, "Invalid object name '{object}'."),
            DbError::DuplicateColumn { column } => write!(
                f,
                "Column names in each table must be unique. Column name '{column}' in table is specified more than once."
            ),
            DbError::DuplicateTable { table, .. } => {
                write!(f, "There is already an object named '{table}' in the database.")
            }
            DbError::InvalidIdentifier { identifier } => {
                write!(f, "The identifier '{identifier}' is too long. Maximum length is 128.")
            }
            DbError::TriggerNotFound { schema, trigger } => {
                write!(f, "Invalid object name '{schema}.{trigger}'.")
            }
            DbError::DuplicateTrigger { trigger, .. } => {
                write!(f, "There is already an object named '{trigger}' in the database.")
            }
            DbError::ViewNotFound { schema, view } => {
                write!(f, "Invalid object name '{schema}.{view}'.")
            }
            DbError::DuplicateView { view, .. } => {
                write!(f, "There is already an object named '{view}' in the database.")
            }
            DbError::TypeNotFound { schema, type_name } => {
                write!(f, "Type '{schema}.{type_name}' not found.")
            }
            DbError::DuplicateType { type_name, .. } => {
                write!(f, "There is already an object named '{type_name}' in the database.")
            }
            DbError::DuplicateSchema { schema } => {
                write!(f, "There is already a schema named '{schema}' in the database.")
            }
            DbError::CursorNotDeclared { cursor } => {
                write!(f, "The cursor '{cursor}' does not exist.")
            }
            DbError::CursorHasNoQuery { cursor } => {
                write!(f, "The cursor '{cursor}' has no query.")
            }
            DbError::DivideByZero => f.write_str("Divide by zero error encountered."),
            DbError::ConversionFailed {
                from_type,
                value,
                to_type,
            } => write!(
                f,
                "Conversion failed when converting the {from_type} value '{value}' to data type {to_type}."
            ),
        }
    }
}

impl std::error::Error for DbError {}

impl DbError {
    pub fn class(&self) -> ErrorClass {
        match self {
            DbError::Parse(_) => ErrorClass::Parse,
            DbError::Semantic(_)
            | DbError::SchemaNotFound { .. }
            | DbError::TableNotFound { .. }
            | DbError::ColumnNotFound { .. }
            | DbError::ColumnNotFoundQualified { .. }
            | DbError::TypeMismatch { .. }
            | DbError::IndexNotFound { .. }
            | DbError::PrimaryKeyNotFound { .. }
            | DbError::ConstraintNotFound { .. }
            | DbError::DatabaseNotFound { .. }
            | DbError::ObjectNotFound { .. }
            | DbError::DuplicateColumn { .. }
            | DbError::DuplicateTable { .. }
            | DbError::InvalidIdentifier { .. }
            | DbError::TriggerNotFound { .. }
            | DbError::DuplicateTrigger { .. }
            | DbError::ViewNotFound { .. }
            | DbError::DuplicateView { .. }
            | DbError::TypeNotFound { .. }
            | DbError::DuplicateType { .. }
            | DbError::DuplicateSchema { .. }
            | DbError::CursorNotDeclared { .. }
            | DbError::CursorHasNoQuery { .. } => ErrorClass::Semantic,
            DbError::Execution(_)
            | DbError::Deadlock(_)
            | DbError::Custom { .. }
            | DbError::Raised { .. }
            | DbError::DivideByZero
            | DbError::ConversionFailed { .. } => ErrorClass::Execution,
            DbError::Storage(_) => ErrorClass::Storage,
        }
    }

    pub fn class_severity(&self) -> u8 {
        match self {
            DbError::Custom { class, .. } => *class,
            DbError::Raised { severity, .. } => *severity,
            _ => match self.class() {
                ErrorClass::Parse => 15,
                ErrorClass::Semantic => 16,
                ErrorClass::Execution => 16,
                ErrorClass::Storage => 17,
            },
        }
    }

    pub fn state(&self) -> u8 {
        match self {
            DbError::Raised { state, .. } => *state,
            _ => 1,
        }
    }

    pub fn number(&self) -> i32 {
        match self {
            DbError::Parse(_) => 102,
            DbError::SchemaNotFound { .. }
            | DbError::TableNotFound { .. }
            | DbError::ObjectNotFound { .. }
            | DbError::TriggerNotFound { .. }
            | DbError::ViewNotFound { .. } => 208, // Invalid object name
            DbError::ColumnNotFound { .. } | DbError::ColumnNotFoundQualified { .. } => 207, // Invalid column name
            DbError::DatabaseNotFound { .. } => 911,
            DbError::DuplicateTable { .. }
            | DbError::DuplicateTrigger { .. }
            | DbError::DuplicateView { .. }
            | DbError::DuplicateType { .. }
            | DbError::DuplicateSchema { .. } => 2714, // There is already an object named...
            DbError::DuplicateColumn { .. } => 2705,
            DbError::Deadlock(_) => 1205,
            DbError::CursorNotDeclared { .. } => 16916,
            DbError::DivideByZero => 8134,
            DbError::ConversionFailed { .. } => 245,
            DbError::Semantic(_) => 50000,
            DbError::Execution(msg) => {
                let lower = msg.to_ascii_lowercase();
                if lower.contains("check constraint")
                    || lower.contains("foreign key constraint")
                    || lower.contains("reference constraint")
                {
                    547
                } else if lower.contains("create unique index") && lower.contains("duplicate key") {
                    1505
                } else if lower.contains("with unique index") {
                    2601
                } else if lower.contains("primary key")
                    || lower.contains("unique key")
                    || lower.contains("duplicate key")
                {
                    2627
                } else if lower.contains("8672") || lower.contains("same row more than once") {
                    8672
                } else if lower.contains("cannot insert the value null")
                    || lower.contains("cannot be null")
                    || lower.contains("does not allow null")
                {
                    515
                } else if lower.contains("arithmetic overflow") {
                    220
                } else if lower.contains("would be truncated") {
                    2628
                } else if lower.contains("divide by zero") {
                    8134
                } else if lower.contains("lock timeout")
                    || lower.contains("lock request time out")
                    || lower.contains("lock conflict (no-wait)")
                {
                    1222
                } else if lower
                    .contains("snapshot isolation transaction aborted due to update conflict")
                {
                    3960
                } else if lower.contains("snapshot isolation transaction failed accessing database")
                {
                    3952
                } else if lower.contains("because the database is read-only") {
                    3906
                } else if lower.contains("because it is currently in use") {
                    3702
                } else {
                    50000
                }
            }
            DbError::Storage(_) => 50001,
            DbError::Custom { number, .. } => *number,
            DbError::Raised { number, .. } => *number,
            _ => 50000,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            DbError::Parse(_) => "TSQL_PARSE_ERROR",
            DbError::Semantic(_) => "TSQL_SEMANTIC_ERROR",
            DbError::SchemaNotFound { .. } => "TSQL_SCHEMA_NOT_FOUND",
            DbError::TableNotFound { .. } => "TSQL_TABLE_NOT_FOUND",
            DbError::ColumnNotFound { .. } => "TSQL_COLUMN_NOT_FOUND",
            DbError::ColumnNotFoundQualified { .. } => "TSQL_COLUMN_NOT_FOUND_QUALIFIED",
            DbError::TypeMismatch { .. } => "TSQL_TYPE_MISMATCH",
            DbError::IndexNotFound { .. } => "TSQL_INDEX_NOT_FOUND",
            DbError::PrimaryKeyNotFound { .. } => "TSQL_PRIMARY_KEY_NOT_FOUND",
            DbError::ConstraintNotFound { .. } => "TSQL_CONSTRAINT_NOT_FOUND",
            DbError::DatabaseNotFound { .. } => "TSQL_DATABASE_NOT_FOUND",
            DbError::ObjectNotFound { .. } => "TSQL_OBJECT_NOT_FOUND",
            DbError::DuplicateColumn { .. } => "TSQL_DUPLICATE_COLUMN",
            DbError::DuplicateTable { .. } => "TSQL_DUPLICATE_TABLE",
            DbError::InvalidIdentifier { .. } => "TSQL_INVALID_IDENTIFIER",
            DbError::TriggerNotFound { .. } => "TSQL_TRIGGER_NOT_FOUND",
            DbError::DuplicateTrigger { .. } => "TSQL_DUPLICATE_TRIGGER",
            DbError::ViewNotFound { .. } => "TSQL_VIEW_NOT_FOUND",
            DbError::DuplicateView { .. } => "TSQL_DUPLICATE_VIEW",
            DbError::TypeNotFound { .. } => "TSQL_TYPE_NOT_FOUND",
            DbError::DuplicateType { .. } => "TSQL_DUPLICATE_TYPE",
            DbError::DuplicateSchema { .. } => "TSQL_DUPLICATE_SCHEMA",
            DbError::CursorNotDeclared { .. } => "TSQL_CURSOR_NOT_DECLARED",
            DbError::CursorHasNoQuery { .. } => "TSQL_CURSOR_HAS_NO_QUERY",
            DbError::DivideByZero => "TSQL_DIVIDE_BY_ZERO",
            DbError::ConversionFailed { .. } => "TSQL_CONVERSION_FAILED",
            DbError::Execution(_) => "TSQL_EXECUTION_ERROR",
            DbError::Storage(_) => "TSQL_STORAGE_ERROR",
            DbError::Deadlock(_) => "TSQL_DEADLOCK_ERROR",
            DbError::Custom { .. } | DbError::Raised { .. } => "TSQL_CUSTOM_ERROR",
        }
    }

    // -- Strongly-typed constructors --
    pub fn schema_not_found(schema: impl Into<String>) -> Self {
        DbError::SchemaNotFound {
            schema: schema.into(),
        }
    }

    pub fn table_not_found(schema: impl Into<String>, table: impl Into<String>) -> Self {
        DbError::TableNotFound {
            schema: schema.into(),
            table: table.into(),
        }
    }

    pub fn column_not_found(column: impl Into<String>) -> Self {
        DbError::ColumnNotFound {
            column: column.into(),
        }
    }

    pub fn column_not_found_qualified(table: impl Into<String>, column: impl Into<String>) -> Self {
        DbError::ColumnNotFoundQualified {
            table: table.into(),
            column: column.into(),
        }
    }

    pub fn type_mismatch(expected: impl Into<String>, found: impl Into<String>) -> Self {
        DbError::TypeMismatch {
            expected: expected.into(),
            found: found.into(),
        }
    }

    pub fn index_not_found(table: impl Into<String>, index: impl Into<String>) -> Self {
        DbError::IndexNotFound {
            table: table.into(),
            index: index.into(),
        }
    }

    pub fn primary_key_not_found(table: impl Into<String>) -> Self {
        DbError::PrimaryKeyNotFound {
            table: table.into(),
        }
    }

    pub fn constraint_not_found(table: impl Into<String>, constraint: impl Into<String>) -> Self {
        DbError::ConstraintNotFound {
            table: table.into(),
            constraint: constraint.into(),
        }
    }

    pub fn database_not_found(database: impl Into<String>) -> Self {
        DbError::DatabaseNotFound {
            database: database.into(),
        }
    }

    pub fn object_not_found(object: impl Into<String>) -> Self {
        DbError::ObjectNotFound {
            object: object.into(),
        }
    }

    pub fn duplicate_column(column: impl Into<String>) -> Self {
        DbError::DuplicateColumn {
            column: column.into(),
        }
    }

    pub fn duplicate_table(schema: impl Into<String>, table: impl Into<String>) -> Self {
        DbError::DuplicateTable {
            schema: schema.into(),
            table: table.into(),
        }
    }

    pub fn invalid_identifier(identifier: impl Into<String>) -> Self {
        DbError::InvalidIdentifier {
            identifier: identifier.into(),
        }
    }

    pub fn trigger_not_found(schema: impl Into<String>, trigger: impl Into<String>) -> Self {
        DbError::TriggerNotFound {
            schema: schema.into(),
            trigger: trigger.into(),
        }
    }

    pub fn duplicate_trigger(schema: impl Into<String>, trigger: impl Into<String>) -> Self {
        DbError::DuplicateTrigger {
            schema: schema.into(),
            trigger: trigger.into(),
        }
    }

    pub fn view_not_found(schema: impl Into<String>, view: impl Into<String>) -> Self {
        DbError::ViewNotFound {
            schema: schema.into(),
            view: view.into(),
        }
    }

    pub fn duplicate_view(schema: impl Into<String>, view: impl Into<String>) -> Self {
        DbError::DuplicateView {
            schema: schema.into(),
            view: view.into(),
        }
    }

    pub fn type_not_found(schema: impl Into<String>, type_name: impl Into<String>) -> Self {
        DbError::TypeNotFound {
            schema: schema.into(),
            type_name: type_name.into(),
        }
    }

    pub fn duplicate_type(schema: impl Into<String>, type_name: impl Into<String>) -> Self {
        DbError::DuplicateType {
            schema: schema.into(),
            type_name: type_name.into(),
        }
    }

    pub fn duplicate_schema(schema: impl Into<String>) -> Self {
        DbError::DuplicateSchema {
            schema: schema.into(),
        }
    }

    pub fn cursor_not_declared(cursor: impl Into<String>) -> Self {
        DbError::CursorNotDeclared {
            cursor: cursor.into(),
        }
    }

    pub fn cursor_has_no_query(cursor: impl Into<String>) -> Self {
        DbError::CursorHasNoQuery {
            cursor: cursor.into(),
        }
    }

    pub fn divide_by_zero() -> Self {
        DbError::DivideByZero
    }

    pub fn conversion_failed(
        from_type: impl Into<String>,
        value: impl Into<String>,
        to_type: impl Into<String>,
    ) -> Self {
        DbError::ConversionFailed {
            from_type: from_type.into(),
            value: value.into(),
            to_type: to_type.into(),
        }
    }
}

/// Represents the outcome of executing a SQL statement.
/// Control flow signals (BREAK, CONTINUE, RETURN) are modeled on the success path
/// rather than as error variants, preventing accidental swallowing by catch-all
/// error handlers and keeping TRY...CATCH semantics clean.
#[derive(Debug, Clone)]
pub enum StmtOutcome<T> {
    /// Statement completed normally.
    Ok(T),
    /// T-SQL BREAK statement inside a WHILE loop.
    Break,
    /// T-SQL CONTINUE statement inside a WHILE loop.
    Continue,
    /// T-SQL GOTO statement targeting a label name.
    Goto(String),
    /// T-SQL RETURN statement, optionally carrying a value (for functions/procedures).
    Return(Option<crate::types::Value>),
}

/// Convenience type alias for statement execution results.
pub type StmtResult<T> = Result<StmtOutcome<T>, DbError>;

impl<T> StmtOutcome<T> {
    /// Returns true if this is a normal completion (Ok).
    pub fn is_ok(&self) -> bool {
        matches!(self, StmtOutcome::Ok(_))
    }

    /// Returns true if this is any control flow signal.
    pub fn is_control_flow(&self) -> bool {
        !matches!(self, StmtOutcome::Ok(_))
    }

    /// Maps the Ok value using the given function, preserving control flow signals.
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> StmtOutcome<U> {
        match self {
            StmtOutcome::Ok(v) => StmtOutcome::Ok(f(v)),
            StmtOutcome::Break => StmtOutcome::Break,
            StmtOutcome::Continue => StmtOutcome::Continue,
            StmtOutcome::Goto(label) => StmtOutcome::Goto(label),
            StmtOutcome::Return(v) => StmtOutcome::Return(v),
        }
    }

    /// Converts a StmtOutcome into a Result, treating control flow signals as errors.
    /// This is used at boundaries where control flow signals should not escape
    /// (e.g., top-level statement execution outside loops/functions).
    pub fn into_result(self) -> Result<T, DbError> {
        match self {
            StmtOutcome::Ok(v) => Ok(v),
            StmtOutcome::Break => Err(DbError::Execution("BREAK outside of WHILE".into())),
            StmtOutcome::Continue => Err(DbError::Execution("CONTINUE outside of WHILE".into())),
            StmtOutcome::Goto(label) => Err(DbError::Execution(format!(
                "Label '{}' has not been declared.",
                label
            ))),
            StmtOutcome::Return(_) => Err(DbError::Execution(
                "RETURN outside of procedure/function".into(),
            )),
        }
    }

    /// Converts a StmtOutcome into a Result, swallowing RETURN signals as Ok(None).
    /// Used at procedure/function boundaries where RETURN is expected.
    pub fn into_result_swallow_return(self) -> Result<T, DbError>
    where
        T: Default,
    {
        match self {
            StmtOutcome::Ok(v) => Ok(v),
            StmtOutcome::Break => Err(DbError::Execution("BREAK outside of WHILE".into())),
            StmtOutcome::Continue => Err(DbError::Execution("CONTINUE outside of WHILE".into())),
            StmtOutcome::Goto(label) => Err(DbError::Execution(format!(
                "Label '{}' has not been declared.",
                label
            ))),
            StmtOutcome::Return(_) => Ok(T::default()),
        }
    }

    /// Returns the inner value if Ok, or the provided default.
    pub fn unwrap_or(self, default: T) -> T {
        match self {
            StmtOutcome::Ok(v) => v,
            _ => default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DbError;

    #[test]
    fn sql_server_error_numbers_match_expected_codes() {
        assert_eq!(
            DbError::Execution(
                "Violation of PRIMARY KEY constraint. Cannot insert duplicate key.".into()
            )
            .number(),
            2627
        );
        assert_eq!(
            DbError::Execution(
                "Violation of UNIQUE KEY constraint on column 'Code'. Cannot insert duplicate key."
                    .into()
            )
            .number(),
            2627
        );
        assert_eq!(
            DbError::Execution(
                "Cannot insert duplicate key row in object 'dbo.Users' with unique index 'UX_Users_Email'."
                    .into()
            )
            .number(),
            2601
        );
        assert_eq!(
            DbError::Execution(
                "The CREATE UNIQUE INDEX statement terminated because a duplicate key was found for the object name 'dbo.Users' and the index name 'UX_Users_Email'."
                    .into()
            )
            .number(),
            1505
        );
        assert_eq!(
            DbError::Execution("FOREIGN KEY constraint violation".into()).number(),
            547
        );
        assert_eq!(
            DbError::Execution("CHECK constraint violation".into()).number(),
            547
        );
        assert_eq!(
            DbError::Execution("String or binary data would be truncated for column 'Name'".into())
                .number(),
            2628
        );
        assert_eq!(
            DbError::Execution("lock timeout (200ms): Write lock on 'dbo.q' is blocked".into())
                .number(),
            1222
        );
        assert_eq!(
            DbError::Execution("lock conflict (no-wait): Write lock on 'dbo.q' is blocked".into())
                .number(),
            1222
        );
        assert_eq!(
            DbError::Execution(
                "Snapshot isolation transaction aborted due to update conflict.".into()
            )
            .number(),
            3960
        );
        assert_eq!(
            DbError::Execution(
                "Snapshot isolation transaction failed accessing database 'AppDb' because snapshot isolation is not allowed in this database."
                    .into()
            )
            .number(),
            3952
        );
        assert_eq!(DbError::Deadlock("victim".into()).number(), 1205);
        assert_eq!(DbError::table_not_found("dbo", "Missing").number(), 208);
        assert_eq!(DbError::column_not_found("MissingCol").number(), 207);
        assert_eq!(DbError::DivideByZero.number(), 8134);
    }
}
