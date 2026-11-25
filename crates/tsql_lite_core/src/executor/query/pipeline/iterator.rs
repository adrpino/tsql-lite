use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use crate::ast::{Expr, JoinType};
use crate::catalog::Catalog;
use crate::error::DbError;
use crate::executor::clock::Clock;
use crate::executor::context::ExecutionContext;
use crate::executor::evaluator::{eval_expr, eval_predicate};
use crate::executor::identifier::{resolve_column_slot, value_at_slot};
use crate::executor::joins::eval_key;
use crate::executor::model::{BoundTable, ContextTable, JoinedRow};
use crate::executor::value_ops::{compare_values, HashKeyFamily, SqlHashKey};
use crate::storage::Storage;
use crate::types::Value;

pub trait RowIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError>;

    fn reset(&mut self) -> Result<(), DbError>;
}

pub struct ScanIterator {
    pub rows: Vec<JoinedRow>,
    pub current_index: usize,
}

impl ScanIterator {
    pub fn new(rows: Vec<JoinedRow>) -> Self {
        Self {
            rows,
            current_index: 0,
        }
    }
}

impl RowIterator for ScanIterator {
    fn next_row(
        &mut self,
        _ctx: &mut ExecutionContext,
        _catalog: &dyn Catalog,
        _storage: &dyn Storage,
        _clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        if self.current_index < self.rows.len() {
            let row = self.rows[self.current_index].clone();
            self.current_index += 1;
            Ok(Some(row))
        } else {
            Ok(None)
        }
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.current_index = 0;
        Ok(())
    }
}

pub struct FilterIterator {
    pub source: Box<dyn RowIterator>,
    pub predicate: Expr,
}

impl RowIterator for FilterIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        while let Some(row) = self.source.next_row(ctx, catalog, storage, clock)? {
            if eval_predicate(&self.predicate, &row, ctx, catalog, storage, clock)? {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.source.reset()
    }
}

pub struct NestedLoopJoinIterator {
    pub left: Box<dyn RowIterator>,
    pub right: Box<dyn RowIterator>,
    pub current_left: Option<JoinedRow>,
    pub join_type: JoinType,
    pub on: Option<Expr>,
    pub right_shape: Vec<ContextTable>,
    pub matched_current_left: bool,
}

impl RowIterator for NestedLoopJoinIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        loop {
            if self.current_left.is_none() {
                self.current_left = self.left.next_row(ctx, catalog, storage, clock)?;
                if self.current_left.is_none() {
                    return Ok(None);
                }
                self.right.reset()?;
                self.matched_current_left = false;
            }

            while let Some(right_row) = self.right.next_row(ctx, catalog, storage, clock)? {
                let mut candidate = self.current_left.as_ref().unwrap().clone();
                candidate.extend(right_row);

                let matches = if let Some(on_expr) = &self.on {
                    eval_predicate(on_expr, &candidate, ctx, catalog, storage, clock)?
                } else {
                    true // Cross join
                };

                if matches {
                    self.matched_current_left = true;
                    return Ok(Some(candidate));
                }
            }

            // Exhausted right side for current_left
            let left_row = self.current_left.take().unwrap();
            if !self.matched_current_left
                && (self.join_type == JoinType::Left || self.join_type == JoinType::Full)
            {
                let mut candidate = left_row;
                candidate.extend(self.right_shape.iter().map(ContextTable::null_row));
                return Ok(Some(candidate));
            }
        }
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.left.reset()?;
        self.current_left = None;
        self.matched_current_left = false;
        Ok(())
    }
}

pub struct HashJoinIterator {
    pub left: Box<dyn RowIterator>,
    pub right: Box<dyn RowIterator>,
    pub left_keys: Vec<Expr>,
    pub right_keys: Vec<Expr>,
    pub residual_on: Option<Expr>,
    pub join_type: JoinType,
    pub left_shape: Vec<ContextTable>,
    pub right_shape: Vec<ContextTable>,

    // Pre-resolved direct column slots for simple column-reference join keys.
    left_key_slots: Vec<Option<(usize, usize)>>,
    right_key_slots: Vec<Option<(usize, usize)>>,

    // Build state
    pub build_done: bool,
    pub right_materialized: Vec<JoinedRow>,
    pub right_raw_keys: Vec<Vec<Value>>,
    pub right_key_families: Vec<Option<HashKeyFamily>>,
    pub hash_map: HashMap<Vec<SqlHashKey>, Vec<usize>>,
    single_hash_map: HashMap<SqlHashKey, Vec<usize>>,
    single_right_raw_keys: Vec<Value>,
    probe_key_buf: Vec<SqlHashKey>,
    pub right_matched: Vec<bool>,

