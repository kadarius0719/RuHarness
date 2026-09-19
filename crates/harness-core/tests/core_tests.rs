//! Schema and planner tests (docs/SCHEMAS.md): golden canonical bytes,
//! round-trip + idempotence, reconcile preservation, topological ordering.

use harness_core::facts::{FileRecord, RefRecord, SymbolRecord};
use harness_core::plan::{self, ComputedUnit, Plan, UnitStatus};
use harness_core::planner;
use harness_core::Facts;
use std::path::PathBuf;

fn sample_facts() -> Facts {
    Facts {
        frontend: "c-tree-sitter".into(),
        files: vec![
            FileRecord {
                path: "src/b.c".into(),
                hash: "blake3:bb".into(),
                includes: vec!["src/b.h".into()],
            },
            FileRecord {
                path: "src/a.c".into(),
                hash: "blake3:aa".into(),
                includes: vec![],
            },
            FileRecord {
                path: "src/b.h".into(),
                hash: "blake3:bh".into(),
                includes: vec![],
            },
        ],
        symbols: vec![
            SymbolRecord {
                name: "beta".into(),
                kind: "function".into(),
                file: "src/b.c".into(),
                visibility: "public".into(),
                signature: "int beta(void)".into(),
                span: (1, 3),
            },
            SymbolRecord {
                name: "alpha".into(),
                kind: "function".into(),
                file: "src/a.c".into(),
                visibility: "public".into(),
                signature: "int alpha(void)".into(),
                span: (1, 5),
            },
            SymbolRecord {
                name: "src/a.c::helper".into(),
                kind: "function".into(),
                file: "src/a.c".into(),
                visibility: "internal".into(),
                signature: "static int helper(void)".into(),
                span: (7, 9),
            },
        ],
        refs: vec![
            RefRecord {
                from: "alpha".into(),
                file: "src/a.c".into(),
                to: "beta".into(),
                refkind: "call".into(),
                resolved: true,
            },
            RefRecord {
                from: "alpha".into(),
                file: "src/a.c".into(),
                to: "src/a.c::helper".into(),
                refkind: "call".into(),
                resolved: true,
            },
            RefRecord {
                from: "beta".into(),
                file: "src/b.c".into(),
                to: "malloc".into(),
                refkind: "call".into(),
                resolved: false,
            },
        ],
    }
}

/// The canonical byte encoding is a public contract: any change to these
/// exact bytes for this input is a schema_version bump (docs/SCHEMAS.md).
#[test]
fn golden_canonical_bytes() {
    let expected = concat!(
        r#"{"k":"header","schema":"ruharness-facts","schema_version":1,"frontend":"c-tree-sitter"}"#,
        "\n",
        r#"{"k":"file","path":"src/a.c","hash":"blake3:aa","includes":[]}"#,
        "\n",
        r#"{"k":"file","path":"src/b.c","hash":"blake3:bb","includes":["src/b.h"]}"#,
        "\n",
        r#"{"k":"file","path":"src/b.h","hash":"blake3:bh","includes":[]}"#,
        "\n",
        r#"{"k":"symbol","name":"alpha","kind":"function","file":"src/a.c","visibility":"public","signature":"int alpha(void)","span":[1,5]}"#,
        "\n",
        r#"{"k":"symbol","name":"src/a.c::helper","kind":"function","file":"src/a.c","visibility":"internal","signature":"static int helper(void)","span":[7,9]}"#,
        "\n",
        r#"{"k":"symbol","name":"beta","kind":"function","file":"src/b.c","visibility":"public","signature":"int beta(void)","span":[1,3]}"#,
        "\n",
        r#"{"k":"ref","from":"alpha","file":"src/a.c","to":"beta","refkind":"call","resolved":true}"#,
        "\n",
        r#"{"k":"ref","from":"alpha","file":"src/a.c","to":"src/a.c::helper","refkind":"call","resolved":true}"#,
        "\n",
        r#"{"k":"ref","from":"beta","file":"src/b.c","to":"malloc","refkind":"call","resolved":false}"#,
        "\n",
    );
    let bytes = sample_facts().to_canonical_bytes().unwrap();
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
}

