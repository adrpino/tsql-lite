use super::super::ScriptExecutor;
use crate::ast::RaiserrorStmt;
use crate::error::DbError;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::eval_expr;

impl<'a> ScriptExecutor<'a> {
    pub(crate) fn execute_raiserror(
        &mut self,
        stmt: &RaiserrorStmt,
        ctx: &mut ExecutionContext<'_>,
    ) -> Result<Option<crate::executor::result::QueryResult>, DbError> {
        let msg_val = eval_expr(
            &stmt.message,
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        let severity_val = eval_expr(
            &stmt.severity,
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;
        let state_val = eval_expr(
            &stmt.state,
            &[],
            ctx,
            self.catalog,
            self.storage,
            self.clock,
        )?;

        let severity = match severity_val {
            crate::types::Value::Bit(b) => i32::from(b),
            crate::types::Value::TinyInt(v) => v as i32,
            crate::types::Value::SmallInt(v) => v as i32,
            crate::types::Value::Int(v) => v,
            crate::types::Value::BigInt(v) => v as i32,
            crate::types::Value::Decimal(raw, scale) => (raw / 10i128.pow(scale as u32)) as i32,
            _ => 16,
        };

        let state = match state_val {
            crate::types::Value::Bit(b) => u8::from(b),
            crate::types::Value::TinyInt(v) => v,
            crate::types::Value::SmallInt(v) => v.clamp(0, 255) as u8,
            crate::types::Value::Int(v) => v.clamp(0, 255) as u8,
            crate::types::Value::BigInt(v) => v.clamp(0, 255) as u8,
            crate::types::Value::Decimal(raw, scale) => {
                ((raw / 10i128.pow(scale as u32)).clamp(0, 255)) as u8
            }
            _ => 1,
        };

        let mut formatted_args = Vec::with_capacity(stmt.args.len());
        for arg in &stmt.args {
            let v = eval_expr(arg, &[], ctx, self.catalog, self.storage, self.clock)?;
            formatted_args.push(v.to_string_value());
        }

        let msg = if formatted_args.is_empty() {
            msg_val.to_string_value()
        } else {
            format_raiserror_message(&msg_val.to_string_value(), &formatted_args)
        };

        if severity >= 11 {
            // Severities 11+ are user errors; 0-10 are informational messages.
            Err(DbError::Raised {
                number: 50000,
                severity: severity.clamp(11, 25) as u8,
                state,
                message: msg,
            })
        } else {
            // Informational severity prints without aborting execution.
            ctx.session.print_output.push(msg);
            Ok(None)
        }
    }
}

pub(crate) fn format_raiserror_message(template: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(template.len());
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    let mut arg_idx = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            if chars[i + 1] == '%' {
                out.push('%');
                i += 2;
                continue;
            }
            let mut j = i + 1;
            // Skip optional flags/width/precision and I64 length prefix
            while j < chars.len() && matches!(chars[j], '-' | '+' | ' ' | '#' | '0'..='9' | '.') {
                j += 1;
            }
            if j + 2 < chars.len()
                && (chars[j] == 'I' || chars[j] == 'i')
                && chars[j + 1] == '6'
                && chars[j + 2] == '4'
            {
                j += 3;
            }
            if j < chars.len() && matches!(chars[j], 'd' | 'i' | 'o' | 's' | 'u' | 'x' | 'X') {
                if let Some(val) = args.get(arg_idx) {
                    out.push_str(val);
                }
                arg_idx += 1;
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