    // Probe state
    pub current_left: Option<JoinedRow>,
    pub current_matches: Vec<usize>,
    pub current_match_idx: usize,
    pub matched_current_left: bool,

    // Post-probe state (for RIGHT/FULL OUTER)
    pub finishing_right: bool,
    pub finishing_idx: usize,
}

impl HashJoinIterator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        left: Box<dyn RowIterator>,
        right: Box<dyn RowIterator>,
        left_keys: Vec<Expr>,
        right_keys: Vec<Expr>,
        residual_on: Option<Expr>,
        join_type: JoinType,
        left_shape: Vec<ContextTable>,
        right_shape: Vec<ContextTable>,
    ) -> Self {
        let left_key_slots = left_keys
            .iter()
            .map(|e| resolve_column_slot(e, &left_shape))
            .collect();
        let right_key_slots = right_keys
            .iter()
            .map(|e| resolve_column_slot(e, &right_shape))
            .collect();
        Self {
            left,
            right,
            left_keys,
            right_keys,
            residual_on,
            join_type,
            left_shape,
            right_shape,
            left_key_slots,
            right_key_slots,
            build_done: false,
            right_materialized: Vec::new(),
            right_raw_keys: Vec::new(),
            right_key_families: Vec::new(),
            hash_map: HashMap::new(),
            single_hash_map: HashMap::new(),
            single_right_raw_keys: Vec::new(),
            probe_key_buf: Vec::new(),
            right_matched: Vec::new(),
            current_left: None,
            current_matches: Vec::new(),
            current_match_idx: 0,
            matched_current_left: false,
            finishing_right: false,
            finishing_idx: 0,
        }
    }
}

