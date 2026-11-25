use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::Arc;

use super::{EngineCatalog, EngineStorage};
use crate::error::DbError;
use crate::executor::database::SessionManagerService;
use crate::executor::journal::Journal;
use crate::executor::locks::SessionId;
use crate::executor::session::{SessionManager, SessionRuntime};
use crate::executor::string_norm::normalize_identifier;

impl<C, S> SessionManager for SessionManagerService<C, S>
where
    C: EngineCatalog,
    S: EngineStorage,
{
    fn create_session(&self) -> SessionId {
        let id = self
            .state
            .next_session_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut runtime = SessionRuntime::new();
        runtime.extras.app_locks = self.state.app_locks.clone();
        self.state
            .sessions
            .write()
            .insert(id, Arc::new(Mutex::new(runtime)));
        self.state.deadlock_priorities.write().insert(id, 0);
        id
    }

    fn reset_session(&self, session_id: SessionId) -> Result<(), DbError> {
        self.state.app_locks.release_all_for_session(session_id);
        let session_mutex = self
            .state
            .get_session(session_id)
            .ok_or_else(|| DbError::Execution(format!("session {} not found", session_id)))?;
        let mut session = session_mutex.lock();
        let mut physical_tables = HashSet::new();
        for table in session.tables.temp_map.values() {
            physical_tables.insert(table.clone());
        }
        for table in session.tables.var_map.values() {
            physical_tables.insert(table.clone());
        }
        session.reset();
        self.state
            .deadlock_priorities
            .write()
            .insert(session_id, session.options.deadlock_priority);
        drop(session);

        self.state
            .table_locks
            .lock()
            .release_all_for_session(session_id);

        if !physical_tables.is_empty() {
            let mut storage = self.state.storage.write();
            for table_name in physical_tables {
                let Some(table) = storage
                    .catalog
                    .get_tables()
                    .iter()
                    .find(|table| table.name.eq_ignore_ascii_case(&table_name))
                    .cloned()
                else {
                    continue;
                };

                let schema_name = table.schema_name.clone();
                let _ = storage.catalog.drop_table(&schema_name, &table_name);
                storage.storage.remove_table(table.id)?;
                storage.table_versions.remove(&format!(
                    "{}.{}",
                    normalize_identifier(&schema_name),
                    normalize_identifier(&table_name)
                ));
            }
        }
        Ok(())
    }

    fn close_session(&self, session_id: SessionId) -> Result<(), DbError> {
        self.state.app_locks.release_all_for_session(session_id);
        self.state
            .table_locks
            .lock()
            .release_all_for_session(session_id);
        self.state.deadlock_priorities.write().remove(&session_id);
        let removed = self.state.sessions.write().remove(&session_id);
        if removed.is_none() {
            return Err(DbError::Execution(format!(
                "session {} not found",
                session_id
            )));
        }
        Ok(())
    }

    fn set_session_journal(
        &self,
        session_id: SessionId,
        journal: Box<dyn Journal>,
    ) -> Result<(), DbError> {
        let session_mutex = self
            .state
            .get_session(session_id)
            .ok_or_else(|| DbError::Execution(format!("session {} not found", session_id)))?;
        let mut session = session_mutex.lock();
        session.journal = journal;
        Ok(())
    }

    fn session_cancel_flag(
        &self,
        session_id: SessionId,
    ) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        let session_mutex = self.state.get_session(session_id)?;
        let session = session_mutex.lock();
        Some(session.extras.cancel_flag.clone())
    }

    fn take_print_output(&self, session_id: SessionId) -> Vec<String> {
        self.state
            .get_session(session_id)
            .map(|s| std::mem::take(&mut s.lock().diagnostics.print_output))
            .unwrap_or_default()
    }
}
