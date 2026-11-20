use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

impl TriggerRegistry for CatalogImpl {
    fn get_triggers(&self) -> &[TriggerDef] {
        &self.triggers
    }

    fn find_triggers_for_table(&self, schema: &str, name: &str) -> Vec<&TriggerDef> {
        self.triggers
            .iter()
            .filter(|t| {
                t.table_schema.eq_ignore_ascii_case(schema)
                    && t.table_name.eq_ignore_ascii_case(name)
            })
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
        Arc::make_mut(&mut self.triggers).push(trigger);
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
}
