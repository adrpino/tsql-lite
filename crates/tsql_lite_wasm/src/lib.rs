use wasm_bindgen::prelude::*;

use tsql_lite_core::{
    parse_batch, parse_sql, CheckpointManager, Database, SessionId, SessionManager, SqlAnalyzer,
    StatementExecutor,
};

#[wasm_bindgen]
pub struct TsqlLiteWasmDb {
    inner: Database,
    default_session: SessionId,
}

impl Default for TsqlLiteWasmDb {
    fn default() -> Self {
        Self::new()
    }
}

impl TsqlLiteWasmDb {
    fn exec_session_inner(&mut self, session_id: u64, sql: &str) -> Result<(), String> {
        let stmt = parse_sql(sql).map_err(|e| e.to_string())?;
        let result = self
            .inner
            .executor()
            .execute_session(session_id, stmt)
            .map_err(|e| e.to_string())?;
        if result.is_some() {
            return Err("exec() received a query statement; use query()".to_string());
        }
        Ok(())
    }

    fn exec_batch_session_inner(&mut self, session_id: u64, sql: &str) -> Result<(), String> {
        let stmts = parse_batch(sql).map_err(|e| e.to_string())?;
        let result = self
            .inner
            .executor()
            .execute_session_batch(session_id, stmts)
            .map_err(|e| e.to_string())?;
        if result.is_some() {
            return Err(
                "exec_batch() ended with a query statement; use query() for SELECT".to_string(),
            );
        }
        Ok(())
    }

    fn query_session_inner(&mut self, session_id: u64, sql: &str) -> Result<String, String> {
        let stmt = parse_sql(sql).map_err(|e| e.to_string())?;
        let result = self
            .inner
            .executor()
            .execute_session(session_id, stmt)
            .map_err(|e| e.to_string())?;
        let result = result.ok_or_else(|| "query() expected a SELECT statement".to_string())?;
        serde_json::to_string(&result.to_json_result()).map_err(|e| e.to_string())
    }

    fn explain_sql_inner(&self, sql: &str) -> Result<String, String> {
        let plan = self
            .inner
            .analyzer()
            .explain_sql(sql)
            .map_err(|e| e.to_string())?;
        serde_json::to_string(&plan).map_err(|e| e.to_string())
    }

    fn export_checkpoint_inner(&self) -> Result<String, String> {
        self.inner
            .checkpoint_manager()
            .export_checkpoint()
            .map_err(|e| e.to_string())
    }

    fn import_checkpoint_inner(&mut self, payload: &str) -> Result<(), String> {
        self.inner
            .checkpoint_manager()
            .import_checkpoint(payload)
            .map_err(|e| e.to_string())
    }
}

#[wasm_bindgen]
impl TsqlLiteWasmDb {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();
        let inner = Database::new();
        let default_session = inner.session_manager().create_session();
        Self {
            inner,
            default_session,
        }
    }

    pub fn exec(&mut self, sql: &str) -> Result<(), JsValue> {
        self.exec_session(self.default_session, sql)
    }

    pub fn exec_batch(&mut self, sql: &str) -> Result<(), JsValue> {
        self.exec_batch_session(self.default_session, sql)
    }

    pub fn query(&mut self, sql: &str) -> Result<String, JsValue> {
        self.query_session(self.default_session, sql)
    }

    pub fn create_session(&self) -> u64 {
        self.inner.session_manager().create_session()
    }

    pub fn close_session(&self, session_id: u64) -> Result<(), JsValue> {
        self.inner
            .session_manager()
            .close_session(session_id)
            .map_err(js_err)
    }

    pub fn exec_session(&mut self, session_id: u64, sql: &str) -> Result<(), JsValue> {
        self.exec_session_inner(session_id, sql).map_err(js_err)
    }

    pub fn exec_batch_session(&mut self, session_id: u64, sql: &str) -> Result<(), JsValue> {
        self.exec_batch_session_inner(session_id, sql)
            .map_err(js_err)
    }

    pub fn query_session(&mut self, session_id: u64, sql: &str) -> Result<String, JsValue> {
        self.query_session_inner(session_id, sql).map_err(js_err)
    }

    pub fn explain_sql(&self, sql: &str) -> Result<String, JsValue> {
        self.explain_sql_inner(sql).map_err(js_err)
    }

    pub fn trace_exec_batch_session(&self, session_id: u64, sql: &str) -> Result<String, JsValue> {
        let trace = self
            .inner
            .analyzer()
            .trace_execute_session_sql(session_id, sql)
            .map_err(js_err)?;
        serde_json::to_string(&trace).map_err(js_err)
    }

    pub fn trace_exec_batch(&self, sql: &str) -> Result<String, JsValue> {
        self.trace_exec_batch_session(self.default_session, sql)
    }

    pub fn session_options(&self, session_id: u64) -> Result<String, JsValue> {
        let options = self
            .inner
            .analyzer()
            .session_options(session_id)
            .map_err(js_err)?;
        serde_json::to_string(&options).map_err(js_err)
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    pub fn export_checkpoint(&self) -> Result<String, JsValue> {
        self.export_checkpoint_inner().map_err(js_err)
    }

    pub fn import_checkpoint(&mut self, payload: &str) -> Result<(), JsValue> {
        self.import_checkpoint_inner(payload).map_err(js_err)
    }
}

fn js_err(err: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[cfg(test)]
mod tests {
    use super::TsqlLiteWasmDb;

    #[test]
    fn wasm_db_exec_query_sessions_explain_and_checkpoint_roundtrip() {
        let mut db = TsqlLiteWasmDb::new();
        let sid = db.default_session;

        db.exec_batch_session_inner(
            sid,
            "CREATE TABLE dbo.Items (Id INT PRIMARY KEY, Name NVARCHAR(50) NOT NULL);
             INSERT INTO dbo.Items VALUES (1, N'Alpha'), (2, N'Beta');",
        )
        .expect("batch DDL+DML should succeed");

        let json = db
            .query_session_inner(sid, "SELECT Id, Name FROM dbo.Items ORDER BY Id;")
            .expect("query should succeed");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed["rows"].as_array().unwrap().len(), 2);

        let plan_json = db
            .explain_sql_inner("SELECT Id FROM dbo.Items WHERE Id = 1;")
            .expect("explain_sql should succeed");
        assert!(!plan_json.is_empty());

        let checkpoint = db
            .export_checkpoint_inner()
            .expect("export_checkpoint should succeed");

        let mut restored = TsqlLiteWasmDb::new();
        let rsid = restored.create_session();
        restored
            .import_checkpoint_inner(&checkpoint)
            .expect("import_checkpoint should succeed");

        let restored_json = restored
            .query_session_inner(rsid, "SELECT COUNT(*) AS Cnt FROM dbo.Items;")
            .expect("query on restored db should succeed");
        assert!(
            restored_json.contains('2'),
            "expected count 2 in {restored_json}"
        );

        restored.reset();
        assert!(restored
            .query_session_inner(rsid, "SELECT * FROM dbo.Items;")
            .is_err());
    }
}
