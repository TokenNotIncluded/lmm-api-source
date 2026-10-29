//! Independent business assertions against the actual frozen History library.
//! PostgreSQL, CLI setup, target restore and transport fault belong to run.py.
use lmm_core::upgrade_history::{Binding, History, HistoryError, Phase, Snapshot};
use serde::Deserialize;
use serde_json::json;
use std::{env, fs};

#[derive(Deserialize)]
struct Fixture {
    writer_url: String,
    authority_id: String,
    authority_system_id: String,
    normal: Binding,
    lost: Binding,
    fence: String,
    receipt: String,
    proxy_url: Option<String>,
}

impl Fixture {
    async fn connect(&self) -> History {
        History::connect(
            &self.writer_url,
            &self.authority_id,
            &self.normal.target_system_identifier,
        )
        .await
        .expect("real restricted writer must connect to the separate authority")
    }

    fn expected(&self, binding: &Binding, sequence: i64) -> Snapshot {
        Snapshot {
            site_id: binding.site_id.clone(),
            operation_id: binding.operation_id.clone(),
            binding: binding.clone(),
            sequence,
            phase: match sequence {
                1 => Phase::Closing,
                2 => Phase::Held,
                3 => Phase::Started,
                4 => Phase::Committed,
                _ => panic!("test oracle has invalid sequence {sequence}"),
            },
            fence_sha256: (sequence >= 2).then(|| self.fence.clone()),
            receipt_sha256: (sequence == 4).then(|| self.receipt.clone()),
        }
    }

    async fn observe(&self, history: &mut History, binding: &Binding, sequence: i64) {
        assert_eq!(
            history.snapshot(binding).await.expect("snapshot read"),
            Some(self.expected(binding, sequence)),
            "durable snapshot must equal independent original intent and expected phase"
        );
    }

    async fn immutable(&self, history: &mut History, binding: &Binding, sequence: i64) {
        for field in [
            "operation",
            "request",
            "candidate",
            "database",
            "oid",
            "system",
        ] {
            let mut changed = binding.clone();
            match field {
                "operation" => changed.operation_id = "replacement-operation-0009".into(),
                "request" => changed.request_sha256 = "f".repeat(64),
                "candidate" => changed.candidate_sha256 = "e".repeat(64),
                "database" => changed.target_database = "replacement_target".into(),
                "oid" => changed.target_database_oid += 1,
                "system" => changed.target_system_identifier = "991234567".into(),
                _ => unreachable!("fixed field list"),
            }
            for outcome in [
                history.reserve(&changed).await,
                history
                    .snapshot(&changed)
                    .await
                    .map(|s| s.expect("existing site")),
            ] {
                if field == "system" {
                    assert!(
                        matches!(outcome, Err(HistoryError::Identity)),
                        "changed physical target accepted"
                    );
                } else {
                    assert!(
                        matches!(outcome, Err(HistoryError::Conflict)),
                        "original {field} binding replaced"
                    );
                }
            }
            self.observe(history, binding, sequence).await;
        }
    }
}

