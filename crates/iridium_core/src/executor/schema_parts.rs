use crate::ast::{
    AlterTableAction, AlterTableStmt, CreateIndexStmt, CreateScalarTypeStmt, CreateSchemaStmt,
    CreateTableStmt, CreateTypeStmt, DropIndexStmt, DropSchemaStmt, DropTableStmt, DropTypeStmt,
    TableConstraintSpec,
};
use crate::catalog::{
    CheckConstraintDef, ColumnDef, ForeignKeyDef, IdentityDef, ScalarTypeDef, TableDef,
    TableTypeDef,
};
use crate::error::DbError;

use super::schema::SchemaExecutor;
use super::schema_physical;
use super::type_mapping::{resolve_data_type_spec, resolve_data_type_spec_with_nullability};

pub(crate) fn apply_table_constraint(
    table: &mut TableDef,
    tc: TableConstraintSpec,
) -> Result<(), DbError> {
    match tc {
        TableConstraintSpec::Default { name, column, expr } => {
            let col = table
                .columns
                .iter_mut()
                .find(|c| c.name.eq_ignore_ascii_case(&column))
                .ok_or_else(|| DbError::column_not_found(&column))?;
            col.default = Some(expr);
            col.default_constraint_name = Some(name);
        }
        TableConstraintSpec::Check { name, expr } => {
            table.check_constraints.push(CheckConstraintDef {
                name,
                expr,
                enabled: true,
                is_not_trusted: false,
            });
        }
        TableConstraintSpec::ForeignKey {
            name,
            columns,
            referenced_table,
            referenced_columns,
            on_delete,
            on_update,
        } => {
            table.foreign_keys.push(ForeignKeyDef {
                name,
                columns,
                referenced_table,
                referenced_columns,
                on_delete: on_delete.unwrap_or(crate::ast::ReferentialAction::NoAction),
                on_update: on_update.unwrap_or(crate::ast::ReferentialAction::NoAction),
                enabled: true,
                is_not_trusted: false,
            });
        }
        TableConstraintSpec::PrimaryKey {
            name: _,
            columns,
            is_clustered,
        } => {
            for col_spec in &columns {
                let col = table
                    .columns
                    .iter_mut()
                    .find(|c| c.name.eq_ignore_ascii_case(&col_spec.name))
                    .ok_or_else(|| DbError::column_not_found(&col_spec.name))?;
                col.primary_key = true;
                col.nullable = false;
                col.is_clustered = is_clustered;
            }
            if columns.len() == 1 {
                if let Some(col) = table
                    .columns
                    .iter_mut()
                    .find(|c| c.name.eq_ignore_ascii_case(&columns[0].name))
                {
                    col.unique = true;
                }
            }
        }
        TableConstraintSpec::Unique {
            name: _,
            columns,
            is_clustered,
        } => {
            for col_spec in &columns {
                let col = table
                    .columns
                    .iter_mut()
                    .find(|c| c.name.eq_ignore_ascii_case(&col_spec.name))
                    .ok_or_else(|| DbError::column_not_found(&col_spec.name))?;
                col.unique = true;
                col.is_clustered = is_clustered;
            }
        }
        TableConstraintSpec::SystemTimePeriod { .. }
        | TableConstraintSpec::SystemVersioning { .. } => {}
    }
    Ok(())
}

fn build_column_def(
    executor: &mut SchemaExecutor<'_>,
    spec: crate::ast::ColumnSpec,
) -> Result<ColumnDef, DbError> {
    let is_rowversion = matches!(spec.data_type, crate::ast::DataTypeSpec::RowVersion);
    let (data_type, udt_nullable) =
        resolve_data_type_spec_with_nullability(&spec.data_type, executor.catalog);
    let nullable = if spec.nullable_explicit {
        spec.nullable
    } else if is_rowversion {
        false
    } else if let Some(udt_null) = udt_nullable {
        udt_null
    } else {
        executor.session_options.ansi_null_dflt_on
    };
    Ok(ColumnDef {
        id: executor.catalog.alloc_column_id(),
        name: spec.name,
        data_type,
        nullable,
        primary_key: spec.primary_key,
        unique: spec.unique || spec.primary_key,
        identity: spec.identity.map(|(seed, inc)| IdentityDef::new(seed, inc)),
        default: spec.default,
        default_constraint_name: spec.default_constraint_name,
        check: spec.check,
        check_constraint_name: spec.check_constraint_name,
        computed_expr: spec.computed_expr,
        collation: spec.collation,
        is_clustered: spec.is_clustered,
        ansi_padding_on: executor.session_options.ansi_padding,
        is_rowversion,
    })
}

