use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

impl TriggerRegistry for CatalogImpl {
    fn get_triggers(&self) -> &[Arc<TriggerDef>] {
        &self.triggers
    }

    fn find_triggers_for_table(&self, schema: &str, name: &str) -> Vec<&TriggerDef> {
        self.triggers
            .iter()
            .filter(|t| {
                t.table_schema.eq_ignore_ascii_case(schema)
                    && t.table_name.eq_ignore_ascii_case(name)
            })
            .map(|t| t.as_ref())
            .collect()
    }

    fn create_trigger(&mut self, trigger: TriggerDef) -> Result<(), DbError> {
        if self.trigger_map.contains_key(&(
            normalize_identifier(&trigger.schema),
            normalize_identifier(&trigger.name),
        )) {
            return Err(DbError::duplicate_trigger(&trigger.schema, &trigger.name));
        }
        let idx = self.triggers.len();
        Arc::make_mut(&mut self.trigger_map).insert(
            (
                normalize_identifier(&trigger.schema),
                normalize_identifier(&trigger.name),
            ),
            idx,
        );
        Arc::make_mut(&mut self.triggers).push(Arc::new(trigger));
        Ok(())
    }

    fn drop_trigger(&mut self, schema: &str, name: &str) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let idx = *self
            .trigger_map
            .get(&key)
            .ok_or_else(|| DbError::trigger_not_found(schema, name))?;
        let triggers = Arc::make_mut(&mut self.triggers);
        triggers.swap_remove(idx);
        let trigger_map = Arc::make_mut(&mut self.trigger_map);
        trigger_map.remove(&key);
        if idx < triggers.len() {
            let swapped = &triggers[idx];
            trigger_map.insert(
                (
                    normalize_identifier(&swapped.schema),
                    normalize_identifier(&swapped.name),
                ),
                idx,
            );
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
        let triggers = Arc::make_mut(&mut self.triggers);
        for t_arc in triggers.iter_mut() {
            let table_match = t_arc.table_schema.eq_ignore_ascii_case(table_schema)
                && t_arc.table_name.eq_ignore_ascii_case(table_name);
            if !table_match {
                continue;
            }
            let should_update = if let Some(tn) = trigger_name {
                let sch_ok = trigger_schema.is_none_or(|s| t_arc.schema.eq_ignore_ascii_case(s));
                sch_ok && t_arc.name.eq_ignore_ascii_case(tn)
            } else {
                true
            };
            if should_update && t_arc.enabled != enabled {
                Arc::make_mut(t_arc).enabled = enabled;
            }
        }
        Ok(())
    }
}