#[test]
fn facts_round_trip_and_idempotence() {
    let dir = temp_dir("facts-rt");
    let path = dir.join("facts.jsonl");
    let facts = sample_facts();
    facts.store(&path).unwrap();
    let loaded = Facts::load(&path).unwrap();
    // Store-load-store is byte-identical (canonical writer is idempotent).
    assert_eq!(
        facts.to_canonical_bytes().unwrap(),
        loaded.to_canonical_bytes().unwrap()
    );
    // Unknown record kinds and a same-version header survive loading.
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{\"k\":\"future-kind\",\"x\":1}\n");
    std::fs::write(&path, text).unwrap();
    let loaded2 = Facts::load(&path).unwrap();
    assert_eq!(loaded2.symbols.len(), 3);
}

#[test]
fn facts_newer_schema_refused() {
    let dir = temp_dir("facts-ver");
    let path = dir.join("facts.jsonl");
    std::fs::write(
        &path,
        "{\"k\":\"header\",\"schema\":\"ruharness-facts\",\"schema_version\":99}\n",
    )
    .unwrap();
    let err = Facts::load(&path).unwrap_err();
    assert!(err.to_string().contains("schema_version 99"), "{err}");
}

#[test]
fn include_closure_is_transitive() {
    let facts = sample_facts();
    let closure = facts.include_closure(&["src/b.c".to_string()]);
    assert_eq!(closure, vec!["src/b.c".to_string(), "src/b.h".to_string()]);
}

#[test]
fn planner_orders_dependencies_first() {
    let units = planner::compute_units(&sample_facts()).unwrap();
    // a.c calls into b.c, so u-b must come first; helper stays internal.
    let ids: Vec<&str> = units.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, vec!["u-b", "u-a"]);
    assert_eq!(units[1].depends_on, vec!["u-b".to_string()]);
    assert_eq!(units[1].symbols, vec!["alpha".to_string()]);
    // b.c's source hash covers its include closure (b.h).
    assert_ne!(units[0].source_hash, units[1].source_hash);
}

#[test]
fn planner_merges_cycles() {
    let mut facts = sample_facts();
    facts.refs.push(RefRecord {
        from: "beta".into(),
        file: "src/b.c".into(),
        to: "alpha".into(),
        refkind: "call".into(),
        resolved: true,
    });
    let units = planner::compute_units(&facts).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id, "u-a-b");
    assert!(units[0].depends_on.is_empty());
}

#[test]
fn reconcile_preserves_human_state() {
    let dir = temp_dir("plan-rec");
    let path = dir.join("plan.toml");
    let mut computed = vec![ComputedUnit {
        id: "u-a".into(),
        files: vec!["src/a.c".into()],
        symbols: vec!["alpha".into()],
        interface: vec!["int alpha(void)".into()],
        depends_on: vec![],
        source_hash: "blake3:v1".into(),
    }];
    plan::reconcile(&path, "demo", &computed).unwrap();

    // A human edits: status, a comment, and an unknown field.
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replace(
        "status = \"pending\"",
        "status = \"in-progress\" # reviewed by human\nrisk_notes = \"tricky aliasing\"",
    );
    std::fs::write(&path, &text).unwrap();

    // Replan with a changed hash and a new unit.
    computed[0].source_hash = "blake3:v2".into();
    computed.push(ComputedUnit {
        id: "u-b".into(),
        files: vec!["src/b.c".into()],
        symbols: vec!["beta".into()],
        interface: vec![],
        depends_on: vec![],
        source_hash: "blake3:bb".into(),
    });
    let changes = plan::reconcile(&path, "demo", &computed).unwrap();
    assert_eq!(changes.len(), 2, "{changes:?}");

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# reviewed by human"),
        "comment lost:\n{text}"
    );
    assert!(text.contains("risk_notes"), "unknown field lost:\n{text}");
    assert!(text.contains("blake3:v2"), "hash not updated:\n{text}");
    let loaded = Plan::load(&path).unwrap();
    assert_eq!(loaded.units.len(), 2);
    assert_eq!(loaded.units[0].status, UnitStatus::InProgress);

    // A third replan without u-b blocks it but keeps it.
    computed.truncate(1);
    plan::reconcile(&path, "demo", &computed).unwrap();
    let loaded = Plan::load(&path).unwrap();
    assert_eq!(loaded.units[1].status, UnitStatus::Blocked);
}