pub(crate) fn create_type(
    executor: &mut SchemaExecutor<'_>,
    stmt: CreateTypeStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo().to_string();
    if executor.catalog.get_schema_id(&schema).is_none() {
        return Err(DbError::schema_not_found(schema));
    }
    let object_id = executor.catalog.alloc_object_id();
    executor.catalog.create_table_type(TableTypeDef {
        object_id,
        schema,
        name: stmt.name.name,
        columns: stmt.columns,
        table_constraints: stmt.table_constraints,
    })
}

pub(crate) fn create_scalar_type(
    executor: &mut SchemaExecutor<'_>,
    stmt: CreateScalarTypeStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo().to_string();
    if executor.catalog.get_schema_id(&schema).is_none() {
        return Err(DbError::schema_not_found(schema));
    }
    let base_type = resolve_data_type_spec(&stmt.base_type, executor.catalog);
    let object_id = executor.catalog.alloc_object_id();
    executor.catalog.create_scalar_type(ScalarTypeDef {
        object_id,
        schema,
        name: stmt.name.name,
        base_type,
        nullable: stmt.nullable,
    })
}

pub(crate) fn drop_type(
    executor: &mut SchemaExecutor<'_>,
    stmt: DropTypeStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo().to_string();
    if executor
        .catalog
        .find_scalar_type(&schema, &stmt.name.name)
        .is_some()
    {
        return executor.catalog.drop_scalar_type(&schema, &stmt.name.name);
    }
    executor.catalog.drop_table_type(&schema, &stmt.name.name)
}

