use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AppLockMode {
    IntentShared,
    Shared,
    Update,
    IntentExclusive,
    SharedIntentExclusive,
    UpdateIntentExclusive,
    Exclusive,
}

impl AppLockMode {
    pub(crate) fn parse_request(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "intentshared" => Some(Self::IntentShared),
            "shared" => Some(Self::Shared),
            "update" => Some(Self::Update),
            "intentexclusive" => Some(Self::IntentExclusive),
            "exclusive" => Some(Self::Exclusive),
            _ => None,
        }
    }

    pub(crate) fn as_sql_str(self) -> &'static str {
        match self {
            Self::IntentShared => "IntentShared",
            Self::Shared => "Shared",
            Self::Update => "Update",
            Self::IntentExclusive => "IntentExclusive",
            Self::SharedIntentExclusive => "SharedIntentExclusive",
            Self::UpdateIntentExclusive => "UpdateIntentExclusive",
            Self::Exclusive => "Exclusive",
        }
    }

    /// Combines two lock modes held by the same owner into the effective strongest mode
    /// (matching SQL Server's lock conversion matrix).
    pub(crate) fn combine(self, other: Self) -> Self {
        use AppLockMode::*;
        if self == other {
            return self;
        }
        match (self, other) {
            (Exclusive, _) | (_, Exclusive) => Exclusive,
            (UpdateIntentExclusive, _) | (_, UpdateIntentExclusive) => UpdateIntentExclusive,
            (SharedIntentExclusive, Update) | (Update, SharedIntentExclusive) => {
                UpdateIntentExclusive
            }
            (SharedIntentExclusive, _) | (_, SharedIntentExclusive) => SharedIntentExclusive,
            (Update, IntentExclusive) | (IntentExclusive, Update) => UpdateIntentExclusive,
            (Shared, IntentExclusive) | (IntentExclusive, Shared) => SharedIntentExclusive,
            (Update, Shared) | (Shared, Update) => Update,
            (Update, IntentShared) | (IntentShared, Update) => Update,
            (IntentExclusive, IntentShared) | (IntentShared, IntentExclusive) => IntentExclusive,
            (Shared, IntentShared) | (IntentShared, Shared) => Shared,
            _ => Exclusive,
        }
    }

    /// Returns `true` if `self` (held by another session) is compatible with `requested`.
    pub(crate) fn is_compatible_with(self, requested: Self) -> bool {
        use AppLockMode::*;
        match (self, requested) {
            (IntentShared, Exclusive) | (Exclusive, _) => false,
            (IntentShared, _) => true,
            (Shared, IntentShared | Shared | Update) => true,
            (Shared, _) => false,
            (Update, IntentShared | Shared) => true,
            (Update, _) => false,
            (IntentExclusive, IntentShared | IntentExclusive) => true,
            (IntentExclusive, _) => false,
            (SharedIntentExclusive | UpdateIntentExclusive, IntentShared) => true,
            (SharedIntentExclusive | UpdateIntentExclusive, _) => false,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct AppLockRegistry {
    /// Keyed by `(session_id, principal_lower, resource_lower, owner_lower)` -> stack of acquired modes.
    locks: HashMap<(u64, String, String, String), Vec<AppLockMode>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SharedAppLocks {
    inner: Arc<Mutex<AppLockRegistry>>,
}

impl SharedAppLocks {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AppLockRegistry::default())),
        }
    }

    pub(crate) fn release_all_for_session(&self, session_id: u64) {
        self.inner
            .lock()
            .locks
            .retain(|(sid, _, _, _), _| *sid != session_id);
    }

    pub(crate) fn release_transaction_locks(&self, session_id: u64) {
        self.inner
            .lock()
            .locks
            .retain(|(sid, _, _, owner), _| *sid != session_id || owner != "transaction");
    }

    fn key(
        session_id: u64,
        principal: &str,
        resource: &str,
        owner: &str,
    ) -> (u64, String, String, String) {
        (
            session_id,
            principal.trim().to_ascii_lowercase(),
            resource.to_ascii_lowercase(),
            owner.trim().to_ascii_lowercase(),
        )
    }

    pub(crate) fn effective_mode(
        &self,
        session_id: u64,
        principal: &str,
        resource: &str,
        owner: &str,
    ) -> &'static str {
        let guard = self.inner.lock();
        let key = Self::key(session_id, principal, resource, owner);
        let Some(stack) = guard.locks.get(&key) else {
            return "NoLock";
        };
        let mut iter = stack.iter().copied();
        let Some(first) = iter.next() else {
            return "NoLock";
        };
        let combined = iter.fold(first, AppLockMode::combine);
        combined.as_sql_str()
    }

    /// SQL Server semantics: locks held by the SAME session never block each other
    /// (even across Session vs Transaction owners); locks held by OTHER sessions block
    /// according to the lock-mode compatibility matrix.
    pub(crate) fn can_grant(
        &self,
        session_id: u64,
        principal: &str,
        resource: &str,
        mode: AppLockMode,
    ) -> bool {
        let guard = self.inner.lock();
        let p_low = principal.trim().to_ascii_lowercase();
        let r_low = resource.to_ascii_lowercase();
        for ((sid, p, r, _), held_modes) in &guard.locks {
            if *sid != session_id && p == &p_low && r == &r_low {
                if let Some(first) = held_modes.first().copied() {
                    let effective = held_modes
                        .iter()
                        .skip(1)
                        .copied()
                        .fold(first, AppLockMode::combine);
                    if !effective.is_compatible_with(mode) {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub(crate) fn acquire(
        &self,
        session_id: u64,
        principal: &str,
        resource: &str,
        mode: AppLockMode,
        owner: &str,
    ) -> i32 {
        let mut guard = self.inner.lock();
        let p_low = principal.trim().to_ascii_lowercase();
        let r_low = resource.to_ascii_lowercase();
        for ((sid, p, r, _), held_modes) in &guard.locks {
            if *sid != session_id && p == &p_low && r == &r_low {
                if let Some(first) = held_modes.first().copied() {
                    let effective = held_modes
                        .iter()
                        .skip(1)
                        .copied()
                        .fold(first, AppLockMode::combine);
                    if !effective.is_compatible_with(mode) {
                        return -1;
                    }
                }
            }
        }
        let key = (session_id, p_low, r_low, owner.trim().to_ascii_lowercase());
        guard.locks.entry(key).or_default().push(mode);
        0
    }

    pub(crate) fn release(
        &self,
        session_id: u64,
        principal: &str,
        resource: &str,
        owner: &str,
    ) -> i32 {
        let mut guard = self.inner.lock();
        let key = Self::key(session_id, principal, resource, owner);
        if let Some(stack) = guard.locks.get_mut(&key) {
            stack.pop();
            if stack.is_empty() {
                guard.locks.remove(&key);
            }
            0
        } else {
            -999
        }
    }
}
