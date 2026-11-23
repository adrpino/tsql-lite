use super::common::{lower_data_type, lower_expr, lower_object_name};
use super::dml::lower_select;
use super::lower_statement;
use crate::ast as executor_ast;
use crate::error::DbError;
use crate::parser::ast;

pub fn lower_ddl(ddl: ast::DdlStatement) -> Result<executor_ast::Statement, DbError> {
    if let ast::DdlStatement::Create(s) = ddl {
        return lower_create(*s);
    }
    let lowered = match ddl {
        ast::DdlStatement::Create(_) => unreachable!(),
        ast::DdlStatement::AlterTable { table, action } => {
            executor_ast::statements::DdlStatement::AlterTable(
                executor_ast::statements::ddl::AlterTableStmt {
                    table: lower_object_name(table),
                    action: lower_alter_action(action)?,
                },
            )
        }
        ast::DdlStatement::TruncateTable(table) => {
            executor_ast::statements::DdlStatement::TruncateTable(
                executor_ast::statements::ddl::TruncateTableStmt {
                    name: lower_object_name(table),
                },
            )
        }
        ast::DdlStatement::DropTable { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropTable(
                executor_ast::statements::ddl::DropTableStmt {
                    name: lower_object_name(name),
                    if_exists,
                },
            )
        }
        ast::DdlStatement::DropView { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropView(
                executor_ast::statements::ddl::DropViewStmt {
                    name: lower_object_name(name),
                    if_exists,
                },
            )
        }
        ast::DdlStatement::DropProcedure { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropProcedure(
                executor_ast::statements::procedural::DropProcedureStmt {
                    name: lower_object_name(name),
                    if_exists,
                },
            )
        }
        ast::DdlStatement::DropFunction { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropFunction(
                executor_ast::statements::procedural::DropFunctionStmt {
                    name: lower_object_name(name),
                    if_exists,
                },
            )
        }
        ast::DdlStatement::DropTrigger { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropTrigger(
                executor_ast::statements::procedural::DropTriggerStmt {
                    name: lower_object_name(name),
                    if_exists,
                },
            )
        }
        ast::DdlStatement::DropIndex {
            name,
            table,
            if_exists,
        } => executor_ast::statements::DdlStatement::DropIndex(
            executor_ast::statements::ddl::DropIndexStmt {
                name: lower_object_name(name),
                table: lower_object_name(table),
                if_exists,
            },
        ),
        ast::DdlStatement::DropType { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropType(
                executor_ast::statements::ddl::DropTypeStmt {
                    name: lower_object_name(name),
                    if_exists,
                },
            )
        }
        ast::DdlStatement::DropSchema { name, if_exists } => {
            executor_ast::statements::DdlStatement::DropSchema(
                executor_ast::statements::ddl::DropSchemaStmt { name, if_exists },
            )
        }
        ast::DdlStatement::CreateIndex(stmt) => {
            executor_ast::statements::DdlStatement::CreateIndex(
                executor_ast::statements::ddl::CreateIndexStmt {
                    name: lower_object_name(stmt.name),
                    table: lower_object_name(stmt.table),
                    columns: stmt
                        .columns
                        .into_iter()
                        .map(|c| executor_ast::statements::ddl::IndexColumnSpec {
                            name: c.name,
                            is_desc: c.is_desc,
                        })
                        .collect(),
                    is_unique: stmt.is_unique,
                    is_clustered: stmt.is_clustered,
                    options: stmt
                        .options
                        .into_iter()
                        .map(|o| match o {
                            ast::IndexOption::FillFactor(f) => {
                                executor_ast::statements::ddl::IndexOptionSpec::FillFactor(f)
                            }
                        })
                        .collect(),
                    filter_predicate: stmt
                        .filter_predicate
                        .map(|e| lower_expr(*e).map(Box::new))
                        .transpose()?,
                },
            )
        }
        ast::DdlStatement::CreateType { name, columns } => {
            executor_ast::statements::DdlStatement::CreateType(
                executor_ast::statements::ddl::CreateTypeStmt {
                    name: lower_object_name(name),
                    columns: columns
                        .into_iter()
                        .map(lower_column_def)
                        .collect::<Result<Vec<_>, _>>()?,
                    table_constraints: vec![],
                },
            )
        }
        ast::DdlStatement::CreateScalarType {
            name,
            base_type,
            nullable,
        } => executor_ast::statements::DdlStatement::CreateScalarType(
            executor_ast::statements::ddl::CreateScalarTypeStmt {
                name: lower_object_name(name),
                base_type: lower_data_type(base_type)?,
                nullable,
            },
        ),
        ast::DdlStatement::CreateSchema(name) => {
            executor_ast::statements::DdlStatement::CreateSchema(
                executor_ast::statements::ddl::CreateSchemaStmt { name },
            )
        }
        ast::DdlStatement::AlterSchemaTransfer {
            target_schema,
            entity_type,
            source,
        } => executor_ast::statements::DdlStatement::AlterSchemaTransfer(
            executor_ast::statements::ddl::AlterSchemaTransferStmt {
                target_schema,
                entity_type,
                source: lower_object_name(source),
            },
        ),
        ast::DdlStatement::CatalogMetadata(action) => {
            executor_ast::statements::DdlStatement::CatalogMetadata(action)
        }
        ast::DdlStatement::CreateSynonym { name, base_object } => {
            executor_ast::statements::DdlStatement::CreateSynonym(
                executor_ast::statements::ddl::CreateSynonymStmt {
                    name: lower_object_name(name),
                    base_object: lower_object_name(base_object),
                },
            )
        }
        ast::DdlStatement::DropSynonym(name) => {
            executor_ast::statements::DdlStatement::DropSynonym(
                executor_ast::statements::ddl::DropSynonymStmt {
                    name: lower_object_name(name),
                },
            )
        }
        ast::DdlStatement::CreateSequence {
            name,
            data_type,
            start_with,
            increment_by,
            min_value,
            max_value,
            cycle,
        } => executor_ast::statements::DdlStatement::CreateSequence(
            executor_ast::statements::ddl::CreateSequenceStmt {
                name: lower_object_name(name),
                data_type: data_type
                    .map(lower_data_type)
                    .transpose()?
                    .unwrap_or(executor_ast::data_types::DataTypeSpec::BigInt),
                start_value: start_with.unwrap_or(1),
                increment: increment_by.unwrap_or(1),
                minimum_value: min_value.unwrap_or(i64::MIN),
                maximum_value: max_value.unwrap_or(i64::MAX),
                is_cycling: cycle,
            },
        ),
        ast::DdlStatement::DropSequence(name) => {
            executor_ast::statements::DdlStatement::DropSequence(
                executor_ast::statements::ddl::DropSequenceStmt {
                    name: lower_object_name(name),
                },
            )
        }
        ast::DdlStatement::AlterSequence {
            name,
            restart,
            increment_by,
            min_value,
            max_value,
            cycle,
        } => executor_ast::statements::DdlStatement::AlterSequence(
            executor_ast::statements::ddl::AlterSequenceStmt {
                name: lower_object_name(name),
                restart,
                increment: increment_by,
                minimum_value: min_value,
                maximum_value: max_value,
                is_cycling: cycle,
            },
        ),
    };
    Ok(executor_ast::Statement::Ddl(Box::new(lowered)))
}

