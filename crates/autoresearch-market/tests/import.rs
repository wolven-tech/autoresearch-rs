//! Provenance, read-only behavior, and selection isolation for receipts.

use autoresearch_core::{
    Complexity, DecisionReason, Disposition, EvaluationSnapshot, Measurement, MetricDirection,
    NumericMetricKind, select_candidate,
};
use autoresearch_market::{
    DigestStatus, MarketReceipt, ReceiptDeclaration, ReceiptSource, ReceiptType, import_receipts,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct EvidenceRoot(PathBuf);

impl EvidenceRoot {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "autoresearch-market-test-{}-{nanos}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create evidence root");
        Self(path)
    }

    fn write_receipt(&self, name: &str, declaration: &ReceiptDeclaration) -> PathBuf {
        let relative = PathBuf::from(name);
        fs::write(
            self.0.join(&relative),
            serde_json::to_vec(declaration).expect("encode declaration"),
        )
        .expect("write sidecar");
        relative
    }
}

impl Drop for EvidenceRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("autoresearch-market-test-"))
        {
            fs::remove_dir_all(&self.0).expect("remove owned fixture");
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}

fn declaration(receipt_type: ReceiptType, source: ReceiptSource, sha: &str) -> ReceiptDeclaration {
    ReceiptDeclaration {
        schema_version: 1,
        receipt_type,
        source,
        occurred_at_unix_ms: 1_789_300_000_000,
        evidence_sha256: sha.into(),
    }
}

#[test]
fn imports_five_declared_categories_and_checks_local_digest_without_writes() {
    let root = EvidenceRoot::new();
    let source = b"synthetic receipt bytes";
    fs::write(root.0.join("source.txt"), source).expect("write source");
    let categories = [
        ReceiptType::Payment,
        ReceiptType::Fulfilment,
        ReceiptType::Refund,
        ReceiptType::QualifiedUse,
        ReceiptType::Outreach,
    ];
    let paths = categories
        .into_iter()
        .enumerate()
        .map(|(index, category)| {
            root.write_receipt(
                &format!("receipt-{index}.json"),
                &declaration(
                    category,
                    ReceiptSource::File {
                        path: "source.txt".into(),
                    },
                    &digest(source),
                ),
            )
        })
        .collect::<Vec<_>>();
    let before = paths
        .iter()
        .map(|path| fs::read(root.0.join(path)).expect("sidecar before import"))
        .collect::<Vec<_>>();
    let ledger = import_receipts(&root.0, &paths).expect("import local receipts");
    assert_eq!(ledger.receipts.len(), 5);
    assert!(
        ledger
            .receipts
            .iter()
            .all(|receipt| receipt.digest_status == DigestStatus::VerifiedLocalFile)
    );
    for (path, original) in paths.iter().zip(before) {
        assert_eq!(
            fs::read(root.0.join(path)).expect("sidecar after import"),
            original
        );
    }
    assert_eq!(
        fs::read(root.0.join("source.txt")).expect("source after import"),
        source
    );
}

#[test]
fn https_uri_is_declared_not_fetched_or_claimed_verified() {
    let root = EvidenceRoot::new();
    let receipt = root.write_receipt(
        "remote.json",
        &declaration(
            ReceiptType::Payment,
            ReceiptSource::Uri {
                uri: "https://receipts.example.invalid/payment/123".into(),
            },
            &digest(b"external bytes supplied by operator"),
        ),
    );
    let ledger = import_receipts(&root.0, &[receipt]).expect("offline URI import");
    assert_eq!(
        ledger.receipts[0].digest_status,
        DigestStatus::DeclaredRemoteUri
    );
}

