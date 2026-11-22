use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

fn view_key(v: &ViewDef) -> (String, String) {
    (
        normalize_identifier(&v.schema),
        normalize_identifier(&v.name),
    )
}

impl ViewRegistry for CatalogImpl {
    fn get_views(&self) -> CowRegistryView<'_, ViewDef> {
        self.views.view()
    }

    fn find_view(&self, schema: &str, name: &str) -> Option<&ViewDef> {
        self.views
            .get(&(normalize_identifier(schema), normalize_identifier(name)))
    }

    fn create_view(&mut self, view: ViewDef) -> Result<(), DbError> {
        let key = view_key(&view);
        if self.views.contains_key(&key) {
            return Err(DbError::duplicate_view(&view.schema, &view.name));
        }
        self.views.insert_or_replace(key, view);
        Ok(())
    }

    fn replace_view(&mut self, view: ViewDef) -> Result<(), DbError> {
        let key = view_key(&view);
        self.views.insert_or_replace(key, view);
        Ok(())
    }

    fn drop_view(&mut self, schema: &str, name: &str) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        self.views
            .remove(&key, view_key)
            .ok_or_else(|| DbError::view_not_found(schema, name))?;
        Ok(())
    }
}
