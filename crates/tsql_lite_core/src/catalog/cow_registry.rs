#[cfg(feature = "checkpoint")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeltaEntry {
    /// Entry lives at `delta_items[idx]`.
    Present(usize),
    /// Entry was removed in this fork and hides the matching entry in `base`.
    Deleted,
}

/// Copy-on-write catalog registry backed by a shared immutable base plus a small
/// per-fork delta overlay.
///
/// - Before the registry is cloned (`Arc::strong_count == 1`), mutations operate
///   in place on `base` and `base_map` with zero overlay overhead.
/// - Once cloned, mutations on any fork record only the touched keys in
///   `delta_items` / `delta_map` / `base_shadowed` in O(1), avoiding cloning the
///   full `base` vector or `base_map` hash table.
#[derive(Debug, Clone)]
pub(crate) struct CowRegistry<K, V> {
    base: Arc<Vec<Arc<V>>>,
    base_map: Arc<HashMap<K, usize>>,
    delta_items: Vec<Arc<V>>,
    delta_map: HashMap<K, DeltaEntry>,
    /// Sorted indices into `base` that are shadowed (replaced or deleted) by `delta_map`.
    base_shadowed: Vec<usize>,
}

impl<K, V> Default for CowRegistry<K, V> {
    fn default() -> Self {
        Self {
            base: Arc::new(Vec::new()),
            base_map: Arc::new(HashMap::new()),
            delta_items: Vec::new(),
            delta_map: HashMap::new(),
            base_shadowed: Vec::new(),
        }
    }
}

impl<K, V> CowRegistry<K, V> {
    #[inline]
    pub fn view(&self) -> CowRegistryView<'_, V> {
        CowRegistryView {
            base: &self.base,
            base_shadowed: &self.base_shadowed,
            delta_added: &self.delta_items,
        }
    }

    #[inline]
    pub fn iter(&self) -> CowRegistryIter<'_, V> {
        self.view().iter()
    }

    #[inline]
    fn has_no_delta(&self) -> bool {
        self.delta_map.is_empty() && self.base_shadowed.is_empty()
    }

    #[inline]
    fn mark_base_shadowed(&mut self, base_idx: usize) {
        if let Err(pos) = self.base_shadowed.binary_search(&base_idx) {
            self.base_shadowed.insert(pos, base_idx);
        }
    }

    #[inline]
    fn is_base_shadowed(&self, base_idx: usize) -> bool {
        self.base_shadowed.binary_search(&base_idx).is_ok()
    }
}

impl<K: Eq + Hash, V> CowRegistry<K, V> {
    #[inline]
    pub fn get(&self, key: &K) -> Option<&V> {
        if !self.delta_map.is_empty() {
            match self.delta_map.get(key) {
                Some(&DeltaEntry::Present(idx)) => return Some(&self.delta_items[idx]),
                Some(&DeltaEntry::Deleted) => return None,
                None => {}
            }
        }
        self.base_map.get(key).map(|&idx| self.base[idx].as_ref())
    }

    #[inline]
    pub fn get_arc(&self, key: &K) -> Option<Arc<V>> {
        if !self.delta_map.is_empty() {
            match self.delta_map.get(key) {
                Some(&DeltaEntry::Present(idx)) => return Some(Arc::clone(&self.delta_items[idx])),
                Some(&DeltaEntry::Deleted) => return None,
                None => {}
            }
        }
        self.base_map
            .get(key)
            .map(|&idx| Arc::clone(&self.base[idx]))
    }

    #[inline]
    pub fn contains_key(&self, key: &K) -> bool {
        if !self.delta_map.is_empty() {
            match self.delta_map.get(key) {
                Some(&DeltaEntry::Present(_)) => return true,
                Some(&DeltaEntry::Deleted) => return false,
                None => {}
            }
        }
        self.base_map.contains_key(key)
    }

    pub fn insert_or_replace(&mut self, key: K, value: V) {
        self.insert_or_replace_arc(key, Arc::new(value));
    }

