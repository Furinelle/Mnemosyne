use anyhow::Result;
use mnemosyne::{
    api,
    checkpoint::{self, CheckpointInput},
    proposals::{Request, Target},
    provenance::{self, SystemClock, WriteRequestV2},
    sleep,
    store::Store,
};
use serde_json::json;
use std::fs;

fn store() -> Result<(tempfile::TempDir, Store)> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    provenance::upgrade_store(&store)?;
    Ok((temp, store))
}

fn memory(store: &Store, key: &str, content: &str) -> Result<Target> {
    let outcome = provenance::write_v2(
        store,
        &WriteRequestV2 {
            memory_type: "codebase".into(),
            title: key.into(),
            content: content.into(),
            importance: 70,
            origin: "test".into(),
            source_session_id: "session".into(),
            source_event_id: key.into(),
            finding_key: key.into(),
            source_kind: "tool_output".into(),
            verification_state: "verified".into(),
            fact_key: key.into(),
            ..Default::default()
        },
        &SystemClock,
    )?;
    let shown = api::show_v2(
        std::slice::from_ref(store),
        &outcome.memory_ref.memory_id,
        None,
    )?;
    Ok(Target {
        memory_ref: outcome.memory_ref,
        expected_rev: shown["revision"]["semantic_rev"].as_u64().unwrap(),
        expected_hash: shown["revision"]["semantic_hash"].as_str().unwrap().into(),
        body: Some(format!("## {key}\n\nrevised")),
        status: None,
    })
}

fn request(target: Target) -> Request {
    Request {
        decision: "REFINE".into(),
        reason: "explicit host suggestion".into(),
        evidence: vec!["test evidence".into()],
        targets: vec![target],
    }
}

#[test]
fn rules_are_offline_metadata_only_and_redact_source_bodies() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "secret", "api_key=not-for-report")?;
    let core = fs::read_to_string(store.core_path())?;
    let report = sleep::rules(&store, 0, None, 10, &SystemClock)?;
    let report_path = fs::read_dir(store.root.join("sleep"))?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name != "cursor.json"))
        .unwrap();
    let raw = fs::read_to_string(report_path)?;
    assert_eq!(report["facts_modified"], false);
    assert!(report["proposal_ids"].as_array().unwrap().is_empty());
    assert!(!raw.contains("api_key") && !raw.contains("not-for-report"));
    assert_eq!(fs::read_to_string(store.core_path())?, core);
    assert_eq!(
        api::show_v2(
            std::slice::from_ref(&store),
            &target.memory_ref.memory_id,
            None
        )?["memory"]["body"],
        "## secret\n\napi_key=not-for-report"
    );
    Ok(())
}

#[test]
fn replay_is_idempotent_and_changed_source_is_stale() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "routing", "port 8080")?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let first = sleep::finish(&store, &batch, vec![request(target.clone())], &SystemClock)?;
    let again = sleep::finish(&store, &batch, vec![request(target.clone())], &SystemClock)?;
    assert_eq!(first, again);
    assert_eq!(first["proposal_ids"].as_array().unwrap().len(), 1);
    api::revise_v2(
        &store,
        &serde_json::from_value(json!({
            "memory_ref": target.memory_ref,
            "expected_rev": target.expected_rev,
            "expected_hash": target.expected_hash,
            "changes": {"body": "## routing\n\nport 9090"},
        }))?,
        &SystemClock,
    )?;
    assert!(sleep::finish(&store, &batch, vec![], &SystemClock).is_err());
    Ok(())
}

#[test]
fn failed_or_partial_proposal_import_does_not_advance_cursor() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "database", "postgres")?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let invalid = Request {
        decision: "REFINE".into(),
        reason: "invalid second request".into(),
        evidence: vec![],
        targets: vec![Target {
            body: None,
            ..target.clone()
        }],
    };
    assert!(sleep::finish(&store, &batch, vec![request(target), invalid], &SystemClock).is_err());
    assert_eq!(sleep::cursor(&store)?, json!({"next_cursor": 0}));
    Ok(())
}

