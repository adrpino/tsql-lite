use crate::ast::{BinaryOp, Expr, OrderByExpr};
use crate::catalog::Catalog;

use crate::executor::model::BoundTable;
use crate::executor::physical::ScanStrategy;

pub(crate) fn choose_scan_strategy(
    bound: &BoundTable,
    predicate: Option<&Expr>,
    order_by: &[OrderByExpr],
    catalog: &dyn Catalog,
) -> ScanStrategy {
    let indexes: Vec<&crate::catalog::IndexDef> = catalog
        .get_indexes()
        .iter()
        .filter(|idx| idx.table_id == bound.table.id)
        .collect();
    if indexes.is_empty() {
        return ScanStrategy::TableScan;
    }

    // P1: Try all indexes, not just the first one
    for idx in &indexes {
        let Some(first_col_id) = idx.column_ids.first() else {
            continue;
        };
        let Some(first_col) = bound.table.columns.iter().find(|c| c.id == *first_col_id) else {
            continue;
        };

        if let Some(pred) = predicate {
            if let Some((op, _)) =
                extract_index_predicate_rhs(Some(pred), &bound.alias, &first_col.name)
            {
                if matches!(op, BinaryOp::Eq) {
                    return ScanStrategy::IndexSeek { index_id: idx.id };
                }
                return ScanStrategy::IndexScan { index_id: idx.id };
            }
        }
    }

    // P1: Check ORDER BY with any index
    if order_by.len() == 1 {
        if let Expr::QualifiedIdentifier(parts) = &order_by[0].expr {
            for idx in &indexes {
                let Some(first_col_id) = idx.column_ids.first() else {
                    continue;
                };
                let Some(first_col) = bound.table.columns.iter().find(|c| c.id == *first_col_id)
                else {
                    continue;
                };
                if parts.len() >= 2
                    && parts[0].eq_ignore_ascii_case(&bound.alias)
                    && parts[1].eq_ignore_ascii_case(&first_col.name)
                    && order_by[0].asc
                {
                    return ScanStrategy::IndexScan { index_id: idx.id };
                }
            }
        }
    }

    // Only use an index when the predicate/ORDER BY constrains the index's leading key column.
    ScanStrategy::TableScan
}

fn flip_cmp_op(op: BinaryOp) -> BinaryOp {
    match op {
        BinaryOp::Gt => BinaryOp::Lt,
        BinaryOp::Gte => BinaryOp::Lte,
        BinaryOp::Lt => BinaryOp::Gt,
        BinaryOp::Lte => BinaryOp::Gte,
        other => other,
    }
}

fn expr_matches_column(expr: &Expr, alias: &str, column: &str) -> bool {
    match expr {
        Expr::Identifier(name) => !name.starts_with('@') && name.eq_ignore_ascii_case(column),
        Expr::QualifiedIdentifier(parts) => {
            parts.len() >= 2
                && parts[parts.len() - 2].eq_ignore_ascii_case(alias)
                && parts[parts.len() - 1].eq_ignore_ascii_case(column)
        }
        _ => false,
    }
}

/// Returns `true` if `expr` can be evaluated without any row binding (literals, `@vars`, or pure
/// arithmetic/casts over them).
pub(crate) fn is_row_independent_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Integer(_)
        | Expr::FloatLiteral(_)
        | Expr::String(_)
        | Expr::UnicodeString(_)
        | Expr::BinaryLiteral(_)
        | Expr::Null => true,
        Expr::Identifier(name) => name.starts_with('@'),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::TryCast { expr, .. }
        | Expr::Convert { expr, .. }
        | Expr::TryConvert { expr, .. } => is_row_independent_expr(expr),
        Expr::Binary { left, right, .. } => {
            is_row_independent_expr(left) && is_row_independent_expr(right)
        }
        _ => false,
    }
}

pub(crate) fn extract_index_predicate_rhs(
    predicate: Option<&Expr>,
    alias: &str,
    column: &str,
) -> Option<(BinaryOp, Expr)> {
    let pred = predicate?;
    match pred {
        Expr::Binary { left, op, right } => {
            if is_supported_index_op(*op) {
                if expr_matches_column(left, alias, column) && is_row_independent_expr(right) {
                    return Some((*op, (**right).clone()));
                }
                if expr_matches_column(right, alias, column) && is_row_independent_expr(left) {
                    return Some((flip_cmp_op(*op), (**left).clone()));
                }
            }
            if *op == BinaryOp::And {
                extract_index_predicate_rhs(Some(left), alias, column)
                    .or_else(|| extract_index_predicate_rhs(Some(right), alias, column))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extracts the most selective single-column `col <op> const` conjunct from `predicate` for
/// zero-clone slice pre-filtering in `TableScanIterator`. Prefers `Eq` (especially on PK/unique
/// columns) over inequality comparisons.
pub(crate) fn extract_fast_scan_predicate(
    predicate: &Expr,
    bound: &BoundTable,
) -> Option<(usize, BinaryOp, Expr)> {
    let mut best: Option<(u8, usize, BinaryOp, Expr)> = None;
    for (col_idx, col) in bound.table.columns.iter().enumerate() {
        // Respect custom per-column collations by leaving them to full eval_predicate.
        if col.collation.is_some() {
            continue;
        }
        if let Some((op, rhs)) =
            extract_index_predicate_rhs(Some(predicate), &bound.alias, &col.name)
        {
            let score = match (op, col.primary_key || col.unique) {
                (BinaryOp::Eq, true) => 3,
                (BinaryOp::Eq, false) => 2,
                _ => 1,
            };
            if best.as_ref().is_none_or(|b| score > b.0) {
                best = Some((score, col_idx, op, rhs));
                if score == 3 {
                    break;
                }
            }
        }
    }
    best.map(|(_, col_idx, op, rhs)| (col_idx, op, rhs))
}

fn is_supported_index_op(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::Gt | BinaryOp::Gte | BinaryOp::Lt | BinaryOp::Lte
    )
}
