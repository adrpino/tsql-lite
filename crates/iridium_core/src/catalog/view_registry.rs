use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

impl ViewRegistry for CatalogImpl {
    fn get_views(&self) -> &[Arc<ViewDef>] {
        &self.views
    }

    fn find_view(&self, schema: &str, name: &str) -> Option<&ViewDef> {
        let idx = self
            .view_map
            .get(&(normalize_identifier(schema), normalize_identifier(name)))?;
        Some(&self.views[*idx])
    }

    fn create_view(&mut self, view: ViewDef) -> Result<(), DbError> {
        if self.find_view(&view.schema, &view.name).is_some() {
            return Err(DbError::duplicate_view(&view.schema, &view.name));
        }
        let idx = self.views.len();
        Arc::make_mut(&mut self.view_map).insert(
            (
                normalize_identifier(&view.schema),
                normalize_identifier(&view.name),
            ),
            idx,
        );
        Arc::make_mut(&mut self.views).push(Arc::new(view));
        Ok(())
    }

    fn drop_view(&mut self, schema: &str, name: &str) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let idx = *self
            .view_map
            .get(&key)
            .ok_or_else(|| DbError::view_not_found(schema, name))?;
        let views = Arc::make_mut(&mut self.views);
        views.swap_remove(idx);
        let view_map = Arc::make_mut(&mut self.view_map);
        view_map.remove(&key);
        if idx < views.len() {
            let swapped = &views[idx];
            view_map.insert(
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