#[test]
fn relation_import_remains_pending_and_never_edits_facts() -> Result<()> {
    let (_temp, store) = store()?;
    let left = memory(&store, "cause", "a")?;
    let right = memory(&store, "effect", "b")?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let relation = Request {
        decision: "CAUSED_BY".into(),
        reason: "host reports causal relation".into(),
        evidence: vec!["reported only".into()],
        targets: vec![
            Target {
                body: None,
                ..left.clone()
            },
            Target {
                body: None,
                ..right.clone()
            },
        ],
    };
    let report = sleep::finish(&store, &batch, vec![relation], &SystemClock)?;
    let id = report["proposal_ids"][0].as_str().unwrap();
    assert_eq!(mnemosyne::proposals::show(&store, id)?.state, "pending");
    for target in [left, right] {
        assert!(
            api::show_v2(
                std::slice::from_ref(&store),
                &target.memory_ref.memory_id,
                None
            )?["memory"]["links"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    Ok(())
}

#[test]
fn pages_are_bounded_and_include_checkpoint_metadata_without_checkpoint_content() -> Result<()> {
    let (temp, store) = store()?;
    memory(&store, "one", "one")?;
    memory(&store, "two", "two")?;
    fs::write(temp.path().join("task.txt"), "work")?;
    checkpoint::create(
        &store,
        CheckpointInput {
            task_id: "task".into(),
            goal: "checkpoint-secret-goal".into(),
            scoped_paths: vec!["task.txt".into()],
            expires: "2099-01-01T00:00:00Z".into(),
            ..Default::default()
        },
        &SystemClock,
    )?;
    let first = sleep::export(&store, 0, None, 1)?;
    assert!(first.partial && first.next_cursor == 1);
    let full = sleep::export(&store, 0, None, 10)?;
    assert!(full.inputs.iter().any(|input| input.kind == "checkpoint"));
    assert!(sleep::export(&store, 1, Some("wrong"), 1).is_err());
    assert!(sleep::export(&store, 0, None, 101).is_err());
    let report = sleep::finish(&store, &full, vec![], &SystemClock)?;
    assert!(!serde_json::to_string(&report)?.contains("checkpoint-secret-goal"));
    Ok(())
}

#[test]
fn oversized_input_is_reported_without_a_cursor_write() -> Result<()> {
    let (_temp, store) = store()?;
    let path = store.working_dir().join("oversized.md");
    fs::write(&path, "x".repeat(128 * 1024 + 1))?;
    let batch = sleep::export(&store, 0, None, 1)?;
    assert!(batch.inputs.is_empty());
    assert_eq!(batch.blocked[0]["reason"], "record_too_large");
    assert_eq!(sleep::cursor(&store)?, json!({"next_cursor": 0}));
    Ok(())
}

#[test]
fn large_store_pages_by_byte_budget_and_rules_can_finish_each_page() -> Result<()> {
    let (_temp, store) = store()?;
    let template = memory(&store, "large-template", &"x".repeat(100_000))?;
    let original_id = template.memory_ref.memory_id;
    let original = fs::read_to_string(store.working_dir().join(format!("{original_id}.md")))?;
    for _ in 1..45 {
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(
            store.working_dir().join(format!("{id}.md")),
            original.replace(&original_id, &id),
        )?;
    }
    let first = sleep::export(&store, 0, None, 1)?;
    assert_eq!(first.inputs.len(), 1);
    assert!(first.partial);

    let mut cursor = 0;
    let mut snapshot = None;
    let mut seen = std::collections::HashSet::new();
    loop {
        let report = sleep::rules(&store, cursor, snapshot.as_deref(), 100, &SystemClock)?;
        let batch: sleep::Batch = serde_json::from_value(report["batch"].clone())?;
        assert!(!batch.inputs.is_empty());
        assert!(batch.next_cursor > cursor);
        let mut page_bytes = 0;
        if cursor == 0 {
            assert!(batch.partial);
        }
        for input in &batch.inputs {
            page_bytes += fs::metadata(
                store
                    .working_dir()
                    .join(format!("{}.md", input.memory_ref.memory_id)),
            )?
            .len();
            assert!(seen.insert(input.memory_ref.memory_id.clone()));
        }
        assert!(page_bytes <= 4 * 1024 * 1024);
        assert_eq!(sleep::cursor(&store)?["next_cursor"], batch.next_cursor);
        cursor = batch.next_cursor;
        snapshot = Some(batch.snapshot);
        if !batch.partial {
            break;
        }
    }
    assert_eq!(seen.len(), 45);
    assert_eq!(cursor, 45);
    Ok(())
}

#[test]
fn changed_unread_record_invalidates_cursor() -> Result<()> {
    let (_temp, store) = store()?;
    memory(&store, "first", "unchanged")?;
    let later = memory(&store, "later", "before")?;
    let first = sleep::export(&store, 0, None, 1)?;
    api::revise_v2(
        &store,
        &serde_json::from_value(json!({
            "memory_ref": later.memory_ref,
            "expected_rev": later.expected_rev,
            "expected_hash": later.expected_hash,
            "changes": {"body": "## later\n\nafter"},
        }))?,
        &SystemClock,
    )?;
    let error = sleep::export(&store, first.next_cursor, Some(&first.snapshot), 1).unwrap_err();
    assert!(error.to_string().contains("STALE_CURSOR"));
    assert_eq!(sleep::cursor(&store)?, json!({"next_cursor": 0}));
    Ok(())
}

#[cfg(unix)]
#[test]
fn replaced_unread_record_with_same_size_and_mtime_is_stale() -> Result<()> {
    let (_temp, store) = store()?;
    memory(&store, "first", "first")?;
    let later = memory(&store, "later", "before")?;
    let first = sleep::export(&store, 0, None, 1)?;
    let path = store
        .working_dir()
        .join(format!("{}.md", later.memory_ref.memory_id));
    let raw = fs::read_to_string(&path)?;
    let modified = fs::metadata(&path)?.modified()?;
    let replacement = path.with_extension("tmp");
    fs::write(&replacement, raw.replace("before", "afterx"))?;
    fs::File::open(&replacement)?.set_times(fs::FileTimes::new().set_modified(modified))?;
    assert_eq!(
        fs::metadata(&replacement)?.len(),
        fs::metadata(&path)?.len()
    );
    fs::rename(replacement, path)?;
    let error = sleep::export(&store, first.next_cursor, Some(&first.snapshot), 1).unwrap_err();
    assert!(error.to_string().contains("STALE_CURSOR"));
    Ok(())
}

#[test]
fn file_inventory_remains_capped_before_reading_bodies() -> Result<()> {
    let (_temp, store) = store()?;
    for index in 0..=10_000 {
        fs::write(store.working_dir().join(format!("{index:05}.md")), [])?;
    }
    let error = sleep::export(&store, 0, None, 1).unwrap_err();
    assert!(
        error.to_string().contains("Sleep scan limit exceeded"),
        "{error:#}"
    );
    assert_eq!(sleep::cursor(&store)?, json!({"next_cursor": 0}));
    Ok(())
}

#[test]
fn upgraded_store_observes_only_exported_memory_and_keeps_cursor_valid() -> Result<()> {
    let (_temp, store) = store()?;
    let original = memory(&store, "known", "known")?;
    let old_id = original.memory_ref.memory_id;
    let new_id = uuid::Uuid::new_v4().to_string();
    let raw = fs::read_to_string(store.working_dir().join(format!("{old_id}.md")))?;
    fs::write(
        store.working_dir().join(format!("{new_id}.md")),
        raw.replace(&old_id, &new_id),
    )?;
    let first = sleep::export(&store, 0, None, 1)?;
    let second = sleep::export(&store, first.next_cursor, Some(&first.snapshot), 1)?;
    assert_eq!(first.inputs.len() + second.inputs.len(), 2);
    assert!(first.inputs[0].revision > 0 && second.inputs[0].revision > 0);
    assert!(sleep::finish(&store, &second, vec![], &SystemClock).is_err());
    sleep::finish(&store, &first, vec![], &SystemClock)?;
    sleep::finish(&store, &second, vec![], &SystemClock)?;
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 2);
    Ok(())
}

#[test]
fn importing_a_proposal_does_not_observe_or_stale_later_pages() -> Result<()> {
    let (_temp, store) = store()?;
    let first_target = memory(&store, "first", "first")?;
    let first_id = &first_target.memory_ref.memory_id;
    let later_id = "zzzz-later";
    let raw = fs::read_to_string(store.working_dir().join(format!("{first_id}.md")))?;
    fs::write(
        store.working_dir().join(format!("{later_id}.md")),
        raw.replace(first_id, later_id),
    )?;
    let first = sleep::export(&store, 0, None, 1)?;
    assert_eq!(first.inputs[0].memory_ref.memory_id, *first_id);
    sleep::finish(&store, &first, vec![request(first_target)], &SystemClock)?;
    let saved = sleep::cursor(&store)?;
    let second = sleep::export(
        &store,
        saved["next_cursor"].as_u64().unwrap() as usize,
        saved["snapshot"].as_str(),
        1,
    )?;
    assert_eq!(second.inputs[0].memory_ref.memory_id, later_id);
    assert_eq!(second.inputs[0].revision, 1);
    Ok(())
}

#[test]
fn paged_outputs_exceed_old_total_budgets_without_early_input_commit() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "seed", "seed")?;
    let id = &target.memory_ref.memory_id;
    let raw = fs::read_to_string(store.working_dir().join(format!("{id}.md")))?;
    for _ in 1..100 {
        let next = uuid::Uuid::new_v4().to_string();
        fs::write(
            store.working_dir().join(format!("{next}.md")),
            raw.replace(id, &next),
        )?;
    }
    let batch = sleep::export(&store, 0, None, 100)?;
    let mut report_bytes = 0;
    let mut ids = std::collections::HashSet::new();
    for index in 0..3 {
        let requests: Vec<_> = (0..10)
            .map(|item| {
                let mut request = request(target.clone());
                request.reason = format!("page {index} item {item}");
                request
            })
            .collect();
        let page = sleep::OutputPage { index, total: 3 };
        let receipt =
            sleep::finish_page(&store, &batch, requests.clone(), page.clone(), &SystemClock)?;
        assert_eq!(
            sleep::finish_page(&store, &batch, requests, page, &SystemClock)?,
            receipt
        );
        report_bytes += serde_json::to_vec(&receipt)?.len();
        assert!(serde_json::to_vec(&receipt)?.len() <= 64 * 1024);
        assert_eq!(receipt["input_committed"], index == 2);
        assert_eq!(
            sleep::cursor(&store)?["next_cursor"],
            if index == 2 { 100 } else { 0 }
        );
        for id in receipt["proposal_ids"].as_array().unwrap() {
            assert!(ids.insert(id.as_str().unwrap().to_owned()));
        }
    }
    assert!(report_bytes > 64 * 1024, "{report_bytes}");
    assert_eq!(ids.len(), 30);
    for id in ids {
        assert_eq!(mnemosyne::proposals::show(&store, &id)?.state, "pending");
    }
    Ok(())
}