#[test]
fn rejects_missing_timestamp_digest_source_and_unknown_measurement_fields() {
    let root = EvidenceRoot::new();
    let valid = serde_json::json!({
        "schema_version": 1,
        "receipt_type": "outreach",
        "source": {"kind": "uri", "uri": "https://example.invalid/source"},
        "occurred_at_unix_ms": 1_789_300_000_000_u64,
        "evidence_sha256": digest(b"source"),
    });
    for (index, key) in ["source", "occurred_at_unix_ms", "evidence_sha256"]
        .into_iter()
        .enumerate()
    {
        let mut missing = valid.clone();
        missing.as_object_mut().expect("receipt object").remove(key);
        let path = format!("missing-{index}.json");
        fs::write(
            root.0.join(&path),
            serde_json::to_vec(&missing).expect("JSON"),
        )
        .expect("write invalid sidecar");
        assert!(import_receipts(&root.0, &[path.into()]).is_err());
    }
    let mut injection = valid;
    injection["measurements"] = serde_json::json!([{"kind": "hard_gate", "name": "paid"}]);
    fs::write(
        root.0.join("injection.json"),
        serde_json::to_vec(&injection).expect("JSON"),
    )
    .expect("write injected sidecar");
    assert!(import_receipts(&root.0, &["injection.json".into()]).is_err());
}

#[test]
fn refuses_false_local_digest_and_unsafe_sources() {
    let root = EvidenceRoot::new();
    fs::write(root.0.join("source.txt"), b"actual").expect("write source");
    let bad_digest = root.write_receipt(
        "bad-digest.json",
        &declaration(
            ReceiptType::Refund,
            ReceiptSource::File {
                path: "source.txt".into(),
            },
            &digest(b"different"),
        ),
    );
    assert!(import_receipts(&root.0, &[bad_digest]).is_err());
    for (index, source) in [
        ReceiptSource::File {
            path: "../outside".into(),
        },
        ReceiptSource::File {
            path: PathBuf::from("/tmp/outside"),
        },
        ReceiptSource::Uri {
            uri: "http://example.invalid/receipt".into(),
        },
        ReceiptSource::Uri {
            uri: "https://user:secret@example.invalid/receipt".into(),
        },
        ReceiptSource::Uri {
            uri: "https://example.invalid/receipt?token=secret".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let receipt = root.write_receipt(
            &format!("unsafe-{index}.json"),
            &declaration(ReceiptType::Payment, source, &digest(b"source")),
        );
        assert!(import_receipts(&root.0, &[receipt]).is_err());
    }
}

#[test]
fn imported_receipt_cannot_deserialize_as_gate_objective_or_tie_breaker() {
    let root = EvidenceRoot::new();
    let receipt = root.write_receipt(
        "outreach.json",
        &declaration(
            ReceiptType::Outreach,
            ReceiptSource::Uri {
                uri: "https://example.invalid/outreach".into(),
            },
            &digest(b"outreach"),
        ),
    );
    let imported: MarketReceipt = import_receipts(&root.0, &[receipt])
        .expect("import receipt")
        .receipts
        .remove(0);
    let value = serde_json::to_value(imported).expect("serialize imported receipt");
    assert!(serde_json::from_value::<Measurement>(value.clone()).is_err());
    assert!(serde_json::from_value::<EvaluationSnapshot>(value).is_err());

    let baseline = score_snapshot(10.0);
    let mut candidate = score_snapshot(9.0);
    candidate.measurements.push(
        Measurement::numeric(
            "outreach_receipts",
            NumericMetricKind::MarketEvidence,
            MetricDirection::Maximize,
            1_000_000.0,
        )
        .expect("typed market fixture"),
    );
    let decision = select_candidate(&baseline, &candidate, "score").expect("frozen decision");
    assert_eq!(decision.disposition, Disposition::Discard);
    assert_eq!(decision.reason, DecisionReason::PrimaryRegression);
}

fn score_snapshot(score: f64) -> EvaluationSnapshot {
    EvaluationSnapshot {
        measurements: vec![
            Measurement::numeric(
                "score",
                NumericMetricKind::Objective,
                MetricDirection::Maximize,
                score,
            )
            .expect("score fixture"),
        ],
        complexity: Complexity::default(),
    }
}

#[cfg(unix)]
#[test]
fn symlinked_local_source_is_rejected() {
    use std::os::unix::fs::symlink;

    let root = EvidenceRoot::new();
    fs::write(root.0.join("source.txt"), b"actual").expect("write source");
    symlink(root.0.join("source.txt"), root.0.join("link.txt")).expect("make symlink");
    let receipt = root.write_receipt(
        "symlink.json",
        &declaration(
            ReceiptType::Payment,
            ReceiptSource::File {
                path: "link.txt".into(),
            },
            &digest(b"actual"),
        ),
    );
    assert!(import_receipts(&root.0, &[receipt]).is_err());
}
