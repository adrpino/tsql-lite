use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

fn trigger_key(t: &TriggerDef) -> (String, String) {
    (
        normalize_identifier(&t.schema),
        normalize_identifier(&t.name),
    )
}

impl TriggerRegistry for CatalogImpl {
    fn get_triggers(&self) -> CowRegistryView<'_, TriggerDef> {
        self.triggers.view()
    }

    fn find_triggers_for_table(&self, schema: &str, name: &str) -> Vec<&TriggerDef> {
        if self.triggers_by_table.is_empty() {
            return Vec::new();
        }
        let table_key = (normalize_identifier(schema), normalize_identifier(name));
        let Some(keys) = self.triggers_by_table.get(&table_key) else {
            return Vec::new();
        };
        keys.iter()
            .filter_map(|k| self.triggers.get(k))
            .collect()
    }

    fn create_trigger(&mut self, trigger: TriggerDef) -> Result<(), DbError> {
        let key = trigger_key(&trigger);
        if self.triggers.contains_key(&key) {
            return Err(DbError::duplicate_trigger(&trigger.schema, &trigger.name));
        }
        let table_key = if !trigger.is_database_scope {
            Some((
                normalize_identifier(&trigger.table_schema),
                normalize_identifier(&trigger.table_name),
            ))
        } else {
            None
        };
        self.triggers.insert_or_replace(key.clone(), trigger);
        if let Some(tk) = table_key {
            Arc::make_mut(&mut self.triggers_by_table)
                .entry(tk)
                .or_default()
                .push(key);
        }
        Ok(())
    }

    fn replace_trigger(&mut self, trigger: TriggerDef) -> Result<(), DbError> {
        let key = trigger_key(&trigger);
        if let Some(old) = self.triggers.get(&key) {
            if !old.is_database_scope {
                let old_tk = (
                    normalize_identifier(&old.table_schema),
                    normalize_identifier(&old.table_name),
                );
                let map = Arc::make_mut(&mut self.triggers_by_table);
                if let Some(list) = map.get_mut(&old_tk) {
                    list.retain(|k| k != &key);
                    if list.is_empty() {
                        map.remove(&old_tk);
                    }
                }
            }
        }
        let new_tk = if !trigger.is_database_scope {
            Some((
                normalize_identifier(&trigger.table_schema),
                normalize_identifier(&trigger.table_name),
            ))
        } else {
            None
        };
        self.triggers.insert_or_replace(key.clone(), trigger);
        if let Some(tk) = new_tk {
            Arc::make_mut(&mut self.triggers_by_table)
                .entry(tk)
                .or_default()
                .push(key);
        }
        Ok(())
    }

    fn drop_trigger(&mut self, schema: &str, name: &str) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let removed = self
            .triggers
            .remove(&key, trigger_key)
            .ok_or_else(|| DbError::trigger_not_found(schema, name))?;
        if !removed.is_database_scope {
            let old_tk = (
                normalize_identifier(&removed.table_schema),
                normalize_identifier(&removed.table_name),
            );
            let map = Arc::make_mut(&mut self.triggers_by_table);
            if let Some(list) = map.get_mut(&old_tk) {
                list.retain(|k| k != &key);
                if list.is_empty() {
                    map.remove(&old_tk);
                }
            }
        }
        Ok(())
    }

    fn set_trigger_enabled(
        &mut self,
        trigger_schema: Option<&str>,
        trigger_name: Option<&str>,
        table_schema: &str,
        table_name: &str,
        enabled: bool,
    ) -> Result<(), DbError> {
        let matching_keys: Vec<(String, String)> = self
            .triggers
            .view()
            .iter()
            .filter(|t| {
                let table_match = t.table_schema.eq_ignore_ascii_case(table_schema)
                    && t.table_name.eq_ignore_ascii_case(table_name);
                if !table_match || t.enabled == enabled {
                    return false;
                }
                if let Some(tn) = trigger_name {
                    let sch_ok = trigger_schema.is_none_or(|s| t.schema.eq_ignore_ascii_case(s));
                    sch_ok && t.name.eq_ignore_ascii_case(tn)
                } else {
                    true
                }
            })
            .map(|t| trigger_key(t))
            .collect();

        for key in matching_keys {
            if let Some(trig) = self.triggers.get_mut(&key) {
                trig.enabled = enabled;
            }
        }
        Ok(())
    }
}