#[test]
fn output_order_content_limits_and_empty_terminal_are_explicit() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "item", "body")?;
    let batch = sleep::export(&store, 0, None, 1)?;
    let page = sleep::OutputPage { index: 0, total: 2 };
    assert!(
        sleep::finish_page(
            &store,
            &batch,
            vec![],
            sleep::OutputPage { index: 1, total: 2 },
            &SystemClock
        )
        .is_err()
    );
    assert!(
        sleep::finish_page(
            &store,
            &batch,
            vec![],
            sleep::OutputPage {
                index: 0,
                total: 1001
            },
            &SystemClock
        )
        .is_err()
    );
    assert!(
        sleep::finish(
            &store,
            &batch,
            vec![request(target.clone()); 21],
            &SystemClock
        )
        .is_err()
    );
    let mut large = request(target.clone());
    large.targets[0].body = Some("x".repeat(64 * 1024));
    assert!(sleep::finish(&store, &batch, vec![large], &SystemClock).is_err());
    sleep::finish_page(&store, &batch, vec![], page.clone(), &SystemClock)?;
    assert!(sleep::finish_page(&store, &batch, vec![request(target)], page, &SystemClock).is_err());
    assert!(
        sleep::finish_page(
            &store,
            &batch,
            vec![],
            sleep::OutputPage { index: 1, total: 3 },
            &SystemClock
        )
        .is_err()
    );
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 0);
    sleep::finish_page(
        &store,
        &batch,
        vec![],
        sleep::OutputPage { index: 1, total: 2 },
        &SystemClock,
    )?;
    let terminal = sleep::export(&store, 1, Some(&batch.snapshot), 1)?;
    assert!(terminal.inputs.is_empty() && !terminal.partial);
    assert_eq!(
        sleep::finish(&store, &terminal, vec![], &SystemClock)?["status"],
        "complete"
    );
    Ok(())
}

