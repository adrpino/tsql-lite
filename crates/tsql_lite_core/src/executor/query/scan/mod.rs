mod executor;
mod strategy;

pub(crate) use executor::execute_scan;
pub(crate) use strategy::{choose_scan_strategy, extract_fast_scan_predicate};