pub fn lower_create(s: ast::CreateStmt) -> Result<executor_ast::Statement, DbError> {
    match s {
        ast::CreateStmt::Table {
            name,
            columns,
            constraints,
        } => Ok(executor_ast::Statement::Ddl(Box::new(
            executor_ast::statements::DdlStatement::CreateTable(
                executor_ast::statements::ddl::CreateTableStmt {
                    name: lower_object_name(name),
                    columns: columns
                        .into_iter()
                        .map(lower_column_def)
                        .collect::<Result<Vec<_>, _>>()?,
                    table_constraints: constraints
                        .into_iter()
                        .map(lower_table_constraint)
                        .collect::<Result<Vec<_>, _>>()?,
                },
            ),
        ))),
        ast::CreateStmt::View {
            name,
            columns,
            ctes,
            query,
            or_alter,
        } => {
            let mut lowered_query = lower_select(query)?;
            let lowered_ctes = ctes
                .into_iter()
                .map(|c| {
                    Ok(executor_ast::statements::procedural::CteDef {
                        name: c.name,
                        columns: c.columns,
                        query: executor_ast::Statement::Dml(
                            executor_ast::statements::DmlStatement::Select(Box::new(lower_select(
                                c.query,
                            )?)),
                        ),
                    })
                })
                .collect::<Result<Vec<_>, DbError>>()?;
            if !columns.is_empty() {
                let has_wildcard = lowered_query.projection.iter().any(|item| {
                    matches!(
                        item.expr,
                        executor_ast::expressions::Expr::Wildcard
                            | executor_ast::expressions::Expr::QualifiedWildcard(_)
                    )
                });
                if !has_wildcard {
                    if columns.len() != lowered_query.projection.len() {
                        return Err(DbError::Semantic(format!(
                            "CREATE VIEW column list specifies {} column(s), but SELECT list has {}",
                            columns.len(),
                            lowered_query.projection.len()
                        )));
                    }
                    for (item, col_name) in lowered_query.projection.iter_mut().zip(columns) {
                        item.alias = Some(col_name);
                    }
                }
            }
            Ok(executor_ast::Statement::Procedural(
                executor_ast::statements::ProceduralStatement::CreateView(Box::new(
                    executor_ast::statements::ddl::CreateViewStmt {
                        name: lower_object_name(name),
                        ctes: lowered_ctes,
                        query: lowered_query,
                        or_alter,
                    },
                )),
            ))
        }
        ast::CreateStmt::Procedure {
            name,
            params,
            body,
            or_alter,
        } => Ok(executor_ast::Statement::Procedural(
            executor_ast::statements::ProceduralStatement::CreateProcedure(Box::new(
                executor_ast::statements::procedural::CreateProcedureStmt {
                    name: lower_object_name(name),
                    params: params
                        .into_iter()
                        .map(lower_routine_param)
                        .collect::<Result<Vec<_>, _>>()?,
                    body: body
                        .into_iter()
                        .map(lower_statement)
                        .collect::<Result<Vec<_>, _>>()?,
                    or_alter,
                },
            )),
        )),
        ast::CreateStmt::Function {
            name,
            params,
            returns,
            body,
            or_alter,
        } => Ok(executor_ast::Statement::Procedural(
            executor_ast::statements::ProceduralStatement::CreateFunction(Box::new(
                executor_ast::statements::procedural::CreateFunctionStmt {
                    name: lower_object_name(name),
                    params: params
                        .into_iter()
                        .map(lower_routine_param)
                        .collect::<Result<Vec<_>, _>>()?,
                    returns: returns.map(lower_data_type).transpose()?,
                    or_alter,
                    body: match body {
                        ast::FunctionBody::ScalarReturn(e) => {
                            executor_ast::statements::procedural::FunctionBody::ScalarReturn(
                                lower_expr(e)?,
                            )
                        }
                        ast::FunctionBody::Block(stmts) => {
                            executor_ast::statements::procedural::FunctionBody::Scalar(
                                stmts
                                    .into_iter()
                                    .map(lower_statement)
                                    .collect::<Result<Vec<_>, _>>()?,
                            )
                        }
                        ast::FunctionBody::Table(sel) => {
                            executor_ast::statements::procedural::FunctionBody::InlineTable(
                                Box::new(lower_select(sel)?),
                            )
                        }
                        ast::FunctionBody::TableWithCte { ctes, query } => {
                            let lowered_ctes = ctes
                                .into_iter()
                                .map(|c| {
                                    Ok(executor_ast::statements::procedural::CteDef {
                                        name: c.name,
                                        columns: c.columns,
                                        query: executor_ast::Statement::Dml(
                                            executor_ast::statements::DmlStatement::Select(
                                                Box::new(lower_select(c.query)?),
                                            ),
                                        ),
                                    })
                                })
                                .collect::<Result<Vec<_>, DbError>>()?;
                            executor_ast::statements::procedural::FunctionBody::InlineTableWithCte(
                                Box::new(executor_ast::statements::procedural::WithCteStmt {
                                    recursive: false,
                                    ctes: lowered_ctes,
                                    body: Box::new(executor_ast::Statement::Dml(
                                        executor_ast::statements::DmlStatement::Select(Box::new(
                                            lower_select(query)?,
                                        )),
                                    )),
                                }),
                            )
                        }
                        ast::FunctionBody::MultiStatementTable {
                            table_var,
                            columns,
                            constraints,
                            body,
                        } => executor_ast::statements::procedural::FunctionBody::MultiStatementTable {
                            table_var,
                            columns: columns
                                .into_iter()
                                .map(lower_column_def)
                                .collect::<Result<Vec<_>, _>>()?,
                            constraints: constraints
                                .into_iter()
                                .map(lower_table_constraint)
                                .collect::<Result<Vec<_>, _>>()?,
                            body: body
                                .into_iter()
                                .map(lower_statement)
                                .collect::<Result<Vec<_>, _>>()?,
                        },
                    },
                },
            )),
        )),
        ast::CreateStmt::Trigger {
            name,
            table,
            events,
            is_instead_of,
            body,
            or_alter,
        } => Ok(executor_ast::Statement::Procedural(
            executor_ast::statements::ProceduralStatement::CreateTrigger(Box::new(
                executor_ast::statements::procedural::CreateTriggerStmt {
                    name: lower_object_name(name),
                    table: lower_object_name(table),
                    events: events
                        .into_iter()
                        .map(|e| match e {
                            ast::TriggerEvent::Insert => executor_ast::TriggerEvent::Insert,
                            ast::TriggerEvent::Update => executor_ast::TriggerEvent::Update,
                            ast::TriggerEvent::Delete => executor_ast::TriggerEvent::Delete,
                        })
                        .collect(),
                    is_instead_of,
                    body: body
                        .into_iter()
                        .map(lower_statement)
                        .collect::<Result<Vec<_>, _>>()?,
                    or_alter,
                },
            )),
        )),
    }
}