#[test]
fn invalid_records_do_not_hide_valid_work_or_allow_a_complete_report() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "valid", "safe")?;
    fs::write(
        store.working_dir().join("000-oversize.md"),
        "x".repeat(128 * 1024 + 1),
    )?;
    fs::write(store.working_dir().join("001-invalid.md"), [0xff])?;
    fs::write(
        store.working_dir().join("bad.id.md"),
        "---\nid: bad.id\n---\n\nbody",
    )?;
    let core = fs::read(store.core_path())?;
    let mut cursor = 0;
    let mut snapshot = None;
    let mut blocked = 0;
    let mut seen = Vec::new();
    loop {
        let batch = sleep::export(&store, cursor, snapshot.as_deref(), 1)?;
        blocked += batch.blocked.len();
        seen.extend(
            batch
                .inputs
                .iter()
                .map(|input| input.memory_ref.memory_id.clone()),
        );
        let result = sleep::finish(&store, &batch, vec![], &SystemClock)?;
        cursor = batch.next_cursor;
        snapshot = Some(batch.snapshot);
        if !batch.partial {
            assert_eq!(result["status"], "partial");
            break;
        }
    }
    assert_eq!(blocked, 3);
    assert_eq!(seen, vec![target.memory_ref.memory_id]);
    assert_eq!(sleep::cursor(&store)?["blocked_count"], 3);
    assert_eq!(fs::read(store.core_path())?, core);
    assert_eq!(
        fs::metadata(store.working_dir().join("000-oversize.md"))?.len(),
        128 * 1024 + 1
    );
    Ok(())
}

