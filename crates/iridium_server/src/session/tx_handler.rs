use tokio::io::AsyncWriteExt;

use crate::tds::batch::build_error_response;
use crate::tds::packet::{self, PacketBuilder, TABULAR_RESULT};
use crate::tds::tokens;
use crate::tds::tx_manager::{
    isolation_level_sql, parse_tx_manager_request, quote_tsql_ident, TxManagerRequest,
};
use iridium_core::error::DbError;

use super::TdsSession;

pub(crate) async fn handle_transaction_manager<W: AsyncWriteExt + Unpin>(
    session: &mut TdsSession,
    data: &[u8],
    writer: &mut W,
) -> Result<(), DbError> {
    let req = match parse_tx_manager_request(data) {
        Ok(r) => r,
        Err(e) => {
            let err = DbError::Parse(e.to_string());
            let err_resp = build_error_response(&err);
            let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
            return Ok(());
        }
    };

    let session_id = session
        .session_id
        .ok_or_else(|| DbError::Execution("session not initialized".to_string()))?;

    let mut resp = PacketBuilder::with_capacity(64);
    match req {
        TxManagerRequest::Begin {
            isolation_level,
            name,
        } => {
            if let Err(e) = exec_begin_tx(session, session_id, isolation_level, &name, &mut resp) {
                let err_resp = build_error_response(&e);
                let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
                return Ok(());
            }
        }
        TxManagerRequest::Commit { name, chain_begin } => {
            let sql = if name.is_empty() {
                "COMMIT TRANSACTION".to_string()
            } else {
                format!("COMMIT TRANSACTION {}", quote_tsql_ident(&name))
            };
            if let Err(e) = session.db.execute_session_batch_sql_multi(session_id, &sql) {
                let err_resp = build_error_response(&e);
                let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
                return Ok(());
            }
            let prev_desc = session.tx_descriptor;
            session.tx_descriptor = 0;
            session.active_savepoints.clear();
            tokens::write_envchange_transaction(
                &mut resp,
                tokens::ENVCHANGE_COMMIT_TRANSACTION,
                None,
                if prev_desc != 0 {
                    Some(prev_desc)
                } else {
                    None
                },
            );
            if let Some((iso, next_name)) = chain_begin {
                if let Err(e) = exec_begin_tx(session, session_id, iso, &next_name, &mut resp) {
                    let err_resp = build_error_response(&e);
                    let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
                    return Ok(());
                }
            }
        }
        TxManagerRequest::Rollback { name, chain_begin } => {
            let is_savepoint = !name.is_empty()
                && session
                    .active_savepoints
                    .iter()
                    .any(|sp| sp.eq_ignore_ascii_case(&name));
            let sql = if name.is_empty() {
                "ROLLBACK TRANSACTION".to_string()
            } else {
                format!("ROLLBACK TRANSACTION {}", quote_tsql_ident(&name))
            };
            if let Err(e) = session.db.execute_session_batch_sql_multi(session_id, &sql) {
                let err_resp = build_error_response(&e);
                let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
                return Ok(());
            }
            if !is_savepoint {
                let prev_desc = session.tx_descriptor;
                session.tx_descriptor = 0;
                session.active_savepoints.clear();
                tokens::write_envchange_transaction(
                    &mut resp,
                    tokens::ENVCHANGE_ROLLBACK_TRANSACTION,
                    None,
                    if prev_desc != 0 {
                        Some(prev_desc)
                    } else {
                        None
                    },
                );
                if let Some((iso, next_name)) = chain_begin {
                    if let Err(e) = exec_begin_tx(session, session_id, iso, &next_name, &mut resp) {
                        let err_resp = build_error_response(&e);
                        let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
                        return Ok(());
                    }
                }
            }
        }
        TxManagerRequest::Save { name } => {
            let sql = format!("SAVE TRANSACTION {}", quote_tsql_ident(&name));
            if let Err(e) = session.db.execute_session_batch_sql_multi(session_id, &sql) {
                let err_resp = build_error_response(&e);
                let _ = packet::write_packet(writer, TABULAR_RESULT, &err_resp.data).await;
                return Ok(());
            }
            if !name.is_empty() {
                session.active_savepoints.push(name);
            }
        }
        TxManagerRequest::Other(req_type) => {
            log::warn!(
                "[conn={}] Unsupported Transaction Manager RequestType {}",
                session.connection_id,
                req_type
            );
        }
    }

    tokens::write_done(&mut resp, tokens::DONE_FINAL, 0, 0);
    let _ = packet::write_packet(writer, TABULAR_RESULT, resp.as_bytes()).await;
    Ok(())
}

fn exec_begin_tx(
    session: &mut TdsSession,
    session_id: iridium_core::SessionId,
    isolation_level: u8,
    name: &str,
    resp: &mut PacketBuilder,
) -> Result<(), DbError> {
    let mut sql = String::new();
    if let Some(iso_sql) = isolation_level_sql(isolation_level) {
        sql.push_str(iso_sql);
        sql.push_str(";\n");
    }
    if name.is_empty() {
        sql.push_str("BEGIN TRANSACTION;");
    } else {
        sql.push_str(&format!("BEGIN TRANSACTION {};", quote_tsql_ident(name)));
    }
    session
        .db
        .execute_session_batch_sql_multi(session_id, &sql)?;
    session.next_tx_descriptor = session.next_tx_descriptor.wrapping_add(1).max(1);
    session.tx_descriptor = session.next_tx_descriptor;
    session.active_savepoints.clear();
    tokens::write_envchange_transaction(
        resp,
        tokens::ENVCHANGE_BEGIN_TRANSACTION,
        Some(session.tx_descriptor),
        None,
    );
    Ok(())
}
