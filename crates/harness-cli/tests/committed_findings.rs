//! docs/PROJECT-MAP-DESIGN.md §5 (a): the walk's additions (dot-folders
//! pruned, links not descended) leave zopfli's committed observer findings
//! byte-identical — the facts a fresh scan gives and the detectors' findings
//! over them, as `harness detect` writes them, read in place without
//! writing anything. (harness-scan's
//! `the_shared_walk_keeps_the_committed_facts_byte_identical` covers every
//! committed `facts.jsonl`.)

use harness_core::config::TargetContext;
use harness_core::hash;
use harness_core::observer::FindingsFile;
use harness_core::traits::{Detector, LanguageFrontend};
use std::path::Path;

#[test]
fn zopfli_findings_stay_byte_identical() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../targets/zopfli");
    harness_core::adopt::testing::adopt(&root);
    let ctx = TargetContext::load(&root).expect("target loads");
    let facts = harness_scan::CFrontend.scan(&ctx).expect("scan");
    let suite = harness_detect::CTreeSitterSuite;
    let pairs: Vec<(String, String)> = facts
        .files
        .iter()
        .map(|f| (f.path.clone(), f.hash.clone()))
        .collect();
    let file = FindingsFile {
        detector_suite: suite.name().to_string(),
        facts_hash: hash::file_set_hash(&pairs),
        findings: suite.detect(&ctx, &facts).expect("detect"),
    };
    let committed =
        std::fs::read(root.join("migration/observer/findings.jsonl")).expect("committed findings");
    assert!(
        file.to_canonical_bytes().expect("bytes") == committed,
        "zopfli's findings differ from the committed ones"
    );
}