#[test]
fn concurrent_page_retries_are_identical_and_replay_cannot_rewind() -> Result<()> {
    let (_temp, store) = store()?;
    memory(&store, "one", "one")?;
    memory(&store, "two", "two")?;
    let batch = sleep::export(&store, 0, None, 1)?;
    let results = std::thread::scope(|scope| {
        let left = scope.spawn(|| sleep::finish(&store, &batch, vec![], &SystemClock));
        let right = scope.spawn(|| sleep::finish(&store, &batch, vec![], &SystemClock));
        (left.join().unwrap(), right.join().unwrap())
    });
    assert_eq!(results.0?, results.1?);
    let next = sleep::export(&store, 1, Some(&batch.snapshot), 1)?;
    sleep::finish(&store, &next, vec![], &SystemClock)?;
    sleep::finish(&store, &batch, vec![], &SystemClock)?;
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 2);
    Ok(())
}

#[cfg(unix)]
#[test]
fn input_links_and_output_links_are_rejected() -> Result<()> {
    use std::os::unix::fs::symlink;
    let (temp, store) = store()?;
    let target = memory(&store, "one", "one")?;
    let original = store
        .working_dir()
        .join(format!("{}.md", target.memory_ref.memory_id));
    let linked = store.working_dir().join("linked.md");
    fs::hard_link(&original, &linked)?;
    assert!(sleep::export(&store, 0, None, 1).is_err());
    fs::remove_file(&linked)?;
    symlink(&original, &linked)?;
    assert!(sleep::export(&store, 0, None, 1).is_err());
    fs::remove_file(&linked)?;
    let batch = sleep::export(&store, 0, None, 1)?;
    fs::create_dir(store.root.join("sleep"))?;
    let outside = temp.path().join("outside.json");
    fs::write(&outside, "unchanged")?;
    let cursor = store.root.join("sleep/cursor.json");
    symlink(&outside, &cursor)?;
    assert!(sleep::finish(&store, &batch, vec![], &SystemClock).is_err());
    fs::remove_file(&cursor)?;
    fs::hard_link(&outside, &cursor)?;
    assert!(sleep::finish(&store, &batch, vec![], &SystemClock).is_err());
    assert_eq!(fs::read_to_string(outside)?, "unchanged");
    Ok(())
}