#[test]
fn adopt_existing_ids_matches_by_file_overlap() {
    let dir = temp_dir("plan-adopt");
    let path = dir.join("plan.toml");
    std::fs::write(
        &path,
        "schema_version = 1\ntarget = \"demo\"\n\n[[unit]]\nid = \"u001-legacy\"\nstatus = \"verified\"\nfiles = [\"src/a.c\"]\n",
    )
    .unwrap();
    let existing = Plan::load(&path).unwrap();
    let mut computed = vec![
        ComputedUnit {
            id: "u-a".into(),
            files: vec!["src/a.c".into()],
            symbols: vec![],
            interface: vec![],
            depends_on: vec![],
            source_hash: "blake3:x".into(),
        },
        ComputedUnit {
            id: "u-b".into(),
            files: vec!["src/b.c".into()],
            symbols: vec![],
            interface: vec![],
            depends_on: vec!["u-a".into()],
            source_hash: "blake3:y".into(),
        },
    ];
    plan::adopt_existing_ids(&mut computed, &existing);
    assert_eq!(computed[0].id, "u001-legacy");
    assert_eq!(computed[1].depends_on, vec!["u001-legacy".to_string()]);
    plan::reconcile(&path, "demo", &computed).unwrap();
    let loaded = Plan::load(&path).unwrap();
    assert_eq!(loaded.units.len(), 2);
    assert_eq!(
        loaded.units[0].status,
        UnitStatus::Verified,
        "status must survive"
    );
}

#[test]
fn execution_order_rejects_missing_and_cycles() {
    let dir = temp_dir("plan-order");
    let path = dir.join("plan.toml");
    std::fs::write(
        &path,
        "schema_version = 1\n[[unit]]\nid = \"a\"\nstatus = \"pending\"\ndepends_on = [\"ghost\"]\n",
    )
    .unwrap();
    let plan_doc = Plan::load(&path).unwrap();
    assert!(plan_doc.execution_order().is_err());

    std::fs::write(
        &path,
        "schema_version = 1\n[[unit]]\nid = \"a\"\nstatus = \"pending\"\ndepends_on = [\"b\"]\n[[unit]]\nid = \"b\"\nstatus = \"pending\"\ndepends_on = [\"a\"]\n",
    )
    .unwrap();
    let plan_doc = Plan::load(&path).unwrap();
    let err = plan_doc.execution_order().unwrap_err();
    assert!(err.to_string().contains("cycle"), "{err}");
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ruharness-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn split_cycle_keeps_both_units() {
    // Regression (M1 review blocker): a merged cycle unit that splits must
    // not collapse both clusters onto one id / corrupt the plan.
    let dir = temp_dir("plan-split");
    let path = dir.join("plan.toml");
    std::fs::write(
        &path,
        "schema_version = 1\ntarget = \"demo\"\n\n[[unit]]\nid = \"u-a-b\"\nstatus = \"in-progress\"\nfiles = [\"src/a.c\", \"src/b.c\"]\n",
    )
    .unwrap();
    let existing = Plan::load(&path).unwrap();
    let mut computed = vec![
        ComputedUnit {
            id: "u-b".into(),
            files: vec!["src/b.c".into()],
            symbols: vec![],
            interface: vec![],
            depends_on: vec![],
            source_hash: "blake3:b".into(),
        },
        ComputedUnit {
            id: "u-a".into(),
            files: vec!["src/a.c".into()],
            symbols: vec![],
            interface: vec![],
            depends_on: vec!["u-b".into()],
            source_hash: "blake3:a".into(),
        },
    ];
    plan::adopt_existing_ids(&mut computed, &existing);
    let ids: Vec<&str> = computed.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["u-a-b", "u-a"], "one claim only");
    assert!(
        !computed.iter().any(|c| c.depends_on.contains(&c.id)),
        "no self-dependency"
    );
    plan::reconcile(&path, "demo", &computed).unwrap();
    let loaded = Plan::load(&path).unwrap();
    assert_eq!(loaded.units.len(), 2);
    assert!(loaded.execution_order().is_ok());
    assert_eq!(
        loaded.units[0].status,
        UnitStatus::InProgress,
        "status kept"
    );
}