pub(crate) fn create_table(
    executor: &mut SchemaExecutor<'_>,
    stmt: CreateTableStmt,
) -> Result<(), DbError> {
    let schema_name = stmt.name.schema_or_dbo().to_string();
    let schema_id = executor
        .catalog
        .get_schema_id(&schema_name)
        .ok_or_else(|| DbError::schema_not_found(schema_name.clone()))?;

    if executor
        .catalog
        .find_table(&schema_name, &stmt.name.name)
        .is_some()
    {
        return Err(DbError::duplicate_table(&schema_name, &stmt.name.name));
    }

    let table_id = executor.catalog.alloc_table_id();

    let mut column_fks: Vec<(String, crate::ast::ForeignKeyRef)> = Vec::new();
    let mut column_masks: Vec<(String, String)> = Vec::new();
    for spec in &stmt.columns {
        if let Some(fk) = &spec.foreign_key {
            column_fks.push((spec.name.clone(), fk.clone()));
        }
        if let Some(mask_fn) = &spec.masking_function {
            column_masks.push((spec.name.clone(), mask_fn.clone()));
        }
    }

    let columns = stmt
        .columns
        .into_iter()
        .map(|spec| build_column_def(executor, spec))
        .collect::<Result<Vec<_>, _>>()?;

    let mut table = TableDef {
        id: table_id,
        schema_id,
        schema_name: schema_name.clone(),
        name: stmt.name.name,
        columns,
        check_constraints: vec![],
        foreign_keys: vec![],
    };

    for (col_name, fk_ref) in column_fks {
        table.foreign_keys.push(ForeignKeyDef {
            name: format!("FK__{}__{}", table.name, col_name),
            columns: vec![col_name],
            referenced_table: fk_ref.referenced_table,
            referenced_columns: fk_ref.referenced_columns,
            on_delete: fk_ref
                .on_delete
                .unwrap_or(crate::ast::ReferentialAction::NoAction),
            on_update: fk_ref
                .on_update
                .unwrap_or(crate::ast::ReferentialAction::NoAction),
            enabled: true,
            is_not_trusted: false,
        });
    }

    let mut period_cols: Option<(String, String)> = None;
    let mut sys_versioning: Option<(bool, Option<crate::ast::ObjectName>)> = None;
    for tc in stmt.table_constraints {
        match tc {
            TableConstraintSpec::SystemTimePeriod { start_col, end_col } => {
                period_cols = Some((start_col, end_col));
            }
            TableConstraintSpec::SystemVersioning {
                enabled,
                history_table,
            } => {
                sys_versioning = Some((enabled, history_table));
            }
            other => apply_table_constraint(&mut table, other)?,
        }
    }

    executor.catalog.register_table(table.clone());
    for (col_name, mask_fn) in column_masks {
        executor
            .catalog
            .set_column_mask(table_id, &col_name, Some(mask_fn));
    }
    executor.storage.ensure_table(table_id)?;

    if let Some((start_col, end_col)) = period_cols {
        let (enabled, hist_obj) = sys_versioning.unwrap_or((false, None));
        let (hist_schema, hist_name) = match hist_obj {
            Some(obj) => (obj.schema_or_dbo().to_string(), obj.name),
            None => (
                schema_name.clone(),
                format!("MSSQL_TemporalHistoryFor_{}", table_id),
            ),
        };
        if enabled {
            crate::executor::temporal::ensure_history_table(
                executor.catalog,
                executor.storage,
                &table,
                &hist_schema,
                &hist_name,
            )?;
        }
        executor
            .catalog
            .set_temporal_config(crate::catalog::TemporalTableConfig {
                table_id,
                start_col,
                end_col,
                history_schema: hist_schema,
                history_table: hist_name,
                system_versioning: enabled,
            });
    }

    let pk_cols: Vec<&crate::catalog::ColumnDef> =
        table.columns.iter().filter(|c| c.primary_key).collect();
    if !pk_cols.is_empty() {
        let index_name = if pk_cols.len() == 1 {
            format!("PK__{}__{}", table.name, pk_cols[0].name)
        } else {
            format!("PK__{}", table.name)
        };
        let col_names: Vec<String> = pk_cols.iter().map(|c| c.name.clone()).collect();
        let is_clustered = pk_cols.iter().any(|c| c.is_clustered);
        executor
            .catalog
            .create_index_with_options(
                "dbo",
                &index_name,
                &table.schema_name,
                &table.name,
                &col_names,
                is_clustered,
                true,
            )
            .map_err(|e| DbError::Execution(format!("Failed to create constraint index: {}", e)))?;
    }
    for col in &table.columns {
        if !col.primary_key && col.unique && col.is_clustered {
            let index_name = format!("UQ__{}__{}", table.name, col.name);
            executor
                .catalog
                .create_index_with_options(
                    "dbo",
                    &index_name,
                    &table.schema_name,
                    &table.name,
                    std::slice::from_ref(&col.name),
                    col.is_clustered,
                    true,
                )
                .map_err(|e| {
                    DbError::Execution(format!("Failed to create constraint index: {}", e))
                })?;
        }
    }

    Ok(())
}

pub(crate) fn drop_table(
    executor: &mut SchemaExecutor<'_>,
    stmt: DropTableStmt,
) -> Result<(), DbError> {
    let schema_name = stmt.name.schema_or_dbo().to_string();
    if let Some(dep) = crate::executor::metadata::sys::dependencies::find_schemabound_dependent(
        executor.catalog,
        &schema_name,
        &stmt.name.name,
    ) {
        return Err(DbError::Execution(format!(
            "Cannot DROP TABLE '{}.{}' because it is being referenced by object '{}'.",
            schema_name, stmt.name.name, dep
        )));
    }
    let table_id = executor.catalog.drop_table(&schema_name, &stmt.name.name)?;
    executor.catalog.remove_table_masks(table_id);
    executor.storage.remove_table(table_id)?;
    Ok(())
}

pub(crate) fn create_schema(
    executor: &mut SchemaExecutor<'_>,
    stmt: CreateSchemaStmt,
) -> Result<(), DbError> {
    executor.catalog.create_schema(&stmt.name)
}

pub(crate) fn drop_schema(
    executor: &mut SchemaExecutor<'_>,
    stmt: DropSchemaStmt,
) -> Result<(), DbError> {
    executor.catalog.drop_schema(&stmt.name)
}

