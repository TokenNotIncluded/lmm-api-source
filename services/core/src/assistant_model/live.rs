//! Bounded observations of existing authorized model workers. This registry has
//! no execution, retry or financial authority and does not survive a restart.
use super::{AssistantModel, Binding, Error, actor, at};
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Key {
    Personal(String),
    Agent(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Model,
    Tool,
    End,
}
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub sequence: u64,
    pub round: u32,
    pub text: String,
    pub phase: Phase,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Subject {
    pub key: Key,
    pub owner_user_id: i64,
    pub text_sha256: Vec<u8>,
}
struct Slot {
    subject: Subject,
    state: watch::Sender<Snapshot>,
    cancel: watch::Sender<bool>,
}
#[derive(Clone)]
pub(crate) struct Registry {
    slots: Arc<Mutex<HashMap<Key, Arc<Slot>>>>,
    capacity: usize,
}
pub(crate) struct Worker {
    registry: Registry,
    slot: Arc<Slot>,
}
impl Registry {
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: Arc::new(Mutex::new(HashMap::new())),
            capacity,
        }
    }
    // Called only after the durable reservation and worker claim COMMIT is known.
    pub fn register(&self, subject: Subject, round: u32) -> Result<Worker, Error> {
        let mut slots = self.slots.lock().map_err(|_| Error::Unavailable)?;
        if slots.len() >= self.capacity || slots.contains_key(&subject.key) {
            return Err(Error::Conflict);
        }
        let slot = Arc::new(Slot {
            subject,
            state: watch::channel(Snapshot {
                sequence: 1,
                round,
                text: String::new(),
                phase: Phase::Model,
            })
            .0,
            cancel: watch::channel(false).0,
        });
        slots.insert(slot.subject.key.clone(), slot.clone());
        Ok(Worker {
            registry: self.clone(),
            slot,
        })
    }
    pub fn subscribe(&self, subject: &Subject) -> Result<watch::Receiver<Snapshot>, Error> {
        let slots = self.slots.lock().map_err(|_| Error::Unavailable)?;
        let slot = slots.get(&subject.key).ok_or(Error::NotFound)?;
        if slot.subject != *subject || *slot.cancel.borrow() {
            return Err(Error::Forbidden);
        }
        Ok(slot.state.subscribe())
    }
    pub fn cancel(&self, key: &Key) {
        if let Ok(slots) = self.slots.lock()
            && let Some(slot) = slots.get(key)
        {
            slot.cancel.send_replace(true);
        }
    }
}
impl Worker {
    pub fn text(&self, text: &str) -> Result<(), Error> {
        if text.len() > 32768 || *self.slot.cancel.borrow() {
            return Err(Error::Forbidden);
        }
        let mut next = self.slot.state.borrow().clone();
        if next.phase != Phase::Model || !text.starts_with(&next.text) {
            return Err(Error::Invalid);
        }
        if text != next.text {
            next.text = text.to_owned();
            next.sequence += 1;
            self.slot.state.send_replace(next);
        }
        Ok(())
    }
    pub fn tool(&self) {
        let mut next = self.slot.state.borrow().clone();
        next.phase = Phase::Tool;
        next.text.clear();
        next.sequence += 1;
        self.slot.state.send_replace(next);
    }
    pub async fn cancelled(&self) {
        let mut receiver = self.slot.cancel.subscribe();
        loop {
            let cancelled = *receiver.borrow_and_update();
            if cancelled || receiver.changed().await.is_err() {
                return;
            }
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut next = self.slot.state.borrow().clone();
        next.phase = Phase::End;
        next.text.clear();
        next.sequence += 1;
        self.slot.state.send_replace(next);
        if let Ok(mut slots) = self.registry.slots.lock() {
            slots.remove(&self.slot.subject.key);
        }
    }
}

impl AssistantModel {
    pub(crate) async fn output_subject(
        &self,
        session: &str,
        binding: &Binding,
    ) -> Result<Subject, Error> {
        let mut tx = self.begin().await?;
        let principal = actor(&mut tx, session).await?;
        // A status receipt can be read after login again; live text is stricter:
        // the original Session, grant, input and current auth version must match.
        let row = sqlx::query("SELECT g.*,c.owner_user_id,c.owner_account_id,u.auth_version AS current_version,o.operation_key,o.text_sha256 FROM core_assistant.grants g JOIN core_assistant.conversations c ON c.id=g.conversation_id JOIN core_identity.users u ON u.id=c.owner_user_id JOIN core_assistant.operations o ON o.grant_id=g.id WHERE g.id=$1")
            .bind(&binding.delegation_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if binding.round != 0
            || binding.operation_key != format!("{}:0", binding.turn_id)
            || binding.text_sha256.len() != 32
            || row.try_get::<String, _>("conversation_id")? != binding.core_conversation_id
            || row.try_get::<String, _>("turn_id")? != binding.turn_id
            || row.try_get::<String, _>("operation_key")? != binding.operation_key
            || row.try_get::<Vec<u8>, _>("text_sha256")? != binding.text_sha256
            || row.try_get::<i64, _>("owner_user_id")? != principal.user_id
            || row.try_get::<i64, _>("owner_account_id")? != principal.owner_account_id
            || row.try_get::<i64, _>("session_id")? != principal.credential_id
            || row.try_get::<i64, _>("auth_version")? != row.try_get::<i64, _>("current_version")?
            || row.try_get::<bool, _>("revoked")?
            || row.try_get::<i64, _>("expires_at")? <= at(&mut tx).await?
        {
            return Err(Error::Forbidden);
        }
        tx.commit().await?;
        Ok(Subject {
            key: Key::Personal(binding.delegation_id.clone()),
            owner_user_id: principal.user_id,
            text_sha256: binding.text_sha256.clone(),
        })
    }
}