    pub fn insert_or_replace_arc(&mut self, key: K, value: Arc<V>) {
        if self.has_no_delta() {
            if let (Some(base), Some(base_map)) = (
                Arc::get_mut(&mut self.base),
                Arc::get_mut(&mut self.base_map),
            ) {
                if let Some(&idx) = base_map.get(&key) {
                    base[idx] = value;
                } else {
                    let idx = base.len();
                    base_map.insert(key, idx);
                    base.push(value);
                }
                return;
            }
        }

        if let Some(&base_idx) = self.base_map.get(&key) {
            self.mark_base_shadowed(base_idx);
        }
        match self.delta_map.get(&key).copied() {
            Some(DeltaEntry::Present(d_idx)) => {
                self.delta_items[d_idx] = value;
            }
            Some(DeltaEntry::Deleted) | None => {
                let d_idx = self.delta_items.len();
                self.delta_items.push(value);
                self.delta_map.insert(key, DeltaEntry::Present(d_idx));
            }
        }
    }

    /// Merges every entry modified (inserted, replaced, or deleted) in `workspace`
    /// relative to `base` into `self`, leaving keys untouched by `workspace` intact.
    pub fn merge_delta_against_base<F>(&mut self, workspace: &Self, base: &Self, key_of: F)
    where
        K: Clone,
        F: Fn(&V) -> K,
    {
        if Arc::ptr_eq(&workspace.base, &base.base)
            && workspace.delta_map.is_empty()
            && base.delta_map.is_empty()
        {
            return;
        }
        if Arc::ptr_eq(&workspace.base, &base.base) {
            // Fast path: workspace and base share the exact same immutable `base` Arc,
            // so only keys present in either `delta_map` can possibly differ.
            for key in workspace.delta_map.keys() {
                let ws_val = workspace.get_arc(key);
                let base_val = base.get_arc(key);
                match (ws_val, base_val) {
                    (Some(a), Some(b)) if Arc::ptr_eq(&a, &b) => {}
                    (None, None) => {}
                    (Some(a), _) => self.insert_or_replace_arc(key.clone(), a),
                    (None, Some(_)) => {
                        let _ = self.remove(key, &key_of);
                    }
                }
            }
            for key in base.delta_map.keys() {
                if workspace.delta_map.contains_key(key) {
                    continue;
                }
                let ws_val = workspace.get_arc(key);
                let base_val = base.get_arc(key);
                match (ws_val, base_val) {
                    (Some(a), Some(b)) if Arc::ptr_eq(&a, &b) => {}
                    (None, None) => {}
                    (Some(a), _) => self.insert_or_replace_arc(key.clone(), a),
                    (None, Some(_)) => {
                        let _ = self.remove(key, &key_of);
                    }
                }
            }
        } else {
            // Fallback if workspace rebuilt its base (e.g. via rebuild_map):
            // diff all keys across workspace and base.
            for item in workspace.iter() {
                let key = key_of(item);
                let ws_val = workspace.get_arc(&key);
                let base_val = base.get_arc(&key);
                match (ws_val, base_val) {
                    (Some(a), Some(b)) if Arc::ptr_eq(&a, &b) => {}
                    (Some(a), _) => self.insert_or_replace_arc(key, a),
                    _ => {}
                }
            }
            for item in base.iter() {
                let key = key_of(item);
                if !workspace.contains_key(&key) {
                    let _ = self.remove(&key, &key_of);
                }
            }
        }
    }

