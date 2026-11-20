use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

impl SchemaRegistry for CatalogImpl {
    fn get_schemas(&self) -> &[SchemaDef] {
        &self.schemas
    }

    fn get_schema_id(&self, name: &str) -> Option<u32> {
        let idx = self.schema_map.get(&normalize_identifier(name))?;
        Some(self.schemas[*idx].id)
    }

    fn create_schema(&mut self, name: &str) -> Result<(), DbError> {
        if self.get_schema_id(name).is_some() {
            return Err(DbError::duplicate_schema(name));
        }
        let id = self.alloc_schema_id();
        let idx = self.schemas.len();
        Arc::make_mut(&mut self.schemas).push(SchemaDef {
            id,
            name: name.to_string(),
        });
        Arc::make_mut(&mut self.schema_map).insert(normalize_identifier(name), idx);
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

        let schemas = Arc::make_mut(&mut self.schemas);
        schemas.retain(|s| s.id != schema_id);
        self.schema_map = Arc::new(
            schemas
                .iter()
                .enumerate()
                .map(|(i, s)| (normalize_identifier(&s.name), i))
                .collect(),
        );
        Ok(())
    }
}
