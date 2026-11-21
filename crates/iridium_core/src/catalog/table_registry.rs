use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

fn table_key(t: &TableDef) -> (u32, String) {
    (t.schema_id, normalize_identifier(&t.name))
}

impl TableRegistry for CatalogImpl {
    fn get_tables(&self) -> CowRegistryView<'_, TableDef> {
        self.tables.view()
    }

    fn find_table(&self, schema: &str, name: &str) -> Option<&TableDef> {
        let schema_id = self.get_schema_id(schema)?;
        self.tables.get(&(schema_id, normalize_identifier(name)))
    }

    fn find_table_arc(&self, schema: &str, name: &str) -> Option<Arc<TableDef>> {
        let schema_id = self.get_schema_id(schema)?;
        self.tables
            .get_arc(&(schema_id, normalize_identifier(name)))
    }

    fn find_table_mut(&mut self, schema: &str, name: &str) -> Option<&mut TableDef> {
        let schema_id = self.get_schema_id(schema)?;
        self.tables
            .get_mut(&(schema_id, normalize_identifier(name)))
    }

    fn register_table(&mut self, table: TableDef) {
        let key = table_key(&table);
        self.tables.insert_or_replace(key, table);
    }

    fn unregister_table_by_id(&mut self, id: u32) {
        if let Some(removed) = self.tables.remove_by(|t| t.id == id, table_key) {
            self.remove_indexes_for_table_id(removed.id);
        }
    }

    fn drop_table(&mut self, schema: &str, name: &str) -> Result<u32, DbError> {
        let schema_id = self
            .get_schema_id(schema)
            .ok_or_else(|| DbError::schema_not_found(schema))?;

        let key = (schema_id, normalize_identifier(name));
        let removed = self
            .tables
            .remove(&key, table_key)
            .ok_or_else(|| DbError::table_not_found(schema, name))?;

        let table_id = removed.id;
        self.remove_indexes_for_table_id(table_id);
        Ok(table_id)
    }

    fn next_identity_value(&mut self, table_id: u32, column_name: &str) -> Result<i64, DbError> {
        let table = self
            .tables
            .find_mut_by(|t| t.id == table_id, table_key)
            .ok_or_else(|| DbError::object_not_found(format!("table ID {}", table_id)))?;

        let col = table
            .columns
            .iter_mut()
            .find(|c| c.name.eq_ignore_ascii_case(column_name))
            .ok_or_else(|| DbError::column_not_found(column_name))?;

        if let Some(identity) = &mut col.identity {
            Ok(identity.next_value())
        } else {
            Err(DbError::Execution(format!(
                "column '{}' is not an IDENTITY column",
                column_name
            )))
        }
    }
}