#[test]
fn access_metadata_rewrite_invalidates_the_exported_snapshot() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "one", "one")?;
    let batch = sleep::export(&store, 0, None, 1)?;
    let path = store
        .working_dir()
        .join(format!("{}.md", target.memory_ref.memory_id));
    let mut memory = mnemosyne::schema::parse_memory(&fs::read_to_string(&path)?)?;
    memory.access_count += 1;
    fs::write(&path, mnemosyne::schema::serialize_memory(&memory))?;
    assert!(sleep::finish(&store, &batch, vec![], &SystemClock).is_err());
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 0);
    Ok(())
}

#[test]
fn pending_outputs_block_other_batches_and_keep_blocked_counts() -> Result<()> {
    let (_temp, store) = store()?;
    memory(&store, "safe", "safe")?;
    fs::write(store.working_dir().join("000-bad.md"), [0xff])?;
    let first = sleep::export(&store, 0, None, 1)?;
    assert_eq!(first.blocked.len(), 1);
    sleep::finish_page(
        &store,
        &first,
        vec![],
        sleep::OutputPage { index: 0, total: 2 },
        &SystemClock,
    )?;
    let next = sleep::export(&store, 1, Some(&first.snapshot), 1)?;
    assert!(sleep::finish(&store, &next, vec![], &SystemClock).is_err());
    let overlap = sleep::export(&store, 0, None, 2)?;
    assert!(sleep::finish(&store, &overlap, vec![], &SystemClock).is_err());
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 0);
    sleep::finish_page(
        &store,
        &first,
        vec![],
        sleep::OutputPage { index: 1, total: 2 },
        &SystemClock,
    )?;
    assert_eq!(
        sleep::finish(&store, &next, vec![], &SystemClock)?["status"],
        "partial"
    );
    assert_eq!(sleep::cursor(&store)?["blocked_count"], 1);
    Ok(())
}

#[test]
fn legacy_completed_batch_replays_without_rewriting_old_evidence() -> Result<()> {
    use sha2::{Digest, Sha256};
    let (_temp, store) = store()?;
    memory(&store, "one", "one")?;
    let batch = sleep::export(&store, 0, None, 1)?;
    let key = format!("{:x}", Sha256::digest(serde_json::to_vec(&batch)?));
    let old = json!({"version":1,"batch":batch,"proposal_ids":[],"mode":"offline_or_host","facts_modified":false});
    fs::create_dir(store.root.join("sleep"))?;
    let path = store.root.join("sleep").join(format!("{key}.json"));
    fs::write(&path, serde_json::to_vec(&old)?)?;
    let before = fs::read(&path)?;
    assert_eq!(sleep::finish(&store, &batch, vec![], &SystemClock)?, old);
    assert_eq!(fs::read(path)?, before);
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 1);
    Ok(())
}

#[test]
fn long_valid_ids_page_before_receipt_metadata_exceeds_budget() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "seed", "seed")?;
    let original = store
        .working_dir()
        .join(format!("{}.md", target.memory_ref.memory_id));
    let mut template = mnemosyne::schema::parse_memory(&fs::read_to_string(&original)?)?;
    template.links.push(mnemosyne::schema::Link {
        id: "missing".into(),
        rel: "related".into(),
    });
    for index in 0..100 {
        template.id = format!("{index:03}-{}", "a".repeat(196));
        fs::write(
            store.working_dir().join(format!("{}.md", template.id)),
            mnemosyne::schema::serialize_memory(&template),
        )?;
    }
    let mut cursor = 0;
    let mut snapshot = None;
    loop {
        let batch = sleep::export(&store, cursor, snapshot.as_deref(), 100)?;
        if cursor == 0 {
            assert!(batch.next_cursor < 100);
        }
        let report = sleep::finish(&store, &batch, vec![], &SystemClock)?;
        assert!(serde_json::to_vec(&report)?.len() <= 64 * 1024);
        assert!(batch.next_cursor > cursor);
        cursor = batch.next_cursor;
        snapshot = Some(batch.snapshot);
        if !batch.partial {
            break;
        }
    }
    assert_eq!(cursor, 101);
    Ok(())
}

