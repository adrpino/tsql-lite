use std::sync::atomic::Ordering;

use super::*;

impl IdAllocator for CatalogImpl {
    fn alloc_table_id(&mut self) -> u32 {
        self.next_table_id.fetch_add(1, Ordering::Relaxed)
    }

    fn alloc_column_id(&mut self) -> u32 {
        self.next_column_id.fetch_add(1, Ordering::Relaxed)
    }

    fn alloc_index_id(&mut self) -> u32 {
        self.next_index_id.fetch_add(1, Ordering::Relaxed)
    }

    fn alloc_object_id(&mut self) -> i32 {
        self.next_object_id.fetch_sub(1, Ordering::Relaxed)
    }

    fn alloc_schema_id(&mut self) -> u32 {
        self.next_schema_id.fetch_add(1, Ordering::Relaxed)
    }
}