pub(crate) fn alter_schema_transfer(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::AlterSchemaTransferStmt,
) -> Result<(), DbError> {
    let src_schema = stmt.source.schema_or_dbo();
    executor.catalog.transfer_object(
        &stmt.target_schema,
        stmt.entity_type.as_deref(),
        src_schema,
        &stmt.source.name,
    )
}

pub(crate) fn create_view(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::CreateViewStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo().to_string();
    let view_name = stmt.name.name;
    let query = stmt.query;
    let schema_id = executor.catalog.get_schema_id(&schema).unwrap_or(1);
    let select_stmt = crate::ast::Statement::Dml(crate::ast::DmlStatement::Select(Box::new(query)));
    let view_stmt = if stmt.ctes.is_empty() {
        select_stmt
    } else {
        crate::ast::Statement::WithCte(Box::new(crate::ast::WithCteStmt {
            recursive: false,
            ctes: stmt.ctes,
            body: Box::new(select_stmt),
        }))
    };
    let view = crate::catalog::ViewDef {
        object_id: executor.catalog.alloc_object_id(),
        schema,
        name: view_name,
        schema_id,
        query: std::sync::Arc::new(view_stmt),
        definition_sql: String::new(),
        is_schema_bound: stmt.is_schema_bound,
        with_check_option: stmt.with_check_option,
    };
    if stmt.or_alter {
        executor.catalog.replace_view(view)
    } else {
        executor.catalog.create_view(view)
    }
}

pub(crate) fn alter_sequence(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::AlterSequenceStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo();
    executor.catalog.alter_sequence(
        schema,
        &stmt.name.name,
        stmt.restart,
        stmt.increment,
        stmt.minimum_value,
        stmt.maximum_value,
        stmt.is_cycling,
    )
}

pub(crate) fn drop_view(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::DropViewStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo();
    if let Some(dep) = crate::executor::metadata::sys::dependencies::find_schemabound_dependent(
        executor.catalog,
        schema,
        &stmt.name.name,
    ) {
        return Err(DbError::Execution(format!(
            "Cannot DROP VIEW '{}.{}' because it is being referenced by object '{}'.",
            schema, stmt.name.name, dep
        )));
    }
    executor.catalog.drop_view(schema, &stmt.name.name)
}

pub(crate) fn create_synonym(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::CreateSynonymStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo().to_string();
    let object_id = executor.catalog.alloc_object_id();
    executor.catalog.create_synonym(crate::catalog::SynonymDef {
        object_id,
        schema,
        name: stmt.name.name,
        base_object: stmt.base_object,
    })
}

pub(crate) fn drop_synonym(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::DropSynonymStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo();
    executor.catalog.drop_synonym(schema, &stmt.name.name)
}

pub(crate) fn create_sequence(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::CreateSequenceStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo().to_string();
    let data_type = resolve_data_type_spec(&stmt.data_type, executor.catalog);
    let object_id = executor.catalog.alloc_object_id();
    executor
        .catalog
        .create_sequence(crate::catalog::SequenceDef {
            object_id,
            schema,
            name: stmt.name.name,
            data_type,
            start_value: stmt.start_value,
            increment: stmt.increment,
            current_value: crate::catalog::SeqCounter::new(stmt.start_value),
            minimum_value: stmt.minimum_value,
            maximum_value: stmt.maximum_value,
            is_cycling: stmt.is_cycling,
        })
}

pub(crate) fn drop_sequence(
    executor: &mut SchemaExecutor<'_>,
    stmt: crate::ast::DropSequenceStmt,
) -> Result<(), DbError> {
    let schema = stmt.name.schema_or_dbo();
    executor.catalog.drop_sequence(schema, &stmt.name.name)
}

pub(crate) fn create_index(
    executor: &mut SchemaExecutor<'_>,
    stmt: CreateIndexStmt,
) -> Result<(), DbError> {
    let index_schema = stmt.name.schema_or_dbo().to_string();
    let table_schema = stmt.table.schema_or_dbo().to_string();

    let col_names: Vec<String> = stmt.columns.iter().map(|c| c.name.clone()).collect();
    executor.catalog.create_index_with_filter(
        &index_schema,
        &stmt.name.name,
        &table_schema,
        &stmt.table.name,
        &col_names,
        stmt.is_clustered,
        stmt.is_unique,
        stmt.filter_predicate,
    )?;

    if let (Some(index_def_id), Some(table)) = (
        executor
            .catalog
            .get_indexes()
            .iter()
            .find(|idx| idx.name.eq_ignore_ascii_case(&stmt.name.name))
            .map(|idx| idx.id),
        executor.catalog.find_table(&table_schema, &stmt.table.name),
    ) {
        schema_physical::rebuild_index_for_table(executor, index_def_id, table.id);
    }

    Ok(())
}