    pub fn remove<F>(&mut self, key: &K, key_of: F) -> Option<Arc<V>>
    where
        K: Clone,
        F: Fn(&V) -> K,
    {
        if self.has_no_delta() {
            if let (Some(base), Some(base_map)) = (
                Arc::get_mut(&mut self.base),
                Arc::get_mut(&mut self.base_map),
            ) {
                let idx = *base_map.get(key)?;
                let removed = base.swap_remove(idx);
                base_map.remove(key);
                if idx < base.len() {
                    let swapped_key = key_of(&base[idx]);
                    base_map.insert(swapped_key, idx);
                }
                return Some(removed);
            }
        }

        match self.delta_map.get(key).copied() {
            Some(DeltaEntry::Present(d_idx)) => {
                let removed = self.delta_items.swap_remove(d_idx);
                if d_idx < self.delta_items.len() {
                    let swapped_key = key_of(&self.delta_items[d_idx]);
                    self.delta_map
                        .insert(swapped_key, DeltaEntry::Present(d_idx));
                }
                if self.base_map.contains_key(key) {
                    self.delta_map.insert(key.clone(), DeltaEntry::Deleted);
                } else {
                    self.delta_map.remove(key);
                }
                Some(removed)
            }
            Some(DeltaEntry::Deleted) => None,
            None => {
                let base_idx = *self.base_map.get(key)?;
                let removed = Arc::clone(&self.base[base_idx]);
                self.mark_base_shadowed(base_idx);
                self.delta_map.insert(key.clone(), DeltaEntry::Deleted);
                Some(removed)
            }
        }
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V>
    where
        K: Clone,
        V: Clone,
    {
        if self.has_no_delta() && Arc::strong_count(&self.base) == 1 {
            let idx = *self.base_map.get(key)?;
            let base = Arc::get_mut(&mut self.base)?;
            return Some(Arc::make_mut(&mut base[idx]));
        }

        match self.delta_map.get(key).copied() {
            Some(DeltaEntry::Present(d_idx)) => Some(Arc::make_mut(&mut self.delta_items[d_idx])),
            Some(DeltaEntry::Deleted) => None,
            None => {
                let base_idx = *self.base_map.get(key)?;
                let cloned = Arc::new((*self.base[base_idx]).clone());
                self.mark_base_shadowed(base_idx);
                let d_idx = self.delta_items.len();
                self.delta_items.push(cloned);
                self.delta_map
                    .insert(key.clone(), DeltaEntry::Present(d_idx));
                Some(Arc::make_mut(&mut self.delta_items[d_idx]))
            }
        }
    }

    pub fn find_mut_by<P, F>(&mut self, predicate: P, key_of: F) -> Option<&mut V>
    where
        V: Clone,
        P: Fn(&V) -> bool,
        F: Fn(&V) -> K,
    {
        if self.has_no_delta() && Arc::strong_count(&self.base) == 1 {
            let idx = self.base.iter().position(|v| predicate(v))?;
            let base = Arc::get_mut(&mut self.base)?;
            return Some(Arc::make_mut(&mut base[idx]));
        }

        if let Some(d_idx) = self.delta_items.iter().position(|v| predicate(v)) {
            return Some(Arc::make_mut(&mut self.delta_items[d_idx]));
        }

        let base_idx = self
            .base
            .iter()
            .enumerate()
            .position(|(i, v)| !self.is_base_shadowed(i) && predicate(v))?;
        let key = key_of(&self.base[base_idx]);
        let cloned = Arc::new((*self.base[base_idx]).clone());
        self.mark_base_shadowed(base_idx);
        let d_idx = self.delta_items.len();
        self.delta_items.push(cloned);
        self.delta_map.insert(key, DeltaEntry::Present(d_idx));
        Some(Arc::make_mut(&mut self.delta_items[d_idx]))
    }

    pub fn remove_by<P, F>(&mut self, predicate: P, key_of: F) -> Option<Arc<V>>
    where
        K: Clone,
        P: Fn(&V) -> bool,
        F: Fn(&V) -> K,
    {
        if self.has_no_delta() && Arc::strong_count(&self.base) == 1 {
            let idx = self.base.iter().position(|v| predicate(v))?;
            let key = key_of(&self.base[idx]);
            return self.remove(&key, key_of);
        }

        if let Some(d_idx) = self.delta_items.iter().position(|v| predicate(v)) {
            let key = key_of(&self.delta_items[d_idx]);
            return self.remove(&key, key_of);
        }

        let base_idx = self
            .base
            .iter()
            .enumerate()
            .position(|(i, v)| !self.is_base_shadowed(i) && predicate(v))?;
        let key = key_of(&self.base[base_idx]);
        self.remove(&key, key_of)
    }

    pub fn rebuild_map<F>(&mut self, key_of: F)
    where
        F: Fn(&V) -> K,
    {
        if !self.delta_map.is_empty() || !self.base_shadowed.is_empty() {
            let live: Vec<Arc<V>> = self.view().iter().map(Arc::clone).collect();
            self.base = Arc::new(live);
            self.delta_items.clear();
            self.delta_map.clear();
            self.base_shadowed.clear();
        }
        self.base_map = Arc::new(
            self.base
                .iter()
                .enumerate()
                .map(|(i, v)| (key_of(v), i))
                .collect(),
        );
    }
}

#[cfg(feature = "checkpoint")]
impl<K, V: Serialize> Serialize for CowRegistry<K, V> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let view = self.view();
        let mut seq = serializer.serialize_seq(Some(view.len()))?;
        for item in view {
            seq.serialize_element(item.as_ref())?;
        }
        seq.end()
    }
}

