use super::super::virtual_table_def;
use super::super::VirtualTable;
use crate::catalog::Catalog;
use crate::executor::context::ExecutionContext;
use crate::storage::StoredRow;
use crate::types::DataType;

pub(crate) struct SysServerEventSessions;
pub(crate) struct SysServerEventSessionEvents;
pub(crate) struct SysServerEventSessionActions;
pub(crate) struct SysServerEventSessionTargets;
pub(crate) struct SysServerEventSessionFields;

impl VirtualTable for SysServerEventSessions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "server_event_sessions",
            vec![
                ("event_session_id", DataType::Int, false),
                ("name", DataType::NVarChar { max_len: 128 }, false),
                ("event_retention_mode", DataType::Char { len: 1 }, true),
                (
                    "event_retention_mode_desc",
                    DataType::NVarChar { max_len: 60 },
                    true,
                ),
                ("max_dispatch_latency", DataType::Int, true),
                ("max_memory", DataType::Int, true),
                ("max_event_size", DataType::Int, true),
                ("memory_partition_mode", DataType::Char { len: 1 }, true),
                (
                    "memory_partition_mode_desc",
                    DataType::NVarChar { max_len: 60 },
                    true,
                ),
                ("track_causality", DataType::Bit, true),
                ("startup_state", DataType::Bit, true),
                ("has_long_running_target", DataType::Bit, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        Vec::new()
    }
}

impl VirtualTable for SysServerEventSessionEvents {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "server_event_session_events",
            vec![
                ("event_session_id", DataType::Int, false),
                ("event_id", DataType::Int, false),
                ("name", DataType::NVarChar { max_len: 128 }, true),
                ("package", DataType::NVarChar { max_len: 128 }, true),
                ("module", DataType::NVarChar { max_len: 128 }, true),
                ("predicate", DataType::NVarChar { max_len: 3000 }, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        Vec::new()
    }
}

impl VirtualTable for SysServerEventSessionActions {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "server_event_session_actions",
            vec![
                ("event_session_id", DataType::Int, false),
                ("event_id", DataType::Int, false),
                ("name", DataType::NVarChar { max_len: 128 }, true),
                ("package", DataType::NVarChar { max_len: 128 }, true),
                ("module", DataType::NVarChar { max_len: 128 }, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        Vec::new()
    }
}

impl VirtualTable for SysServerEventSessionTargets {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "server_event_session_targets",
            vec![
                ("event_session_id", DataType::Int, false),
                ("target_id", DataType::Int, false),
                ("name", DataType::NVarChar { max_len: 128 }, true),
                ("package", DataType::NVarChar { max_len: 128 }, true),
                ("module", DataType::NVarChar { max_len: 128 }, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        Vec::new()
    }
}

impl VirtualTable for SysServerEventSessionFields {
    fn definition(&self) -> crate::catalog::TableDef {
        virtual_table_def(
            "server_event_session_fields",
            vec![
                ("event_session_id", DataType::Int, false),
                ("object_id", DataType::Int, false),
                ("name", DataType::NVarChar { max_len: 128 }, true),
                ("value", DataType::SqlVariant, true),
            ],
        )
    }

    fn rows(&self, _catalog: &dyn Catalog, _ctx: &ExecutionContext) -> Vec<StoredRow> {
        Vec::new()
    }
}