pub fn lower_column_def(
    c: ast::ColumnDef,
) -> Result<executor_ast::statements::ddl::ColumnSpec, DbError> {
    let nullable_explicit = c.is_nullable.is_some();
    Ok(executor_ast::statements::ddl::ColumnSpec {
        name: c.name,
        data_type: lower_data_type(c.data_type)?,
        nullable: c.is_nullable.unwrap_or(true),
        nullable_explicit,
        identity: c.identity_spec,
        primary_key: c.is_primary_key,
        unique: c.is_unique,
        default: c.default_expr.map(lower_expr).transpose()?,
        default_constraint_name: c.default_constraint_name,
        check: c.check_expr.map(lower_expr).transpose()?,
        check_constraint_name: c.check_constraint_name,
        computed_expr: c.computed_expr.map(lower_expr).transpose()?,
        foreign_key: c
            .foreign_key
            .map(|fk| executor_ast::statements::ddl::ForeignKeyRef {
                referenced_table: lower_object_name(fk.ref_table),
                referenced_columns: fk.ref_columns,
                on_delete: fk.on_delete.map(lower_referential_action),
                on_update: fk.on_update.map(lower_referential_action),
            }),
        collation: c.collation,
        is_clustered: c.is_clustered,
        ansi_padding_on: true,
    })
}