pub(crate) fn drop_index(
    executor: &mut SchemaExecutor<'_>,
    stmt: DropIndexStmt,
) -> Result<(), DbError> {
    let index_schema = stmt.name.schema_or_dbo().to_string();
    let table_schema = stmt.table.schema_or_dbo().to_string();
    executor.catalog.drop_index(
        &index_schema,
        &stmt.name.name,
        &table_schema,
        &stmt.table.name,
    )
}

pub(crate) fn alter_table(
    executor: &mut SchemaExecutor<'_>,
    stmt: AlterTableStmt,
) -> Result<(), DbError> {
    let schema_name = stmt.table.schema_or_dbo().to_string();
    if let AlterTableAction::ToggleTrigger {
        trigger_name,
        enable,
    } = stmt.action
    {
        let (t_sch, t_name) = match &trigger_name {
            Some(obj) => (obj.schema.as_deref(), Some(obj.name.as_str())),
            None => (None, None),
        };
        if !stmt.table.name.eq_ignore_ascii_case("__DATABASE__")
            && executor
                .catalog
                .find_table(&schema_name, &stmt.table.name)
                .is_none()
        {
            return Err(DbError::table_not_found(&schema_name, &stmt.table.name));
        }
        return executor.catalog.set_trigger_enabled(
            t_sch,
            t_name,
            &schema_name,
            &stmt.table.name,
            enable,
        );
    }
    let table_id = executor
        .catalog
        .find_table(&schema_name, &stmt.table.name)
        .map(|t| t.id)
        .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;

    match stmt.action {
        AlterTableAction::AddColumn(col_spec) => {
            let mask_fn = col_spec.masking_function.clone();
            let col_name = col_spec.name.clone();
            let col = build_column_def(executor, col_spec)?;
            let table_mut = executor
                .catalog
                .find_table_mut(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;
            table_mut.columns.push(col);
            if let Some(mask) = mask_fn {
                executor
                    .catalog
                    .set_column_mask(table_id, &col_name, Some(mask));
            }

            schema_physical::add_null_column_to_table(executor, table_id)?;
        }
        AlterTableAction::SetColumnMask {
            column,
            masking_function,
        } => {
            let exists = executor
                .catalog
                .find_table(&schema_name, &stmt.table.name)
                .is_some_and(|t| {
                    t.columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(&column))
                });
            if !exists {
                return Err(DbError::column_not_found(&column));
            }
            executor
                .catalog
                .set_column_mask(table_id, &column, masking_function);
        }
        AlterTableAction::DropColumn(col_name) => {
            if let Some(dep) =
                crate::executor::metadata::sys::dependencies::find_schemabound_column_dependent(
                    executor.catalog,
                    &schema_name,
                    &stmt.table.name,
                    &col_name,
                )
            {
                return Err(DbError::Execution(format!(
                    "The object '{}' is dependent on column '{}'. ALTER TABLE DROP COLUMN {} failed because one or more objects access this column.",
                    dep, col_name, col_name
                )));
            }
            let table_mut = executor
                .catalog
                .find_table_mut(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;
            let col_idx = table_mut
                .columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(&col_name))
                .ok_or_else(|| DbError::column_not_found(&col_name))?;
            table_mut.columns.remove(col_idx);
            executor.catalog.set_column_mask(table_id, &col_name, None);

            schema_physical::drop_column_from_table(executor, table_id, col_idx)?;
        }
        AlterTableAction::AlterColumn {
            name,
            data_type,
            nullable,
        } => {
            if let Some(dep) =
                crate::executor::metadata::sys::dependencies::find_schemabound_column_dependent(
                    executor.catalog,
                    &schema_name,
                    &stmt.table.name,
                    &name,
                )
            {
                return Err(DbError::Execution(format!(
                    "The object '{}' is dependent on column '{}'. ALTER TABLE ALTER COLUMN {} failed because one or more objects access this column.",
                    dep, name, name
                )));
            }
            let resolved_dt = resolve_data_type_spec(&data_type, executor.catalog);
            let table_mut = executor
                .catalog
                .find_table_mut(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;
            let col = table_mut
                .columns
                .iter_mut()
                .find(|c| c.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| DbError::column_not_found(&name))?;
            col.data_type = resolved_dt;
            if let Some(n) = nullable {
                col.nullable = n;
            }
        }
        AlterTableAction::AddConstraint {
            constraint,
            with_check,
        } => {
            let table_mut = executor
                .catalog
                .find_table_mut(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;
            let prev_chk_len = table_mut.check_constraints.len();
            let prev_fk_len = table_mut.foreign_keys.len();
            apply_table_constraint(table_mut, constraint)?;
            if with_check == Some(false) {
                for chk in &mut table_mut.check_constraints[prev_chk_len..] {
                    chk.is_not_trusted = true;
                }
                for fk in &mut table_mut.foreign_keys[prev_fk_len..] {
                    fk.is_not_trusted = true;
                }
            }
        }
        AlterTableAction::CheckConstraint {
            check,
            with_check,
            name,
        } => {
            let table_mut = executor
                .catalog
                .find_table_mut(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;
            for fk in &mut table_mut.foreign_keys {
                if name
                    .as_ref()
                    .is_none_or(|n| fk.name.eq_ignore_ascii_case(n))
                {
                    fk.enabled = check;
                    if !check {
                        fk.is_not_trusted = true;
                    } else if with_check == Some(true) {
                        fk.is_not_trusted = false;
                    }
                }
            }
            for chk in &mut table_mut.check_constraints {
                if name
                    .as_ref()
                    .is_none_or(|n| chk.name.eq_ignore_ascii_case(n))
                {
                    chk.enabled = check;
                    if !check {
                        chk.is_not_trusted = true;
                    } else if with_check == Some(true) {
                        chk.is_not_trusted = false;
                    }
                }
            }
        }
        AlterTableAction::ToggleTrigger {
            trigger_name,
            enable,
        } => {
            let (t_sch, t_name) = match &trigger_name {
                Some(obj) => (obj.schema.as_deref(), Some(obj.name.as_str())),
                None => (None, None),
            };
            executor.catalog.set_trigger_enabled(
                t_sch,
                t_name,
                &schema_name,
                &stmt.table.name,
                enable,
            )?;
        }
        AlterTableAction::SetSystemVersioning {
            enabled,
            history_table,
        } => {
            let table_snapshot = executor
                .catalog
                .find_table(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?
                .clone();
            if let Some(mut cfg) = executor.catalog.get_temporal_config(table_id).cloned() {
                cfg.system_versioning = enabled;
                if let Some(hist) = history_table {
                    cfg.history_schema = hist.schema_or_dbo().to_string();
                    cfg.history_table = hist.name;
                }
                if enabled {
                    crate::executor::temporal::ensure_history_table(
                        executor.catalog,
                        executor.storage,
                        &table_snapshot,
                        &cfg.history_schema,
                        &cfg.history_table,
                    )?;
                }
                executor.catalog.set_temporal_config(cfg);
            }
        }
        AlterTableAction::DropConstraint(constraint_name) => {
            let table_mut = executor
                .catalog
                .find_table_mut(&schema_name, &stmt.table.name)
                .ok_or_else(|| DbError::table_not_found(&schema_name, &stmt.table.name))?;

            let removed = table_mut
                .check_constraints
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(&constraint_name));
            if let Some(pos) = removed {
                table_mut.check_constraints.remove(pos);
            } else {
                let removed = table_mut
                    .foreign_keys
                    .iter()
                    .position(|fk| fk.name.eq_ignore_ascii_case(&constraint_name));
                if let Some(pos) = removed {
                    table_mut.foreign_keys.remove(pos);
                } else {
                    return Err(DbError::constraint_not_found(
                        &stmt.table.name,
                        constraint_name,
                    ));
                }
            }
        }
    }

    Ok(())
}
