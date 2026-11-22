use super::*;
use crate::error::DbError;
use crate::executor::string_norm::normalize_identifier;

impl SequenceRegistry for CatalogImpl {
    fn get_sequences(&self) -> &[SequenceDef] {
        &self.sequences
    }

    fn find_sequence(&self, schema: &str, name: &str) -> Option<&SequenceDef> {
        let idx = self
            .sequence_map
            .get(&(normalize_identifier(schema), normalize_identifier(name)))?;
        Some(&self.sequences[*idx])
    }

    fn create_sequence(&mut self, sequence: SequenceDef) -> Result<(), DbError> {
        if self
            .find_sequence(&sequence.schema, &sequence.name)
            .is_some()
        {
            return Err(DbError::Execution(format!(
                "sequence '{}' already exists in schema '{}'",
                sequence.name, sequence.schema
            )));
        }

        let idx = self.sequences.len();
        Arc::make_mut(&mut self.sequence_map).insert(
            (
                normalize_identifier(&sequence.schema),
                normalize_identifier(&sequence.name),
            ),
            idx,
        );
        Arc::make_mut(&mut self.sequences).push(sequence);
        Ok(())
    }

    fn alter_sequence(
        &mut self,
        schema: &str,
        name: &str,
        restart: Option<Option<i64>>,
        increment: Option<i64>,
        minimum_value: Option<Option<i64>>,
        maximum_value: Option<Option<i64>>,
        is_cycling: Option<bool>,
    ) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let idx = *self
            .sequence_map
            .get(&key)
            .ok_or_else(|| DbError::object_not_found(format!("{}.{}", schema, name)))?;
        let seq = &mut Arc::make_mut(&mut self.sequences)[idx];
        if let Some(inc) = increment {
            if inc == 0 {
                return Err(DbError::Execution(
                    "The increment value for a sequence object cannot be zero.".into(),
                ));
            }
            seq.increment = inc;
        }
        if let Some(min_opt) = minimum_value {
            seq.minimum_value = min_opt.unwrap_or(i64::MIN);
        }
        if let Some(max_opt) = maximum_value {
            seq.maximum_value = max_opt.unwrap_or(i64::MAX);
        }
        if let Some(cyc) = is_cycling {
            seq.is_cycling = cyc;
        }
        if let Some(restart_val) = restart {
            seq.current_value
                .store(restart_val.unwrap_or(seq.start_value));
        }
        Ok(())
    }

    fn drop_sequence(&mut self, schema: &str, name: &str) -> Result<(), DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let idx = *self
            .sequence_map
            .get(&key)
            .ok_or_else(|| DbError::object_not_found(format!("{}.{}", schema, name)))?;
        let sequences = Arc::make_mut(&mut self.sequences);
        sequences.swap_remove(idx);
        let sequence_map = Arc::make_mut(&mut self.sequence_map);
        sequence_map.remove(&key);
        if idx < sequences.len() {
            let swapped = &sequences[idx];
            sequence_map.insert(
                (
                    normalize_identifier(&swapped.schema),
                    normalize_identifier(&swapped.name),
                ),
                idx,
            );
        }
        Ok(())
    }

    fn next_sequence_value(&mut self, schema: &str, name: &str) -> Result<i64, DbError> {
        self.next_sequence_value_shared(schema, name)
    }

    fn next_sequence_value_shared(&self, schema: &str, name: &str) -> Result<i64, DbError> {
        let key = (normalize_identifier(schema), normalize_identifier(name));
        let idx = *self
            .sequence_map
            .get(&key)
            .ok_or_else(|| DbError::object_not_found(format!("{}.{}", schema, name)))?;

        let seq = &self.sequences[idx];
        loop {
            let val = seq.current_value.load();
            let next_val = if seq.increment >= 0 {
                if val > seq.maximum_value - seq.increment {
                    if seq.is_cycling {
                        seq.minimum_value
                    } else {
                        return Err(DbError::Execution(format!(
                            "sequence '{}' has reached its maximum value",
                            name
                        )));
                    }
                } else {
                    val + seq.increment
                }
            } else if val < seq.minimum_value - seq.increment {
                if seq.is_cycling {
                    seq.maximum_value
                } else {
                    return Err(DbError::Execution(format!(
                        "sequence '{}' has reached its minimum value",
                        name
                    )));
                }
            } else {
                val + seq.increment
            };
            if seq.current_value.compare_exchange(val, next_val).is_ok() {
                return Ok(val);
            }
        }
    }
}