fn conflict<T: std::fmt::Debug>(result: Result<T, HistoryError>, context: &str) {
    assert!(
        matches!(result, Err(HistoryError::Conflict)),
        "{context}: {result:?}"
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: independent <stage> <private fixture.json>"
    );
    let fixture: Fixture =
        serde_json::from_slice(&fs::read(&args[2]).expect("fixture input")).expect("fixture JSON");
    let mut h = fixture.connect().await;
    match args[1].as_str() {
        "reserve" => {
            assert!(
                matches!(
                    History::connect(
                        &fixture.writer_url,
                        &"0".repeat(64),
                        &fixture.normal.target_system_identifier
                    )
                    .await,
                    Err(HistoryError::Identity)
                ),
                "wrong authority accepted"
            );
            assert!(
                matches!(
                    History::connect(
                        &fixture.writer_url,
                        &fixture.authority_id,
                        &fixture.authority_system_id
                    )
                    .await,
                    Err(HistoryError::Identity)
                ),
                "authority on target physical cluster accepted"
            );
            for b in [&fixture.normal, &fixture.lost] {
                assert_eq!(h.snapshot(b).await.expect("empty observation"), None);
                assert_eq!(h.reserve(b).await.expect("reserve"), fixture.expected(b, 1));
                assert_eq!(
                    h.reserve(b).await.expect("identical reserve"),
                    fixture.expected(b, 1)
                );
                conflict(h.start(b).await, "closing cannot consume");
                conflict(
                    h.committed(b, &fixture.receipt).await,
                    "closing cannot declare commit",
                );
                fixture.immutable(&mut h, b, 1).await;
            }
        }
        "hold" => {
            for b in [&fixture.normal, &fixture.lost] {
                fixture.observe(&mut h, b, 1).await;
                assert_eq!(
                    h.held(b, &fixture.fence)
                        .await
                        .expect("record synthetic fence fact"),
                    fixture.expected(b, 2)
                );
                conflict(
                    h.held(b, &fixture.fence).await,
                    "held transition cannot repeat",
                );
                conflict(
                    h.committed(b, &fixture.receipt).await,
                    "held cannot skip consumption",
                );
            }
        }
        "race" => {
            let (mut a, mut b, mut c, mut d) = tokio::join!(
                fixture.connect(),
                fixture.connect(),
                fixture.connect(),
                fixture.connect()
            );
            let outcomes = tokio::join!(
                a.start(&fixture.normal),
                b.start(&fixture.normal),
                c.start(&fixture.normal),
                d.start(&fixture.normal)
            );
            let outcomes = [outcomes.0, outcomes.1, outcomes.2, outcomes.3];
            assert_eq!(
                outcomes.iter().filter(|r| r.is_ok()).count(),
                1,
                "exactly one consumer owns the original intent"
            );
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|r| matches!(r, Err(HistoryError::Conflict)))
                    .count(),
                3,
                "all other consumers must receive CAS conflict"
            );
            for snapshot in outcomes.into_iter().filter_map(Result::ok) {
                assert_eq!(snapshot, fixture.expected(&fixture.normal, 3));
            }
            conflict(h.start(&fixture.normal).await, "fifth consumer denied");
            fixture.observe(&mut h, &fixture.normal, 3).await;
            fixture.observe(&mut h, &fixture.lost, 2).await;
        }
        "commit" => {
            fixture.observe(&mut h, &fixture.normal, 3).await;
            assert_eq!(
                h.committed(&fixture.normal, &fixture.receipt)
                    .await
                    .expect("record receipt of actual synthetic target transaction"),
                fixture.expected(&fixture.normal, 4)
            );
            conflict(
                h.committed(&fixture.normal, &"d".repeat(64)).await,
                "existing receipt cannot be overwritten",
            );
            fixture.observe(&mut h, &fixture.normal, 4).await;
        }
        "fault" => {
            fixture.observe(&mut h, &fixture.lost, 2).await;
            let mut disconnected = History::connect(
                fixture.proxy_url.as_deref().expect("real fault proxy"),
                &fixture.authority_id,
                &fixture.lost.target_system_identifier,
            )
            .await
            .expect("proxy connection");
            assert!(
                matches!(
                    disconnected.start(&fixture.lost).await,
                    Err(HistoryError::CommitUnknown)
                ),
                "lost COMMIT ACK must remain unknown to caller"
            );
            assert!(
                matches!(
                    disconnected.snapshot(&fixture.lost).await,
                    Err(HistoryError::Storage)
                ),
                "dead connection must not invent empty history"
            );
            drop(disconnected);
            let mut reconnected = fixture.connect().await;
            fixture.observe(&mut reconnected, &fixture.lost, 3).await;
            assert_eq!(
                reconnected
                    .reserve(&fixture.lost)
                    .await
                    .expect("reserve observes tombstone"),
                fixture.expected(&fixture.lost, 3)
            );
            conflict(
                reconnected.start(&fixture.lost).await,
                "unknown acknowledgement cannot allow retry",
            );
        }
        "no_replay" => {
            for (b, sequence) in [(&fixture.normal, 4), (&fixture.lost, 3)] {
                fixture.observe(&mut h, b, sequence).await;
                assert_eq!(
                    h.reserve(b)
                        .await
                        .expect("reserve must observe original phase"),
                    fixture.expected(b, sequence)
                );
                conflict(
                    h.start(b).await,
                    "target restore/reconnect cannot create a second consumer",
                );
                conflict(h.held(b, &fixture.fence).await, "cannot reset to held");
                fixture.immutable(&mut h, b, sequence).await;
            }
        }
        other => panic!("unknown test stage {other}"),
    }
    println!(
        "{}",
        json!({"stage":args[1],"status":"PASS","normal":h.snapshot(&fixture.normal).await.expect("final normal observation"),"lost":h.snapshot(&fixture.lost).await.expect("final lost observation")})
    );
}
