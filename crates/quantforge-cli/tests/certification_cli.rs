use quantforge_broker::SymbolSpecification;
use quantforge_core::{ContentHash, FloatPolicy};
use quantforge_ir::StrategyIr;
use quantforge_quality::{
    BoundGateEvidence, CERTIFICATION_SCHEMA_VERSION, CHALLENGE_PROTOCOL, CertificationEvidence,
    DataGateEvidence, DataSplitPlan, EVIDENCE_PROTOCOL_VERSION, EXTERNAL_PARITY_PROTOCOL,
    EvidenceBinding, ExternalEngine, ExternalParityEvidence, ILLUMINATION_PROTOCOL,
    INDICATOR_PARITY_PROTOCOL, JUDGE_PROTOCOL, SEALED_FINAL_PROTOCOL, SealedFinalEvidence,
    VALIDATION_PROTOCOL,
};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct SplitArtifact {
    plan: DataSplitPlan,
}

fn gate(
    binding: &EvidenceBinding,
    artifact_hash: ContentHash,
    protocol: &str,
) -> BoundGateEvidence {
    BoundGateEvidence {
        binding: binding.clone(),
        artifact_hash,
        protocol_version: protocol.into(),
        passed: true,
        override_flags: Vec::new(),
    }
}

fn write_gate_artifact(directory: &Path, label: &str) -> (PathBuf, ContentHash) {
    let path = directory.join(format!("{label}.json"));
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "gate": label,
        "passed": true,
        "fixture": true
    }))
    .unwrap();
    fs::write(&path, &bytes).unwrap();
    (path, ContentHash::sha256(bytes))
}

#[test]
fn certified_cli_admits_once_and_vault_refuses_the_same_evidence_twice() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixtures = workspace.join("fixtures");
    let directory = tempfile::tempdir().unwrap();
    let split_path = directory.path().join("split.json");
    let split = Command::new(env!("CARGO_BIN_EXE_quantforge"))
        .args([
            "split-plan",
            fixtures.join("EURUSD_M15_sample.tsv").to_str().unwrap(),
            "--metadata",
            fixtures
                .join("EURUSD_M15_sample.metadata.csv")
                .to_str()
                .unwrap(),
            "--out",
            split_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        split.status.success(),
        "{}",
        String::from_utf8_lossy(&split.stderr)
    );
    let split: SplitArtifact = serde_json::from_slice(&fs::read(&split_path).unwrap()).unwrap();

    let strategy_path = fixtures.join("EURUSD_fixture_strategy.json");
    let broker_path = fixtures.join("EURUSD_fixture_broker.json");
    let strategy: StrategyIr = serde_json::from_slice(&fs::read(&strategy_path).unwrap()).unwrap();
    let broker: SymbolSpecification =
        serde_json::from_slice(&fs::read(&broker_path).unwrap()).unwrap();
    let binding = EvidenceBinding {
        strategy_fingerprint: strategy
            .structural_fingerprint(FloatPolicy::default())
            .unwrap(),
        broker_spec_hash: broker.content_hash().unwrap(),
    };

    let artifacts: Vec<_> = [
        "validation",
        "illumination",
        "challenge",
        "judge",
        "external",
        "indicator",
        "sealed",
    ]
    .into_iter()
    .map(|label| write_gate_artifact(directory.path(), label))
    .collect();
    let split_plan_hash = split.plan.content_hash().unwrap();
    let evidence = CertificationEvidence {
        schema_version: CERTIFICATION_SCHEMA_VERSION,
        protocol_version: EVIDENCE_PROTOCOL_VERSION.into(),
        candidate: binding.clone(),
        split_plan_hash: split_plan_hash.clone(),
        validation: DataGateEvidence {
            gate: gate(&binding, artifacts[0].1.clone(), VALIDATION_PROTOCOL),
            data_hash: split.plan.validation.data_hash.clone(),
        },
        illumination: gate(&binding, artifacts[1].1.clone(), ILLUMINATION_PROTOCOL),
        challenge: gate(&binding, artifacts[2].1.clone(), CHALLENGE_PROTOCOL),
        judge: gate(&binding, artifacts[3].1.clone(), JUDGE_PROTOCOL),
        external_parity: ExternalParityEvidence {
            gate: gate(&binding, artifacts[4].1.clone(), EXTERNAL_PARITY_PROTOCOL),
            engine: ExternalEngine::Mt5StrategyTester,
            protective_orders_present: true,
        },
        indicator_parity: gate(&binding, artifacts[5].1.clone(), INDICATOR_PARITY_PROTOCOL),
        sealed_final: SealedFinalEvidence {
            gate: gate(&binding, artifacts[6].1.clone(), SEALED_FINAL_PROTOCOL),
            split_plan_hash,
            sealed_data_hash: split.plan.sealed_final.data_hash.clone(),
            shortlisted_before_open: true,
            used_in_selection_score: false,
        },
        incubation: None,
        evaluations_touched: 100,
        research_override_flags: Vec::new(),
    };
    let evidence_path = directory.path().join("evidence.json");
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    let vault = directory.path().join("vault");

    let run_certify = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_quantforge"));
        command.args([
            "certify",
            "--strategy",
            strategy_path.to_str().unwrap(),
            "--broker",
            broker_path.to_str().unwrap(),
            "--split-plan",
            split_path.to_str().unwrap(),
            "--evidence",
            evidence_path.to_str().unwrap(),
            "--vault",
            vault.to_str().unwrap(),
        ]);
        for (path, _) in &artifacts {
            command.arg("--artifact").arg(path);
        }
        command.output().unwrap()
    };

    let first = run_certify();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let candidate_vault = vault.join(binding.strategy_fingerprint.as_str());
    assert_eq!(fs::read_dir(&candidate_vault).unwrap().count(), 1);

    let duplicate = run_certify();
    assert!(!duplicate.status.success());
    assert_eq!(fs::read_dir(candidate_vault).unwrap().count(), 1);
}
