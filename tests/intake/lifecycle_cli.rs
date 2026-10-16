//! The full work lifecycle on an intake-scaffolded ledger (issue #326).
//!
//! `init --accept` scaffolds the goal and the work ledger under
//! `.awr/intake/`, and docs/dev/TAKEOVER.md documents those as authored
//! source files. After #323 the mutation writeback already treats them that
//! way; the creation path must too, so a freshly initialized project can run
//! init → create → claim/progress → evidence/completion without relocating
//! the ledger, while runtime-owned paths stay rejected.

use awr_core::Id;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Project(PathBuf);
impl Project {
    fn new(name: &str) -> Self {
        let p = Self(std::env::temp_dir().join(format!("awr-intake-{name}-{}", Id::new())));
        fs::create_dir_all(&p.0).unwrap();
        p
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_awr"))
            .arg("--project")
            .arg(&self.0)
            .arg("--json")
            .args(args)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?}\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn revision(&self) -> String {
        self.ok(&["status"])["project_revision"].to_string()
    }
    fn ledger(&self) -> PathBuf {
        self.0.join(".awr/intake/work-ledger.yaml")
    }
    fn ledger_text(&self) -> String {
        fs::read_to_string(self.ledger()).unwrap()
    }
    fn hex_digest(path: &PathBuf) -> String {
        let bytes = fs::read(path).unwrap();
        format!("{:x}", Sha256::digest(&bytes))
    }
}

#[test]
fn intake_scaffolded_ledger_supports_the_full_work_lifecycle() {
    let p = Project::new("lifecycle");
    p.ok(&[
        "init",
        "--accept",
        "--goal",
        "Verify the authored intake lifecycle",
    ]);
    assert!(p.ledger().is_file(), "init must scaffold the intake ledger");

    // Creation is a two-phase preview/accept against the intake ledger.
    let preview = p.ok(&[
        "work",
        "create",
        "--title",
        "Lifecycle demo",
        "--request-key",
        "lifecycle-1",
    ]);
    let fingerprint = preview["preview"]["fingerprint"].as_str().unwrap();
    let accepted = p.ok(&[
        "work",
        "create",
        "--title",
        "Lifecycle demo",
        "--request-key",
        "lifecycle-1",
        "--accept",
        "--expected-preview",
        fingerprint,
        "--expected-revision",
        &p.revision(),
    ]);
    let key = accepted["external_key"].as_str().unwrap().to_string();
    let ledger = p.ledger_text();
    assert!(ledger.contains(&key), "created work must reach the ledger");
    assert!(ledger.contains("\"status\": \"draft\""));

    // The host finishes the drafted intake record by hand; the created task
    // stays draft until the intake work is reviewed into an executable one.
    let drafted = ledger.replace(
        "\"status\": \"draft\"",
        "\"status\": \"ready\"\n  \"acceptance\": [\"Verify the authored intake lifecycle\"]\n  \"next_action\": \"Run the lifecycle\"",
    );
    assert_ne!(drafted, ledger);
    fs::write(p.ledger(), drafted).unwrap();
    p.ok(&["source", "reindex"]);

    let session = p.ok(&[
        "session",
        "start",
        "--work",
        &key,
        "--agent",
        "fixture-agent",
        "--provider",
        "fixture",
        "--model",
        "fixture",
        "--expected-revision",
        &p.revision(),
        "--claim",
    ])["session"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Progress writes through the authored intake ledger (issue #326 step 2).
    p.ok(&[
        "work",
        "progress",
        &key,
        "--session",
        &session,
        "--reason",
        "exercised the authored intake ledger",
        "--next-action",
        "verify completion",
        "--expected-revision",
        &p.revision(),
        "--response-view",
        "summary",
    ]);

    // Evidence plus completion bind to the current ledger bytes
    // (issue #326 step 3).
    let source_sha = Project::hex_digest(&p.ledger());
    let verified_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let report = serde_json::json!({
        "version": 1,
        "work_item": key,
        "source_sha": source_sha,
        "command": "verify the intake lifecycle",
        "scope": ["lifecycle"],
        "verified_at": verified_at,
        "checks": [{
            "name": "intake_lifecycle",
            "passed": true,
            "details": "create, claim, progress, evidence and completion write through on the intake ledger",
            "criteria": ["Verify the authored intake lifecycle"],
        }],
    });
    fs::write(
        p.0.join("report.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    let evidence_input = serde_json::json!({
        "work_item_key": key,
        "external_key": "EV-intake-lifecycle",
        "evidence_type": "verification_report",
        "summary": "end-to-end authored intake lifecycle",
        "locator": "report.json",
        "level": "locally_verified",
        "sha256": Project::hex_digest(&p.0.join("report.json")),
        "source_sha": source_sha,
        "command": "verify the intake lifecycle",
        "scope": ["lifecycle"],
        "verified_at": verified_at,
    });
    let input = p.0.join(".evidence-input.json");
    fs::write(&input, serde_json::to_vec(&evidence_input).unwrap()).unwrap();
    p.ok(&[
        "evidence",
        "add",
        "--input",
        input.to_str().unwrap(),
        "--expected-revision",
        &p.revision(),
    ]);

    let completion_input = serde_json::json!({
        "version": 1,
        "source_sha": source_sha,
        "minimum_level": "locally_verified",
        "acceptance": [{
            "criterion": "Verify the authored intake lifecycle",
            "evidence": ["EV-intake-lifecycle"],
        }],
    });
    let input = p.0.join(".completion-input.json");
    fs::write(&input, serde_json::to_vec(&completion_input).unwrap()).unwrap();
    p.ok(&[
        "work",
        "complete",
        &key,
        "--session",
        &session,
        "--reason",
        "lifecycle verified",
        "--input",
        input.to_str().unwrap(),
        "--expected-revision",
        &p.revision(),
        "--response-view",
        "summary",
    ]);
    assert!(p.ledger_text().contains("\"status\": \"completed\""));
}

#[test]
fn runtime_owned_ledgers_still_reject_creation() {
    // A ledger registered under .awr/ but outside .awr/intake stays
    // runtime-owned: the authored-intake exception must not become a prefix
    // escape hatch.
    let base = std::env::temp_dir().join(format!("awr-intake-runtime-{}", Id::new()));
    fs::create_dir_all(base.join("project/.awr")).unwrap();
    let root = base.canonicalize().unwrap().join("project");
    fs::write(
        root.join(".awr/work.yaml"),
        "work_items:\n- id: W\n  title: Runtime ledger\n  status: ready\n  acceptance: [Preserve runtime ownership]\n",
    )
    .unwrap();
    fs::write(
        root.join("sources.toml"),
        "[project]\nname='Runtime ledger fixture'\nauthorized_roots=[]\n[[sources]]\ndomain='ledger'\nrole='primary'\npath='.awr/work.yaml'\nadapter='yaml-ledger-v1'\n",
    )
    .unwrap();
    let p = Project(root.clone());
    p.ok(&["init", "--manifest", "sources.toml", "--accept"]);
    let out = p.run(&[
        "work",
        "create",
        "--title",
        "Must not land in runtime state",
        "--request-key",
        "runtime-1",
    ]);
    assert!(!out.status.success());
    let error: Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(error["code"], "RuleViolation");
    assert!(error["message"].as_str().unwrap().contains("runtime-owned"));
    assert!(
        !fs::read_to_string(root.join(".awr/work.yaml"))
            .unwrap()
            .contains("Must not land in runtime state")
    );
}