pub fn lower_index_column(c: ast::IndexColumn) -> executor_ast::statements::ddl::IndexColumnSpec {
    executor_ast::statements::ddl::IndexColumnSpec {
        name: c.name,
        is_desc: c.is_desc,
    }
}

pub fn lower_routine_param(
    p: ast::RoutineParam,
) -> Result<executor_ast::statements::RoutineParam, DbError> {
    let (param_type, is_readonly) = match p.data_type {
        ast::DataType::Custom(name) => {
            let name_str = name.as_str();
            let (schema, type_name) = match name_str.rsplit_once('.') {
                Some((schema, ty)) => (Some(schema.to_string()), ty.to_string()),
                None => (None, name_str.to_string()),
            };
            let obj = executor_ast::ObjectName {
                database: None,
                schema,
                name: type_name,
            };
            if p.is_readonly {
                (
                    executor_ast::statements::RoutineParamType::TableType(obj),
                    true,
                )
            } else {
                (
                    executor_ast::statements::RoutineParamType::Scalar(
                        executor_ast::data_types::DataTypeSpec::Custom(Box::new(obj)),
                    ),
                    false,
                )
            }
        }
        other => (
            executor_ast::statements::RoutineParamType::Scalar(lower_data_type(other)?),
            p.is_readonly,
        ),
    };
    Ok(executor_ast::statements::RoutineParam {
        name: p.name,
        param_type,
        is_output: p.is_output,
        is_readonly,
        default: p.default.map(lower_expr).transpose()?,
    })
}