#[cfg(feature = "checkpoint")]
impl<'de, K, V: Deserialize<'de>> Deserialize<'de> for CowRegistry<K, V> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let items: Vec<V> = Vec::deserialize(deserializer)?;
        Ok(Self {
            base: Arc::new(items.into_iter().map(Arc::new).collect()),
            base_map: Arc::new(HashMap::new()),
            delta_items: Vec::new(),
            delta_map: HashMap::new(),
            base_shadowed: Vec::new(),
        })
    }
}

/// Zero-allocation read-only view over all live entries in a [`CowRegistry`].
#[derive(Debug)]
pub struct CowRegistryView<'a, T> {
    base: &'a [Arc<T>],
    base_shadowed: &'a [usize],
    delta_added: &'a [Arc<T>],
}

impl<T> Clone for CowRegistryView<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for CowRegistryView<'_, T> {}

impl<'a, T> CowRegistryView<'a, T> {
    #[inline]
    pub fn iter(&self) -> CowRegistryIter<'a, T> {
        CowRegistryIter {
            base: self.base,
            base_idx: 0,
            base_shadowed: self.base_shadowed,
            shadow_idx: 0,
            delta_added: self.delta_added,
            delta_idx: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.base.len() - self.base_shadowed.len() + self.delta_added.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<'a, T> IntoIterator for CowRegistryView<'a, T> {
    type Item = &'a Arc<T>;
    type IntoIter = CowRegistryIter<'a, T>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Zero-allocation iterator over live entries in a [`CowRegistryView`].
#[derive(Debug)]
pub struct CowRegistryIter<'a, T> {
    base: &'a [Arc<T>],
    base_idx: usize,
    base_shadowed: &'a [usize],
    shadow_idx: usize,
    delta_added: &'a [Arc<T>],
    delta_idx: usize,
}

impl<T> Clone for CowRegistryIter<'_, T> {
    fn clone(&self) -> Self {
        Self {
            base: self.base,
            base_idx: self.base_idx,
            base_shadowed: self.base_shadowed,
            shadow_idx: self.shadow_idx,
            delta_added: self.delta_added,
            delta_idx: self.delta_idx,
        }
    }
}

impl<'a, T> Iterator for CowRegistryIter<'a, T> {
    type Item = &'a Arc<T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.base_idx < self.base.len() {
            let idx = self.base_idx;
            self.base_idx += 1;
            if self.shadow_idx < self.base_shadowed.len()
                && self.base_shadowed[self.shadow_idx] == idx
            {
                self.shadow_idx += 1;
                continue;
            }
            return Some(&self.base[idx]);
        }
        if self.delta_idx < self.delta_added.len() {
            let item = &self.delta_added[self.delta_idx];
            self.delta_idx += 1;
            return Some(item);
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let base_rem = (self.base.len() - self.base_idx)
            .saturating_sub(self.base_shadowed.len() - self.shadow_idx);
        let total = base_rem + (self.delta_added.len() - self.delta_idx);
        (total, Some(total))
    }
}

