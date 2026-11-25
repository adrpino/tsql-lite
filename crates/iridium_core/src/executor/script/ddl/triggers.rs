use super::super::ScriptExecutor;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn ddl_event_matches(registered: &str, actual: &str) -> bool {
    if registered.eq_ignore_ascii_case(actual) {
        return true;
    }
    match registered.to_ascii_uppercase().as_str() {
        "DDL_DATABASE_LEVEL_EVENTS" => true,
        "DDL_TABLE_EVENTS" => matches!(actual, "CREATE_TABLE" | "ALTER_TABLE" | "DROP_TABLE"),
        "DDL_VIEW_EVENTS" => matches!(actual, "CREATE_VIEW" | "ALTER_VIEW" | "DROP_VIEW"),
        "DDL_TABLE_VIEW_EVENTS" => matches!(
            actual,
            "CREATE_TABLE"
                | "ALTER_TABLE"
                | "DROP_TABLE"
                | "CREATE_VIEW"
                | "ALTER_VIEW"
                | "DROP_VIEW"
                | "CREATE_INDEX"
                | "ALTER_INDEX"
                | "DROP_INDEX"
        ),
        "DDL_INDEX_EVENTS" => matches!(actual, "CREATE_INDEX" | "ALTER_INDEX" | "DROP_INDEX"),
        "DDL_PROCEDURE_EVENTS" => {
            matches!(
                actual,
                "CREATE_PROCEDURE" | "ALTER_PROCEDURE" | "DROP_PROCEDURE"
            )
        }
        "DDL_FUNCTION_EVENTS" => {
            matches!(
                actual,
                "CREATE_FUNCTION" | "ALTER_FUNCTION" | "DROP_FUNCTION"
            )
        }
        "DDL_TRIGGER_EVENTS" => {
            matches!(actual, "CREATE_TRIGGER" | "ALTER_TRIGGER" | "DROP_TRIGGER")
        }
        "DDL_SCHEMA_EVENTS" => matches!(actual, "CREATE_SCHEMA" | "ALTER_SCHEMA" | "DROP_SCHEMA"),
        "DDL_TYPE_EVENTS" => matches!(actual, "CREATE_TYPE" | "DROP_TYPE"),
        "DDL_SEQUENCE_EVENTS" => {
            matches!(
                actual,
                "CREATE_SEQUENCE" | "ALTER_SEQUENCE" | "DROP_SEQUENCE"
            )
        }
        "DDL_SYNONYM_EVENTS" => matches!(actual, "CREATE_SYNONYM" | "DROP_SYNONYM"),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_eventdata_xml(
    event_type: &str,
    db_name: &str,
    schema_name: Option<&str>,
    object_name: &str,
    object_type: &str,
    login_name: &str,
    user_name: &str,
    spid: u64,
    post_time: &str,
    command_text: &str,
) -> String {
    let schema_xml = match schema_name {
        Some(s) if !s.is_empty() => format!("<SchemaName>{}</SchemaName>", xml_escape(s)),
        _ => String::new(),
    };
    format!(
        "<EVENT_INSTANCE>\
         <EventType>{}</EventType>\
         <PostTime>{}</PostTime>\
         <SPID>{}</SPID>\
         <ServerName>IRIDIUM-SQL</ServerName>\
         <LoginName>{}</LoginName>\
         <UserName>{}</UserName>\
         <DatabaseName>{}</DatabaseName>\
         {}\
         <ObjectName>{}</ObjectName>\
         <ObjectType>{}</ObjectType>\
         <TSQLCommand>\
         <SetOptions ANSI_NULLS=\"ON\" ANSI_NULL_DEFAULT=\"ON\" ANSI_PADDING=\"ON\" QUOTED_IDENTIFIER=\"ON\" ENCRYPTED=\"FALSE\" />\
         <CommandText>{}</CommandText>\
         </TSQLCommand>\
         </EVENT_INSTANCE>",
        xml_escape(event_type),
        xml_escape(post_time),
        spid,
        xml_escape(login_name),
        xml_escape(user_name),
        xml_escape(db_name),
        schema_xml,
        xml_escape(object_name),
        xml_escape(object_type),
        xml_escape(command_text),
    )
}

#[derive(Debug, Clone)]
pub(crate) struct DdlEventMeta {
    pub event_type: &'static str,
    pub schema_name: Option<String>,
    pub object_name: String,
    pub object_type: &'static str,
    pub command_text: String,
}

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn has_matching_ddl_trigger(
        &self,
        event_type: &str,
        ctx: &ExecutionContext<'_>,
    ) -> bool {
        if ctx.frame.trigger_depth >= 16 {
            return false;
        }
        self.catalog.get_triggers().iter().any(|t| {
            t.is_database_scope
                && t.enabled
                && !ctx.frame.active_triggers.iter().any(|(s, n)| {
                    s.eq_ignore_ascii_case(&t.schema) && n.eq_ignore_ascii_case(&t.name)
                })
                && t.ddl_events
                    .iter()
                    .any(|ev| ddl_event_matches(ev, event_type))
        })
    }

    pub(crate) fn run_with_ddl_triggers<T, F>(
        &mut self,
        meta: Option<DdlEventMeta>,
        ctx: &mut ExecutionContext<'_>,
        action: F,
    ) -> Result<T, DbError>
    where
        F: FnOnce(&mut Self, &mut ExecutionContext<'_>) -> Result<T, DbError>,
    {
        let Some(meta) = meta else {
            return action(self, ctx);
        };
        if !self.has_matching_ddl_trigger(meta.event_type, ctx) {
            return action(self, ctx);
        }

        let cat_snap = self.catalog.clone_boxed();
        let stor_snap = self.snapshot_transactional_tables(ctx)?;
        let outcome = action(self, ctx)?;

        let triggers: Vec<crate::catalog::TriggerDef> = self
            .catalog
            .get_triggers()
            .iter()
            .filter(|t| {
                t.is_database_scope
                    && t.enabled
                    && !ctx.frame.active_triggers.iter().any(|(s, n)| {
                        s.eq_ignore_ascii_case(&t.schema) && n.eq_ignore_ascii_case(&t.name)
                    })
                    && t.ddl_events
                        .iter()
                        .any(|ev| ddl_event_matches(ev, meta.event_type))
            })
            .map(|t| t.as_ref().clone())
            .collect();

        if triggers.is_empty() {
            return Ok(outcome);
        }

        let post_time = self
            .clock
            .now_datetime_literal()
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        let db_name = ctx
            .metadata
            .database
            .as_deref()
            .unwrap_or(&ctx.metadata.original_database)
            .to_string();
        let user_name = ctx
            .session
            .extras
            .impersonation_stack
            .last()
            .map(|s| s.as_str())
            .or(ctx.metadata.user.as_deref())
            .unwrap_or("dbo")
            .to_string();
        let xml = build_eventdata_xml(
            meta.event_type,
            &db_name,
            meta.schema_name.as_deref(),
            &meta.object_name,
            meta.object_type,
            "sa",
            &user_name,
            ctx.session_id(),
            &post_time,
            &meta.command_text,
        );

        let saved_xml = ctx.frame.ddl_event_xml.replace(xml);
        let saved_rolled_back = ctx.frame.script_tx.rolled_back;
        ctx.frame.script_tx.rolled_back = false;

        let mut trigger_err: Option<DbError> = None;
        for trig in triggers {
            ctx.frame.trigger_depth += 1;
            ctx.frame
                .active_triggers
                .push((trig.schema.clone(), trig.name.clone()));
            ctx.frame
                .trigger_frames
                .push(crate::executor::context::TriggerIntrospectionFrame {
                    object_id: trig.object_id,
                    table_columns: Vec::new(),
                    updated_columns: Vec::new(),
                });
            let exec_res = self.execute_batch(&trig.body, ctx);
            ctx.frame.trigger_frames.pop();
            ctx.frame.active_triggers.pop();
            ctx.frame.trigger_depth -= 1;

            if let Err(e) = exec_res {
                trigger_err = Some(e);
                break;
            }
            if ctx.frame.script_tx.rolled_back {
                break;
            }
        }

        let was_rolled_back = ctx.frame.script_tx.rolled_back;
        ctx.frame.ddl_event_xml = saved_xml;
        ctx.frame.script_tx.rolled_back = saved_rolled_back;

        if let Some(err) = trigger_err {
            self.catalog.restore_from_boxed(cat_snap);
            self.restore_transactional_tables(&stor_snap, ctx)?;
            return Err(err);
        }
        if was_rolled_back {
            self.catalog.restore_from_boxed(cat_snap);
            self.restore_transactional_tables(&stor_snap, ctx)?;
            return Err(DbError::Execution(
                "The transaction ended in the trigger. The batch has been aborted.".into(),
            ));
        }

        Ok(outcome)
    }
}