pub fn lower_alter_action(
    a: ast::AlterTableAction,
) -> Result<executor_ast::statements::ddl::AlterTableAction, DbError> {
    match a {
        ast::AlterTableAction::AddColumn(c) => Ok(
            executor_ast::statements::ddl::AlterTableAction::AddColumn(lower_column_def(c)?),
        ),
        ast::AlterTableAction::DropColumn(c) => Ok(
            executor_ast::statements::ddl::AlterTableAction::DropColumn(c),
        ),
        ast::AlterTableAction::AlterColumn {
            name,
            data_type,
            nullable,
        } => Ok(
            executor_ast::statements::ddl::AlterTableAction::AlterColumn {
                name,
                data_type: lower_data_type(data_type)?,
                nullable,
            },
        ),
        ast::AlterTableAction::AddConstraint {
            constraint,
            with_check,
        } => Ok(
            executor_ast::statements::ddl::AlterTableAction::AddConstraint {
                constraint: lower_table_constraint(constraint)?,
                with_check,
            },
        ),
        ast::AlterTableAction::DropConstraint(c) => {
            Ok(executor_ast::statements::ddl::AlterTableAction::DropConstraint(c))
        }
        ast::AlterTableAction::CheckConstraint {
            check,
            with_check,
            name,
        } => Ok(
            executor_ast::statements::ddl::AlterTableAction::CheckConstraint {
                check,
                with_check,
                name,
            },
        ),
        ast::AlterTableAction::ToggleTrigger {
            trigger_name,
            enable,
        } => Ok(
            executor_ast::statements::ddl::AlterTableAction::ToggleTrigger {
                trigger_name: trigger_name.map(lower_object_name),
                enable,
            },
        ),
        ast::AlterTableAction::SetSystemVersioning {
            enabled,
            history_table,
        } => Ok(
            executor_ast::statements::ddl::AlterTableAction::SetSystemVersioning {
                enabled,
                history_table: history_table.map(lower_object_name),
            },
        ),
    }
}

pub fn lower_table_constraint(
    c: ast::TableConstraint,
) -> Result<executor_ast::statements::ddl::TableConstraintSpec, DbError> {
    match c {
        ast::TableConstraint::PrimaryKey {
            name,
            columns,
            is_clustered,
        } => Ok(
            executor_ast::statements::ddl::TableConstraintSpec::PrimaryKey {
                name: name.unwrap_or_default(),
                columns: columns.into_iter().map(lower_index_column).collect(),
                is_clustered,
            },
        ),
        ast::TableConstraint::Unique {
            name,
            columns,
            is_clustered,
        } => Ok(executor_ast::statements::ddl::TableConstraintSpec::Unique {
            name: name.unwrap_or_default(),
            columns: columns.into_iter().map(lower_index_column).collect(),
            is_clustered,
        }),
        ast::TableConstraint::ForeignKey {
            name,
            columns,
            ref_table,
            ref_columns,
            on_delete,
            on_update,
        } => Ok(
            executor_ast::statements::ddl::TableConstraintSpec::ForeignKey {
                name: name.unwrap_or_default(),
                columns,
                referenced_table: lower_object_name(ref_table),
                referenced_columns: ref_columns,
                on_delete: on_delete.map(lower_referential_action),
                on_update: on_update.map(lower_referential_action),
            },
        ),
        ast::TableConstraint::Check { name, expr } => {
            Ok(executor_ast::statements::ddl::TableConstraintSpec::Check {
                name: name.unwrap_or_default(),
                expr: lower_expr(expr)?,
            })
        }
        ast::TableConstraint::Default { name, column, expr } => Ok(
            executor_ast::statements::ddl::TableConstraintSpec::Default {
                name: name.unwrap_or_default(),
                column,
                expr: lower_expr(expr)?,
            },
        ),
        ast::TableConstraint::SystemTimePeriod { start_col, end_col } => Ok(
            executor_ast::statements::ddl::TableConstraintSpec::SystemTimePeriod {
                start_col,
                end_col,
            },
        ),
        ast::TableConstraint::SystemVersioning {
            enabled,
            history_table,
        } => Ok(
            executor_ast::statements::ddl::TableConstraintSpec::SystemVersioning {
                enabled,
                history_table: history_table.map(lower_object_name),
            },
        ),
    }
}

pub fn lower_referential_action(
    a: ast::ReferentialAction,
) -> executor_ast::statements::ddl::ReferentialAction {
    match a {
        ast::ReferentialAction::NoAction => {
            executor_ast::statements::ddl::ReferentialAction::NoAction
        }
        ast::ReferentialAction::Cascade => {
            executor_ast::statements::ddl::ReferentialAction::Cascade
        }
        ast::ReferentialAction::SetNull => {
            executor_ast::statements::ddl::ReferentialAction::SetNull
        }
        ast::ReferentialAction::SetDefault => {
            executor_ast::statements::ddl::ReferentialAction::SetDefault
        }
    }
}