#[test]
fn reconcile_rejects_duplicate_computed_ids() {
    let dir = temp_dir("plan-dup");
    let path = dir.join("plan.toml");
    let cu = ComputedUnit {
        id: "u-x".into(),
        files: vec!["src/x.c".into()],
        symbols: vec![],
        interface: vec![],
        depends_on: vec![],
        source_hash: "blake3:x".into(),
    };
    let err = plan::reconcile(&path, "demo", &[cu.clone(), cu]).unwrap_err();
    assert!(err.to_string().contains("duplicate unit id"), "{err}");
}

#[test]
fn stem_collision_units_both_survive() {
    // Regression (M1 review blocker): same-stem files in different dirs must
    // not silently drop a unit.
    let mut facts = sample_facts();
    facts.files.push(FileRecord {
        path: "src2/a.c".into(),
        hash: "blake3:a2".into(),
        includes: vec![],
    });
    facts.symbols.push(SymbolRecord {
        name: "alpha2".into(),
        kind: "function".into(),
        file: "src2/a.c".into(),
        visibility: "public".into(),
        signature: "int alpha2(void)".into(),
        span: (1, 2),
    });
    let units = planner::compute_units(&facts).unwrap();
    assert_eq!(
        units.len(),
        3,
        "{:?}",
        units.iter().map(|u| &u.id).collect::<Vec<_>>()
    );
    let ids: std::collections::BTreeSet<&str> = units.iter().map(|u| u.id.as_str()).collect();
    assert!(ids.contains("u-src-a"), "{ids:?}");
    assert!(ids.contains("u-src2-a"), "{ids:?}");
}

#[test]
fn target_config_cannot_request_unbounded_llm_spend() {
    // Regression (M3 review): harness.toml is hostile input.
    let dir = temp_dir("cfg-clamp");
    let base = "schema_version = 1\n[target]\nname = \"t\"\nsource_dir = \"src\"\n";
    std::fs::write(
        dir.join("harness.toml"),
        format!("{base}[llm.migrate]\nmax_repairs = 4294967295\n"),
    )
    .unwrap();
    let err = harness_core::TargetContext::load(&dir).unwrap_err();
    assert!(err.to_string().contains("max_repairs"), "{err}");
    std::fs::write(
        dir.join("harness.toml"),
        format!("{base}[llm]\nmax_tokens = 1000000\n"),
    )
    .unwrap();
    assert!(harness_core::TargetContext::load(&dir).is_err());
    std::fs::write(
        dir.join("harness.toml"),
        format!("{base}[llm.migrate]\nmax_repairs = 3\n"),
    )
    .unwrap();
    assert!(harness_core::TargetContext::load(&dir).is_ok());
}

#[test]
fn hostile_plan_paths_are_refused_at_load() {
    // Regression (M3 design review): plan fields become path components.
    let dir = temp_dir("plan-paths");
    let path = dir.join("plan.toml");
    for bad in [
        "id = \"../../etc\"\nstatus = \"pending\"\n",
        "id = \"ok\"\nstatus = \"pending\"\nfiles = [\"/etc/passwd\"]\n",
        "id = \"ok\"\nstatus = \"pending\"\n[unit.oracle]\nkind = \"x\"\nrust_crate = \"../up\"\n",
        "id = \"ok\"\nstatus = \"pending\"\n[unit.oracle]\nkind = \"x\"\ndriver = \"a/../../b.c\"\n",
    ] {
        std::fs::write(&path, format!("schema_version = 1\n[[unit]]\n{bad}")).unwrap();
        assert!(Plan::load(&path).is_err(), "accepted hostile plan: {bad}");
    }
}