impl RowIterator for HashJoinIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        let is_single_key = self.left_keys.len() == 1 && self.right_keys.len() == 1;

        if !self.build_done {
            self.right_key_families = vec![None; self.right_keys.len()];
            if is_single_key {
                let rslot = self.right_key_slots[0];
                let rexpr = &self.right_keys[0];
                while let Some(right_row) = self.right.next_row(ctx, catalog, storage, clock)? {
                    let ri = self.right_materialized.len();
                    let evaluated: Option<Value>;
                    let val_ref: &Value = if let Some(slot) = rslot {
                        evaluated = None;
                        value_at_slot(&right_row, slot)
                    } else {
                        evaluated = Some(eval_expr(rexpr, &right_row, ctx, catalog, storage, clock)?);
                        evaluated.as_ref().unwrap()
                    };
                    if !val_ref.is_null() {
                        let nk = SqlHashKey::from_value(val_ref);
                        let fam = nk.family();
                        match self.right_key_families[0] {
                            None => self.right_key_families[0] = Some(fam),
                            Some(prev) if prev != fam => {
                                self.right_key_families[0] = Some(HashKeyFamily::Mixed);
                            }
                            _ => {}
                        }
                        self.single_hash_map.entry(nk).or_default().push(ri);
                    }
                    if rslot.is_none() {
                        self.single_right_raw_keys.push(evaluated.unwrap());
                    }
                    self.right_materialized.push(right_row);
                    self.right_matched.push(false);
                }
            } else {
                while let Some(right_row) = self.right.next_row(ctx, catalog, storage, clock)? {
                    let ri = self.right_materialized.len();
                    let raw_key =
                        eval_key(&self.right_keys, &right_row, ctx, catalog, storage, clock)?;
                    if !raw_key.iter().any(|v| v.is_null()) {
                        let norm_key: Vec<SqlHashKey> =
                            raw_key.iter().map(SqlHashKey::from_value).collect();
                        for (slot, nk) in norm_key.iter().enumerate() {
                            let fam = nk.family();
                            match self.right_key_families[slot] {
                                None => self.right_key_families[slot] = Some(fam),
                                Some(prev) if prev != fam => {
                                    self.right_key_families[slot] = Some(HashKeyFamily::Mixed);
                                }
                                _ => {}
                            }
                        }
                        self.hash_map.entry(norm_key).or_default().push(ri);
                    }
                    self.right_raw_keys.push(raw_key);
                    self.right_materialized.push(right_row);
                    self.right_matched.push(false);
                }
            }
            self.build_done = true;
        }

        if !self.finishing_right {
            let needs_unmatched_left =
                self.join_type == JoinType::Left || self.join_type == JoinType::Full;
            loop {
                if self.current_left.is_none() {
                    self.current_left = self.left.next_row(ctx, catalog, storage, clock)?;
                    if self.current_left.is_none() {
                        if self.join_type == JoinType::Right || self.join_type == JoinType::Full {
                            self.finishing_right = true;
                            break;
                        } else {
                            return Ok(None);
                        }
                    }

                    self.matched_current_left = false;
                    self.current_matches.clear();
                    self.current_match_idx = 0;

                    if is_single_key {
                        let left_row = self.current_left.as_ref().unwrap();
                        let evaluated: Option<Value>;
                        let val_ref: &Value = if let Some(slot) = self.left_key_slots[0] {
                            value_at_slot(left_row, slot)
                        } else {
                            evaluated = Some(eval_expr(
                                &self.left_keys[0],
                                left_row,
                                ctx,
                                catalog,
                                storage,
                                clock,
                            )?);
                            evaluated.as_ref().unwrap()
                        };
                        if !val_ref.is_null() {
                            let nk = SqlHashKey::from_value(val_ref);
                            let same_family = matches!(
                                self.right_key_families[0],
                                Some(rf) if rf == nk.family() && rf != HashKeyFamily::Mixed
                            );
                            if same_family {
                                if let Some(indices) = self.single_hash_map.get(&nk) {
                                    self.current_matches.extend_from_slice(indices);
                                }
                            } else if !self.right_materialized.is_empty() {
                                // Cross-family join fallback to exact SQL equality.
                                if let Some(rslot) = self.right_key_slots[0] {
                                    for (ri, rrow) in self.right_materialized.iter().enumerate() {
                                        let rv = value_at_slot(rrow, rslot);
                                        if !rv.is_null()
                                            && compare_values(val_ref, rv) == Ordering::Equal
                                        {
                                            self.current_matches.push(ri);
                                        }
                                    }
                                } else {
                                    for (ri, rv) in self.single_right_raw_keys.iter().enumerate() {
                                        if !rv.is_null()
                                            && compare_values(val_ref, rv) == Ordering::Equal
                                        {
                                            self.current_matches.push(ri);
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        let raw_key = eval_key(
                            &self.left_keys,
                            self.current_left.as_ref().unwrap(),
                            ctx,
                            catalog,
                            storage,
                            clock,
                        )?;
                        if !raw_key.iter().any(|v| v.is_null()) {
                            self.probe_key_buf.clear();
                            self.probe_key_buf
                                .extend(raw_key.iter().map(SqlHashKey::from_value));
                            let same_family =
                                self.probe_key_buf.iter().enumerate().all(|(slot, nk)| {
                                    matches!(
                                        self.right_key_families.get(slot).copied().flatten(),
                                        Some(rf) if rf == nk.family() && rf != HashKeyFamily::Mixed
                                    )
                                });
                            if same_family {
                                if let Some(indices) = self.hash_map.get(&self.probe_key_buf) {
                                    self.current_matches.extend_from_slice(indices);
                                }
                            } else if !self.right_raw_keys.is_empty() {
                                // Cross-family join (e.g. INT = NVARCHAR from STRING_SPLIT,
                                // FLOAT = INT, UUID = VARCHAR, DATETIME = VARCHAR): fall back
                                // to exact SQL equality so implicit coercions always match.
                                for (ri, rkey) in self.right_raw_keys.iter().enumerate() {
                                    if rkey.iter().any(|v| v.is_null()) {
                                        continue;
                                    }
                                    if raw_key.iter().zip(rkey.iter()).all(|(lv, rv)| {
                                        compare_values(lv, rv) == Ordering::Equal
                                    }) {
                                        self.current_matches.push(ri);
                                    }
                                }
                            }
                        }
                    }
                }

                while self.current_match_idx < self.current_matches.len() {
                    let ri = self.current_matches[self.current_match_idx];
                    self.current_match_idx += 1;
                    let is_last_match = self.current_match_idx == self.current_matches.len();
                    let can_move_left = is_last_match
                        && (self.residual_on.is_none()
                            || self.matched_current_left
                            || !needs_unmatched_left);
                    let right_row = &self.right_materialized[ri];
                    let mut candidate = if can_move_left {
                        let mut left_row = self.current_left.take().unwrap();
                        left_row.reserve(right_row.len());
                        left_row
                    } else {
                        let left_row = self.current_left.as_ref().unwrap();
                        let mut buf = Vec::with_capacity(left_row.len() + right_row.len());
                        buf.extend(left_row.iter().cloned());
                        buf
                    };
                    candidate.extend(right_row.iter().cloned());
                    if let Some(ref res_pred) = self.residual_on {
                        if !eval_predicate(res_pred, &candidate, ctx, catalog, storage, clock)? {
                            continue;
                        }
                    }
                    self.right_matched[ri] = true;
                    self.matched_current_left = true;
                    return Ok(Some(candidate));
                }

                // Exhausted matches for current_left (may already have been moved on last match)
                if let Some(left_row) = self.current_left.take() {
                    if !self.matched_current_left && needs_unmatched_left {
                        let mut candidate = left_row;
                        candidate.extend(self.right_shape.iter().map(ContextTable::null_row));
                        return Ok(Some(candidate));
                    }
                }
            }
        }

        // Finishing right side (for RIGHT/FULL)
        while self.finishing_idx < self.right_matched.len() {
            let ri = self.finishing_idx;
            self.finishing_idx += 1;
            if !self.right_matched[ri] {
                let mut candidate: JoinedRow =
                    self.left_shape.iter().map(ContextTable::null_row).collect();
                candidate.extend(self.right_materialized[ri].clone());
                return Ok(Some(candidate));
            }
        }

        Ok(None)
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.left.reset()?;
        self.current_left = None;
        self.current_matches.clear();
        self.current_match_idx = 0;
        self.matched_current_left = false;
        self.finishing_right = false;
        self.finishing_idx = 0;
        self.right_matched.fill(false);
        // We don't need to rebuild the right side hash map.
        Ok(())
    }
}

pub struct TableScanIterator {
    pub bound: BoundTable,
    pub next_index: usize,
    /// Optional single-column `col <op> const` pre-filter evaluated directly against borrowed
    /// `&StoredRow` slices before cloning or constructing `ContextTable`.
    pub fast_filter: Option<(usize, crate::ast::BinaryOp, Value)>,
}

#[inline]
fn fast_filter_matches(
    stored: &crate::storage::StoredRow,
    filter: &(usize, crate::ast::BinaryOp, Value),
) -> bool {
    let (col_idx, op, rhs) = filter;
    let Some(lhs) = stored.values.get(*col_idx) else {
        return false;
    };
    if lhs.is_null() || rhs.is_null() {
        return false;
    }
    let ord = compare_values(lhs, rhs);
    match op {
        crate::ast::BinaryOp::Eq => ord == Ordering::Equal,
        crate::ast::BinaryOp::NotEq => ord != Ordering::Equal,
        crate::ast::BinaryOp::Gt => ord == Ordering::Greater,
        crate::ast::BinaryOp::Gte => matches!(ord, Ordering::Greater | Ordering::Equal),
        crate::ast::BinaryOp::Lt => ord == Ordering::Less,
        crate::ast::BinaryOp::Lte => matches!(ord, Ordering::Less | Ordering::Equal),
        _ => true,
    }
}

impl RowIterator for TableScanIterator {
    fn next_row(
        &mut self,
        ctx: &mut ExecutionContext,
        catalog: &dyn Catalog,
        storage: &dyn Storage,
        clock: &dyn Clock,
    ) -> Result<Option<JoinedRow>, DbError> {
        let has_rls = !catalog.get_security_policies().is_empty();
        // Fast path: zero-copy slice iteration over InMemoryStorage (no per-row HashMap lookup
        // and zero StoredRow clones for deleted or fast-filtered non-matching rows).
        if let Some(slice) = storage.get_table_slice(self.bound.table.id) {
            while self.next_index < slice.len() {
                let idx = self.next_index;
                self.next_index += 1;
                let stored = &slice[idx];
                if stored.deleted {
                    continue;
                }
                if let Some(ref f) = self.fast_filter {
                    if !fast_filter_matches(stored, f) {
                        continue;
                    }
                }
                if has_rls
                    && !crate::executor::rls::row_passes_rls_filter(
                        &self.bound.table,
                        stored,
                        ctx,
                        catalog,
                        storage,
                        clock,
                    )?
                {
                    continue;
                }
                return Ok(Some(vec![ContextTable {
                    table: self.bound.table.clone(),
                    alias: self.bound.alias.clone(),
                    row: Some(Arc::new(stored.clone())),
                    storage_index: Some(idx),
                    source_aliases: Vec::new(),
                }]));
            }
            return Ok(None);
        }

        loop {
            let row = storage.get_row(self.bound.table.id, self.next_index)?;
            match row {
                Some(stored) => {
                    let idx = self.next_index;
                    self.next_index += 1;
                    if stored.deleted {
                        continue;
                    }
                    if let Some(ref f) = self.fast_filter {
                        if !fast_filter_matches(&stored, f) {
                            continue;
                        }
                    }
                    if !has_rls
                        || crate::executor::rls::row_passes_rls_filter(
                            &self.bound.table,
                            &stored,
                            ctx,
                            catalog,
                            storage,
                            clock,
                        )?
                    {
                        return Ok(Some(vec![ContextTable {
                            table: self.bound.table.clone(),
                            alias: self.bound.alias.clone(),
                            row: Some(Arc::new(stored)),
                            storage_index: Some(idx),
                            source_aliases: Vec::new(),
                        }]));
                    }
                }
                None => return Ok(None),
            }
        }
    }

    fn reset(&mut self) -> Result<(), DbError> {
        self.next_index = 0;
        Ok(())
    }
}
