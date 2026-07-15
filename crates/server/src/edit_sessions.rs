//! In-process record working copies and owned edit locks (#182).
//!
//! Canonical user rows are never used as edit buffers. Existing-record sessions
//! snapshot the complete row and hold an owned lock until commit/revert. Pending
//! inserts live only in this registry, so process loss is an implicit revert.

use std::collections::{hash_map::RandomState, HashMap};
use std::fmt;
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::{Duration, Instant};

/// A concrete record-edit scope. Route coordinates ensure a token opened for one
/// record or portal cannot be replayed against another endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditScope {
    Base {
        layout_id: i64,
        table_id: i64,
        record_id: i64,
    },
    PendingBase {
        layout_id: i64,
        table_id: i64,
        synthetic_id: i64,
    },
    Related {
        layout_id: i64,
        base_id: i64,
        object_id: i64,
        table_id: i64,
        record_id: i64,
    },
    PendingRelated {
        layout_id: i64,
        base_id: i64,
        object_id: i64,
        table_id: i64,
    },
}

impl EditScope {
    fn lock_key(&self) -> Option<(i64, i64)> {
        match *self {
            Self::Base {
                table_id,
                record_id,
                ..
            }
            | Self::Related {
                table_id,
                record_id,
                ..
            } => Some((table_id, record_id)),
            Self::PendingBase { .. } | Self::PendingRelated { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EditSession {
    pub(crate) token: String,
    pub(crate) owner: String,
    pub(crate) scope: EditScope,
    pub(crate) original: HashMap<i64, String>,
    pub(crate) working: HashMap<i64, String>,
    pub(crate) last_activity: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionError {
    Locked,
    Unknown,
    WrongOwner,
    WrongScope,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => f.write_str("record is already open in another editing session"),
            Self::Unknown => f.write_str("edit session is no longer active"),
            Self::WrongOwner => f.write_str("edit session belongs to another owner"),
            Self::WrongScope => f.write_str("edit session does not match this record"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct EditSessionRegistry {
    sessions: HashMap<String, EditSession>,
    locks: HashMap<(i64, i64), String>,
    next: u64,
    lease: Duration,
    token_seed: RandomState,
}

impl Default for EditSessionRegistry {
    fn default() -> Self {
        Self::new(Duration::from_secs(30 * 60))
    }
}

impl EditSessionRegistry {
    pub(crate) fn new(lease: Duration) -> Self {
        Self {
            sessions: HashMap::new(),
            locks: HashMap::new(),
            next: 1,
            lease,
            token_seed: RandomState::new(),
        }
    }

    fn token(&mut self) -> String {
        let seq = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        let mut high = self.token_seed.build_hasher();
        ("record-edit-high", seq).hash(&mut high);
        let mut low = self.token_seed.build_hasher();
        ("record-edit-low", seq).hash(&mut low);
        format!("e{:016x}{:016x}", high.finish(), low.finish())
    }

    fn synthetic_id(&self) -> i64 {
        -(self.next.min(i64::MAX as u64) as i64)
    }

    pub(crate) fn begin_existing(
        &mut self,
        owner: String,
        scope: EditScope,
        values: HashMap<i64, String>,
    ) -> Result<EditSession, SessionError> {
        self.expire();
        let key = scope.lock_key().expect("existing scope has a lock key");
        if let Some(token) = self.locks.get(&key) {
            let existing = self
                .sessions
                .get_mut(token)
                .expect("lock/session stay paired");
            if existing.owner != owner {
                return Err(SessionError::Locked);
            }
            if existing.scope != scope {
                return Err(SessionError::WrongScope);
            }
            existing.last_activity = Instant::now();
            eprintln!(
                "[record-edit] event=reopen token={} scope={:?}",
                existing.token, existing.scope
            );
            return Ok(existing.clone());
        }

        let token = self.token();
        let session = EditSession {
            token: token.clone(),
            owner,
            scope,
            original: values.clone(),
            working: values,
            last_activity: Instant::now(),
        };
        self.locks.insert(key, token.clone());
        self.sessions.insert(token, session.clone());
        eprintln!(
            "[record-edit] event=open token={} scope={:?}",
            session.token, session.scope
        );
        Ok(session)
    }

    pub(crate) fn begin_pending_base(
        &mut self,
        owner: String,
        layout_id: i64,
        table_id: i64,
        values: HashMap<i64, String>,
    ) -> EditSession {
        self.expire();
        let synthetic_id = self.synthetic_id();
        let token = self.token();
        let session = EditSession {
            token: token.clone(),
            owner,
            scope: EditScope::PendingBase {
                layout_id,
                table_id,
                synthetic_id,
            },
            original: values.clone(),
            working: values,
            last_activity: Instant::now(),
        };
        self.sessions.insert(token, session.clone());
        eprintln!(
            "[record-edit] event=open_pending token={} scope={:?}",
            session.token, session.scope
        );
        session
    }

    pub(crate) fn begin_pending_related(
        &mut self,
        owner: String,
        layout_id: i64,
        base_id: i64,
        object_id: i64,
        table_id: i64,
        values: HashMap<i64, String>,
    ) -> EditSession {
        self.expire();
        let token = self.token();
        let session = EditSession {
            token: token.clone(),
            owner,
            scope: EditScope::PendingRelated {
                layout_id,
                base_id,
                object_id,
                table_id,
            },
            original: values.clone(),
            working: values,
            last_activity: Instant::now(),
        };
        self.sessions.insert(token, session.clone());
        eprintln!(
            "[record-edit] event=open_pending token={} scope={:?}",
            session.token, session.scope
        );
        session
    }

    pub(crate) fn overlay(
        &mut self,
        token: &str,
        owner: &str,
        scope: &EditScope,
        values: impl IntoIterator<Item = (i64, String)>,
    ) -> Result<EditSession, SessionError> {
        self.expire();
        let session = self.sessions.get_mut(token).ok_or(SessionError::Unknown)?;
        if session.owner != owner {
            return Err(SessionError::WrongOwner);
        }
        if &session.scope != scope {
            return Err(SessionError::WrongScope);
        }
        session.working.extend(values);
        session.last_activity = Instant::now();
        Ok(session.clone())
    }

    pub(crate) fn pending_base(
        &mut self,
        token: &str,
        layout_id: i64,
        table_id: i64,
    ) -> Option<EditSession> {
        self.expire();
        let session = self.sessions.get_mut(token)?;
        match session.scope {
            EditScope::PendingBase {
                layout_id: l,
                table_id: t,
                ..
            } if l == layout_id && t == table_id => {
                session.last_activity = Instant::now();
                Some(session.clone())
            }
            _ => None,
        }
    }

    pub(crate) fn release(
        &mut self,
        token: &str,
        owner: &str,
        scope: &EditScope,
    ) -> Result<EditSession, SessionError> {
        let session = self.sessions.get(token).ok_or(SessionError::Unknown)?;
        if session.owner != owner {
            return Err(SessionError::WrongOwner);
        }
        if &session.scope != scope {
            return Err(SessionError::WrongScope);
        }
        let session = self.sessions.remove(token).expect("checked above");
        if let Some(key) = session.scope.lock_key() {
            self.locks.remove(&key);
        }
        Ok(session)
    }

    pub(crate) fn locked(&mut self, key: (i64, i64)) -> bool {
        self.expire();
        self.locks.contains_key(&key)
    }

    pub(crate) fn expire(&mut self) -> usize {
        let now = Instant::now();
        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| now.duration_since(session.last_activity) >= self.lease)
            .map(|(token, _)| token.clone())
            .collect();
        for token in &expired {
            if let Some(session) = self.sessions.remove(token) {
                if let Some(key) = session.scope.lock_key() {
                    self.locks.remove(&key);
                }
                eprintln!(
                    "[record-edit] event=lease_expired token={} scope={:?}",
                    session.token, session.scope
                );
            }
        }
        expired.len()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(id: i64) -> EditScope {
        EditScope::Base {
            layout_id: 2,
            table_id: 3,
            record_id: id,
        }
    }

    #[test]
    fn locks_are_owned_and_reopen_is_idempotent() {
        let mut registry = EditSessionRegistry::default();
        let first = registry
            .begin_existing("owner-a".into(), base(4), HashMap::new())
            .unwrap();
        let reopened = registry
            .begin_existing("owner-a".into(), base(4), HashMap::new())
            .unwrap();
        assert_eq!(first.token, reopened.token);
        assert!(matches!(
            registry.begin_existing("owner-b".into(), base(4), HashMap::new()),
            Err(SessionError::Locked)
        ));
        registry.release(&first.token, "owner-a", &base(4)).unwrap();
        assert!(!registry.locked((3, 4)));
    }

    #[test]
    fn owner_and_scope_are_required_for_mutation() {
        let mut registry = EditSessionRegistry::default();
        let session = registry
            .begin_existing("owner-a".into(), base(4), HashMap::new())
            .unwrap();
        assert!(matches!(
            registry.overlay(&session.token, "owner-b", &base(4), [(9, "x".into())]),
            Err(SessionError::WrongOwner)
        ));
        assert!(matches!(
            registry.release(&session.token, "owner-a", &base(5)),
            Err(SessionError::WrongScope)
        ));
        assert!(registry.locked((3, 4)));
    }

    #[test]
    fn pending_records_are_only_working_copies() {
        let mut registry = EditSessionRegistry::default();
        let session = registry.begin_pending_base("owner".into(), 2, 3, HashMap::new());
        assert!(matches!(
            session.scope,
            EditScope::PendingBase { synthetic_id, .. } if synthetic_id < 0
        ));
        assert!(registry.pending_base(&session.token, 2, 3).is_some());
        registry
            .release(&session.token, "owner", &session.scope)
            .unwrap();
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn expired_sessions_release_owned_locks_without_writes() {
        let mut registry = EditSessionRegistry::new(Duration::ZERO);
        registry
            .begin_existing("owner".into(), base(4), HashMap::new())
            .unwrap();
        assert!(!registry.locked((3, 4)));
        assert_eq!(registry.len(), 0);
    }
}
