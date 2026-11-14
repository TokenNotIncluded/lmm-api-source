//! Process faults for owned HTTP fixtures only; never compiled into production.
use std::{
    collections::BTreeSet,
    sync::{Mutex, OnceLock},
};
pub(super) fn crash_at(stage: &str) {
    if std::env::var("LMM_RESPONSES_CONTINUITY_CRASH_AT").as_deref() == Ok(stage) {
        std::process::exit(97);
    }
}
/// Drop the application acknowledgement after a real commit. Actual wire ACK
/// loss is separately exercised by the reviewed PostgreSQL proxy test support.
pub(super) fn lose_ack_once(stage: &str) -> bool {
    static LOST: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    if std::env::var("LMM_RESPONSES_CONTINUITY_LOSE_ACK_AT").as_deref() == Ok(stage) {
        return LOST
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(stage.into());
    }
    false
}