impl<T> ExactSizeIterator for CowRegistryIter<'_, T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Item {
        id: u32,
        key: String,
        val: i32,
    }

    fn key_of(item: &Item) -> String {
        item.key.clone()
    }

    fn snapshot(reg: &CowRegistry<String, Item>) -> BTreeMap<String, Item> {
        let view = reg.view();
        let mut out = BTreeMap::new();
        let mut count = 0;
        for arc in view {
            count += 1;
            let prev = out.insert(arc.key.clone(), (**arc).clone());
            assert!(prev.is_none(), "duplicate key yielded in view: {}", arc.key);
        }
        assert_eq!(count, view.len(), "view.len() mismatch with iterated count");
        assert_eq!(view.is_empty(), count == 0);
        out
    }

    #[test]
    fn test_fork_mutations_do_not_clone_shared_base() {
        let mut base: CowRegistry<String, Item> = CowRegistry::default();
        for i in 0..100 {
            base.insert_or_replace(
                format!("k{i}"),
                Item {
                    id: i,
                    key: format!("k{i}"),
                    val: i as i32,
                },
            );
        }
        assert!(base.delta_map.is_empty());

        let base_ptr = Arc::as_ptr(&base.base);
        let map_ptr = Arc::as_ptr(&base.base_map);

        let mut fork = base.clone();
        // Drop, re-insert, mutate in place, insert new, drop new
        assert_eq!(fork.remove(&"k10".to_string(), key_of).unwrap().id, 10);
        assert!(fork.get(&"k10".to_string()).is_none());
        fork.insert_or_replace(
            "k10".to_string(),
            Item {
                id: 910,
                key: "k10".to_string(),
                val: -10,
            },
        );
        fork.get_mut(&"k20".to_string()).unwrap().val = 999;
        fork.find_mut_by(|it| it.id == 30, key_of).unwrap().val = 3030;
        fork.insert_or_replace(
            "temp".to_string(),
            Item {
                id: 500,
                key: "temp".to_string(),
                val: 5,
            },
        );
        assert!(fork.remove_by(|it| it.id == 500, key_of).is_some());

        // Shared base Arc pointers must be untouched
        assert_eq!(Arc::as_ptr(&fork.base), base_ptr);
        assert_eq!(Arc::as_ptr(&fork.base_map), map_ptr);
        assert_eq!(Arc::strong_count(&base.base), 2);
        assert_eq!(Arc::strong_count(&base.base_map), 2);

        // Base must remain completely unmodified
        assert_eq!(base.get(&"k10".to_string()).unwrap().val, 10);
        assert_eq!(base.get(&"k20".to_string()).unwrap().val, 20);
        assert_eq!(base.get(&"k30".to_string()).unwrap().val, 30);
        assert_eq!(base.view().len(), 100);

        // Fork reflects all mutations
        assert_eq!(fork.get(&"k10".to_string()).unwrap().val, -10);
        assert_eq!(fork.get(&"k20".to_string()).unwrap().val, 999);
        assert_eq!(fork.get(&"k30".to_string()).unwrap().val, 3030);
        assert!(fork.get(&"temp".to_string()).is_none());
        assert_eq!(fork.view().len(), 100);

        // Nested fork inherits parent fork's delta without disturbing parent
        let mut child = fork.clone();
        assert!(child.remove(&"k10".to_string(), key_of).is_some());
        assert!(child.get(&"k10".to_string()).is_none());
        assert_eq!(fork.get(&"k10".to_string()).unwrap().val, -10);
        assert_eq!(child.view().len(), 99);
        assert_eq!(fork.view().len(), 100);

        // Serde round-trip preserves forked state cleanly
        let encoded = serde_json::to_string(&child).unwrap();
        let mut decoded: CowRegistry<String, Item> = serde_json::from_str(&encoded).unwrap();
        decoded.rebuild_map(key_of);
        assert_eq!(snapshot(&decoded), snapshot(&child));
    }

    #[test]
    fn test_cow_registry_randomized_differential_oracle() {
        let mut state: u64 = 0x517C_C1B7_2722_0A95;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut regs: Vec<CowRegistry<String, Item>> = vec![CowRegistry::default()];
        let mut oracles: Vec<BTreeMap<String, Item>> = vec![BTreeMap::new()];
        let mut next_id: u32 = 1;

        for step in 0..10_000 {
            let slot = (next() as usize) % regs.len();
            let k_idx = (next() as usize) % 24;
            let key = format!("k{k_idx}");
            match (next() % 9) as u8 {
                // Insert or replace
                0 | 1 => {
                    let item = Item {
                        id: next_id,
                        key: key.clone(),
                        val: (next() & 0xFFFF) as i32,
                    };
                    next_id += 1;
                    regs[slot].insert_or_replace(key.clone(), item.clone());
                    oracles[slot].insert(key, item);
                }
                // Remove by key
                2 => {
                    let got = regs[slot].remove(&key, key_of).map(|a| (*a).clone());
                    let exp = oracles[slot].remove(&key);
                    assert_eq!(got, exp, "remove mismatch at step {step}");
                }
                // Mutate by key
                3 => {
                    let new_val = (next() & 0xFFFF) as i32;
                    let got = regs[slot].get_mut(&key).map(|it| {
                        it.val = new_val;
                        it.id
                    });
                    let exp = oracles[slot].get_mut(&key).map(|it| {
                        it.val = new_val;
                        it.id
                    });
                    assert_eq!(got, exp, "get_mut mismatch at step {step}");
                }
                // Mutate by predicate (id)
                4 => {
                    if let Some(target_id) = oracles[slot].get(&key).map(|it| it.id) {
                        let new_val = (next() & 0xFFFF) as i32;
                        let got =
                            regs[slot]
                                .find_mut_by(|it| it.id == target_id, key_of)
                                .map(|it| {
                                    it.val = new_val;
                                    it.id
                                });
                        let exp = oracles[slot].get_mut(&key).map(|it| {
                            it.val = new_val;
                            it.id
                        });
                        assert_eq!(got, exp, "find_mut_by mismatch at step {step}");
                    }
                }
                // Remove by predicate (id)
                5 => {
                    if let Some(target_id) = oracles[slot].get(&key).map(|it| it.id) {
                        let got = regs[slot]
                            .remove_by(|it| it.id == target_id, key_of)
                            .map(|a| (*a).clone());
                        let exp = oracles[slot].remove(&key);
                        assert_eq!(got, exp, "remove_by mismatch at step {step}");
                    }
                }
                // Clone into a fork (keep up to 6 live forks)
                6 => {
                    if regs.len() < 6 {
                        let cloned_reg = regs[slot].clone();
                        let cloned_oracle = oracles[slot].clone();
                        regs.push(cloned_reg);
                        oracles.push(cloned_oracle);
                    } else {
                        let dst = (next() as usize) % regs.len();
                        regs[dst] = regs[slot].clone();
                        oracles[dst] = oracles[slot].clone();
                    }
                }
                // Rebuild map (flattens delta)
                7 => {
                    regs[slot].rebuild_map(key_of);
                }
                // Point lookup check
                _ => {
                    assert_eq!(regs[slot].get(&key), oracles[slot].get(&key));
                    assert_eq!(
                        regs[slot].contains_key(&key),
                        oracles[slot].contains_key(&key)
                    );
                    assert_eq!(regs[slot].get_arc(&key).as_deref(), oracles[slot].get(&key));
                }
            }

            if step % 50 == 0 {
                for (r, o) in regs.iter().zip(oracles.iter()) {
                    assert_eq!(snapshot(r), *o, "full snapshot mismatch at step {step}");
                }
            }
        }

        for (r, o) in regs.iter().zip(oracles.iter()) {
            assert_eq!(snapshot(r), *o);
        }
    }
}
