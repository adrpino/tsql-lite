use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

impl SchemaRegistry for CatalogImpl {
    fn get_schemas(&self) -> CowRegistryView<'_, SchemaDef> {
        self.schemas.view()
    }

    fn get_schema_id(&self, name: &str) -> Option<u32> {
        self.schemas.get(&normalize_identifier(name)).map(|s| s.id)
    }

    fn create_schema(&mut self, name: &str) -> Result<(), DbError> {
        if self.get_schema_id(name).is_some() {
            return Err(DbError::duplicate_schema(name));
        }
        let id = self.alloc_schema_id();
        self.schemas.insert_or_replace(
            normalize_identifier(name),
            SchemaDef {
                id,
                name: name.to_string(),
            },
        );
        Ok(())
    }

    fn drop_schema(&mut self, name: &str) -> Result<(), DbError> {
        let schema_id = self
            .get_schema_id(name)
            .ok_or_else(|| DbError::schema_not_found(name))?;

        let has_tables = self.tables.iter().any(|t| t.schema_id == schema_id);
        if has_tables {
            return Err(DbError::Execution(format!(
                "schema '{}' cannot be dropped because it contains tables",
                name
            )));
        }

        self.schemas.remove(&normalize_identifier(name), |s| {
            normalize_identifier(&s.name)
        });
        Ok(())
    }

    fn transfer_object(
        &mut self,
        target_schema: &str,
        entity_type: Option<&str>,
        source_schema: &str,
        source_name: &str,
    ) -> Result<(), DbError> {
        let target_schema_id = self
            .get_schema_id(target_schema)
            .ok_or_else(|| DbError::schema_not_found(target_schema))?;
        let target_schema_name = self
            .schemas
            .iter()
            .find(|s| s.id == target_schema_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| target_schema.to_string());
        let src_schema_id = self
            .get_schema_id(source_schema)
            .ok_or_else(|| DbError::schema_not_found(source_schema))?;

        let norm_src = normalize_identifier(source_schema);
        let norm_dst = normalize_identifier(&target_schema_name);
        let norm_name = normalize_identifier(source_name);
        let same_schema = target_schema_id == src_schema_id;
        let is_type_only = entity_type
            .map(|e| e.eq_ignore_ascii_case("TYPE"))
            .unwrap_or(false);

        if is_type_only {
            let src_key = (norm_src.clone(), norm_name.clone());
            let dst_key = (norm_dst.clone(), norm_name.clone());
            if let Some(&idx) = self.type_map.get(&src_key) {
                if same_schema {
                    return Ok(());
                }
                if self.type_map.contains_key(&dst_key)
                    || self.scalar_type_map.contains_key(&dst_key)
                {
                    return Err(DbError::DuplicateType {
                        schema: target_schema_name,
                        type_name: source_name.to_string(),
                    });
                }
                let types = Arc::make_mut(&mut self.table_types);
                types[idx].schema = target_schema_name;
                self.type_map = Arc::new(
                    types
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            (
                                (
                                    normalize_identifier(&t.schema),
                                    normalize_identifier(&t.name),
                                ),
                                i,
                            )
                        })
                        .collect(),
                );
                return Ok(());
            }
            if let Some(idx) = self.scalar_type_map.get(&src_key).copied() {
                if same_schema {
                    return Ok(());
                }
                if self.scalar_type_map.contains_key(&dst_key)
                    || self.type_map.contains_key(&dst_key)
                {
                    return Err(DbError::DuplicateType {
                        schema: target_schema_name,
                        type_name: source_name.to_string(),
                    });
                }
                let types = Arc::make_mut(&mut self.scalar_types);
                types[idx].schema = target_schema_name;
                self.scalar_type_map = Arc::new(
                    types
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            (
                                (
                                    normalize_identifier(&t.schema),
                                    normalize_identifier(&t.name),
                                ),
                                i,
                            )
                        })
                        .collect(),
                );
                return Ok(());
            }
            return Err(DbError::ObjectNotFound {
                object: format!("{source_schema}.{source_name}"),
            });
        }

        // 1. Table (preserves table_id so storage rows remain intact; also moves indexes & triggers)
        let table_src_key = (src_schema_id, norm_name.clone());
        if self.tables.contains_key(&table_src_key) {
            if same_schema {
                return Ok(());
            }
            let table_dst_key = (target_schema_id, norm_name.clone());
            if self.tables.contains_key(&table_dst_key) {
                return Err(DbError::DuplicateTable {
                    schema: target_schema_name,
                    table: source_name.to_string(),
                });
            }
            let Some(table_arc) = self.tables.remove(&table_src_key, |t| {
                (t.schema_id, normalize_identifier(&t.name))
            }) else {
                return Err(DbError::ObjectNotFound {
                    object: format!("{source_schema}.{source_name}"),
                });
            };
            let mut moved = (*table_arc).clone();
            let table_id = moved.id;
            moved.schema_id = target_schema_id;
            moved.schema_name = target_schema_name.clone();
            self.tables.insert_or_replace(table_dst_key, moved);

            // Update indexes attached to this table via O(1) CoW key move.
            let attached_idx_keys: Vec<(u32, u32, String)> = self
                .indexes
                .iter()
                .filter(|i| i.table_id == table_id)
                .map(|i| (i.schema_id, i.table_id, normalize_identifier(&i.name)))
                .collect();
            for old_key in attached_idx_keys {
                if let Some(idx_arc) = self.indexes.remove(&old_key, |i| {
                    (i.schema_id, i.table_id, normalize_identifier(&i.name))
                }) {
                    let mut idx_def = (*idx_arc).clone();
                    idx_def.schema_id = target_schema_id;
                    let new_key = (
                        idx_def.schema_id,
                        idx_def.table_id,
                        normalize_identifier(&idx_def.name),
                    );
                    self.indexes.insert_or_replace(new_key, idx_def);
                }
            }

            // Move triggers attached to this table.
            let attached_keys: Vec<(String, String)> = self
                .triggers
                .iter()
                .filter(|tr| {
                    normalize_identifier(&tr.table_schema) == norm_src
                        && normalize_identifier(&tr.table_name) == norm_name
                })
                .map(|tr| {
                    (
                        normalize_identifier(&tr.schema),
                        normalize_identifier(&tr.name),
                    )
                })
                .collect();
            for old_key in attached_keys {
                if let Some(tr_arc) = self.triggers.remove(&old_key, |t| {
                    (
                        normalize_identifier(&t.schema),
                        normalize_identifier(&t.name),
                    )
                }) {
                    let mut tr = (*tr_arc).clone();
                    tr.schema = target_schema_name.clone();
                    tr.table_schema = target_schema_name.clone();
                    let new_key = (norm_dst.clone(), normalize_identifier(&tr.name));
                    self.triggers.insert_or_replace(new_key, tr);
                }
            }
            return Ok(());
        }

        // 2. View
        let obj_src_key = (norm_src.clone(), norm_name.clone());
        let obj_dst_key = (norm_dst.clone(), norm_name.clone());
        if self.views.contains_key(&obj_src_key) {
            if same_schema {
                return Ok(());
            }
            if self.views.contains_key(&obj_dst_key) {
                return Err(DbError::DuplicateView {
                    schema: target_schema_name,
                    view: source_name.to_string(),
                });
            }
            if let Some(view_arc) = self.views.remove(&obj_src_key, |v| {
                (
                    normalize_identifier(&v.schema),
                    normalize_identifier(&v.name),
                )
            }) {
                let mut v = (*view_arc).clone();
                v.schema = target_schema_name;
                v.schema_id = target_schema_id;
                self.views.insert_or_replace(obj_dst_key, v);
                return Ok(());
            }
        }

        // 3. Routine (procedure or function)
        if self.routines.contains_key(&obj_src_key) {
            if same_schema {
                return Ok(());
            }
            if self.routines.contains_key(&obj_dst_key) {
                return Err(DbError::Execution(format!(
                    "There is already an object named '{target_schema_name}.{source_name}' in the database."
                )));
            }
            if let Some(r_arc) = self.routines.remove(&obj_src_key, |r| {
                (
                    normalize_identifier(&r.schema),
                    normalize_identifier(&r.name),
                )
            }) {
                let mut r = (*r_arc).clone();
                r.schema = target_schema_name;
                self.routines.insert_or_replace(obj_dst_key, r);
                return Ok(());
            }
        }

        // 4. Synonym
        if let Some(&idx) = self.synonym_map.get(&obj_src_key) {
            if same_schema {
                return Ok(());
            }
            if self.synonym_map.contains_key(&obj_dst_key) {
                return Err(DbError::Execution(format!(
                    "There is already a synonym named '{target_schema_name}.{source_name}'."
                )));
            }
            let syns = Arc::make_mut(&mut self.synonyms);
            syns[idx].schema = target_schema_name;
            self.synonym_map = Arc::new(
                syns.iter()
                    .enumerate()
                    .map(|(i, s)| {
                        (
                            (
                                normalize_identifier(&s.schema),
                                normalize_identifier(&s.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
            return Ok(());
        }

        // 5. Sequence
        if let Some(&idx) = self.sequence_map.get(&obj_src_key) {
            if same_schema {
                return Ok(());
            }
            if self.sequence_map.contains_key(&obj_dst_key) {
                return Err(DbError::Execution(format!(
                    "There is already a sequence named '{target_schema_name}.{source_name}'."
                )));
            }
            let seqs = Arc::make_mut(&mut self.sequences);
            seqs[idx].schema = target_schema_name;
            self.sequence_map = Arc::new(
                seqs.iter()
                    .enumerate()
                    .map(|(i, s)| {
                        (
                            (
                                normalize_identifier(&s.schema),
                                normalize_identifier(&s.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
            return Ok(());
        }

        // 6. Fallback: Table Type without explicit TYPE:: class
        if let Some(&idx) = self.type_map.get(&obj_src_key) {
            if same_schema {
                return Ok(());
            }
            if self.type_map.contains_key(&obj_dst_key) {
                return Err(DbError::DuplicateType {
                    schema: target_schema_name,
                    type_name: source_name.to_string(),
                });
            }
            let types = Arc::make_mut(&mut self.table_types);
            types[idx].schema = target_schema_name;
            self.type_map = Arc::new(
                types
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        (
                            (
                                normalize_identifier(&t.schema),
                                normalize_identifier(&t.name),
                            ),
                            i,
                        )
                    })
                    .collect(),
            );
            return Ok(());
        }

        Err(DbError::ObjectNotFound {
            object: format!("{source_schema}.{source_name}"),
        })
    }
}
