use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

fn routine_key(r: &RoutineDef) -> (String, String) {
    (
        normalize_identifier(&r.schema),
        normalize_identifier(&r.name),
    )
}

impl RoutineRegistry for CatalogImpl {
    fn get_routines(&self) -> CowRegistryView<'_, RoutineDef> {
        self.routines.view()
    }

    fn find_routine(&self, schema: &str, name: &str) -> Option<&RoutineDef> {
        self.routines
            .get(&(normalize_identifier(schema), normalize_identifier(name)))
    }

    fn create_routine(&mut self, routine: RoutineDef) -> Result<(), DbError> {
        let key = routine_key(&routine);
        if self.routines.contains_key(&key) {
            return Err(DbError::object_not_found(format!(
                "routine '{}.{}'",
                routine.schema, routine.name
            )));
        }
        self.routines.insert_or_replace(key, routine);
        Ok(())
    }

    fn replace_routine(&mut self, routine: RoutineDef) -> Result<(), DbError> {
        let key = routine_key(&routine);
        self.routines.insert_or_replace(key, routine);
        Ok(())
    }

    fn drop_routine(
        &mut self,
        schema: &str,
        name: &str,
        expect_function: bool,
    ) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let Some(existing) = self.routines.get(&key) else {
            let kind = if expect_function {
                "function"
            } else {
                "procedure"
            };
            return Err(DbError::object_not_found(format!(
                "{} '{}.{}'",
                kind, schema, name
            )));
        };

        let is_function = matches!(existing.kind, RoutineKind::Function { .. });
        if is_function != expect_function {
            return Err(DbError::invalid_identifier(format!(
                "'{}.{}' has different routine kind",
                schema, name
            )));
        }
        self.routines.remove(&key, routine_key);
        Ok(())
    }
}