#[test]
fn committed_replay_survives_source_change_and_cannot_overwrite_new_run() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "late", "before")?;
    let old = sleep::export(&store, 0, None, 1)?;
    let page = sleep::OutputPage { index: 0, total: 2 };
    let requests = vec![request(target.clone())];
    let receipt = sleep::finish_page(&store, &old, requests.clone(), page.clone(), &SystemClock)?;
    api::revise_v2(
        &store,
        &serde_json::from_value(json!({
            "memory_ref": target.memory_ref, "expected_rev": target.expected_rev,
            "expected_hash": target.expected_hash, "changes": {"body": "## late\n\nafter"}
        }))?,
        &SystemClock,
    )?;
    let new = sleep::export(&store, 0, None, 1)?;
    sleep::finish(&store, &new, vec![], &SystemClock)?;
    let progress = sleep::cursor(&store)?;
    assert_eq!(
        receipt,
        sleep::finish_page(&store, &old, requests, page, &SystemClock)?
    );
    assert!(
        sleep::finish_page(
            &store,
            &old,
            vec![],
            sleep::OutputPage { index: 1, total: 2 },
            &SystemClock
        )
        .is_err()
    );
    assert_eq!(sleep::cursor(&store)?, progress);
    assert_eq!(fs::read_dir(store.root.join("proposals"))?.count(), 1);
    Ok(())
}

#[test]
fn replay_preserves_reviewed_proposals_and_sealed_output() -> Result<()> {
    for action in ["approve", "reject"] {
        let (_temp, store) = store()?;
        let target = memory(&store, action, "before")?;
        let batch = sleep::export(&store, 0, None, 1)?;
        let requests = vec![request(target)];
        let receipt = sleep::finish(&store, &batch, requests.clone(), &SystemClock)?;
        let id = receipt["proposal_ids"][0].as_str().unwrap();
        let proposal = mnemosyne::proposals::show(&store, id)?;
        mnemosyne::proposals::review(&store, id, action, &proposal.summary_hash, &SystemClock)?;
        let reviewed = fs::read(store.root.join("proposals").join(format!("{id}.json")))?;
        let progress = sleep::cursor(&store)?;
        assert_eq!(
            receipt,
            sleep::finish(&store, &batch, requests.clone(), &SystemClock)?
        );
        assert_eq!(
            reviewed,
            fs::read(store.root.join("proposals").join(format!("{id}.json")))?
        );
        assert_eq!(progress, sleep::cursor(&store)?);
        assert!(
            sleep::finish_page(
                &store,
                &batch,
                requests,
                sleep::OutputPage { index: 0, total: 2 },
                &SystemClock
            )
            .is_err()
        );
        assert!(
            sleep::finish_page(
                &store,
                &batch,
                vec![],
                sleep::OutputPage { index: 1, total: 2 },
                &SystemClock
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn missing_output_cannot_be_replaced_by_a_duplicate_or_final_page() -> Result<()> {
    let (_temp, store) = store()?;
    memory(&store, "gap", "before")?;
    let batch = sleep::export(&store, 0, None, 1)?;
    let first = sleep::OutputPage { index: 0, total: 3 };
    sleep::finish_page(&store, &batch, vec![], first.clone(), &SystemClock)?;
    sleep::finish_page(&store, &batch, vec![], first, &SystemClock)?;
    let last = sleep::OutputPage { index: 2, total: 3 };
    assert!(sleep::finish_page(&store, &batch, vec![], last.clone(), &SystemClock).is_err());
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 0);
    sleep::finish_page(
        &store,
        &batch,
        vec![],
        sleep::OutputPage { index: 1, total: 3 },
        &SystemClock,
    )?;
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 0);
    sleep::finish_page(&store, &batch, vec![], last, &SystemClock)?;
    assert_eq!(sleep::cursor(&store)?["next_cursor"], 1);
    Ok(())
}
