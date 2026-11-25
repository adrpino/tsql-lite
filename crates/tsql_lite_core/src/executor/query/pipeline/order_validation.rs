use crate::error::DbError;

use crate::ast::{OrderByExpr, SelectItem};
use crate::executor::projection::{expr_label, resolve_projected_order_index_with_projection};

pub(crate) fn validate_projected_order_by(
    columns: &[String],
    projection: &[SelectItem],
    order_by: &[OrderByExpr],
) -> Result<(), DbError> {
    for item in order_by {
        if resolve_projected_order_index_with_projection(columns, projection, item).is_none() {
            return Err(DbError::invalid_identifier(format!(
                "invalid column in ORDER BY: {}",
                expr_label(&item.expr)
            )));
        }
    }
    Ok(())
}
