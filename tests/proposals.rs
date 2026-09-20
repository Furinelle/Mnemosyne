use anyhow::Result;
use mnemosyne::{
    api,
    proposals::{self, Request, Target},
    provenance::{self, SystemClock, WriteRequestV2},
    store::{self, Store},
};
use serde_json::json;
fn fixture() -> Result<(tempfile::TempDir, Store, Target)> {
    let tmp = tempfile::tempdir()?;
    let s = Store {
        scope: "project".into(),
        root: tmp.path().join(".mnemosyne"),
    };
    provenance::upgrade_store(&s)?;
    let out = provenance::write_v2(
        &s,
        &WriteRequestV2 {
            memory_type: "codebase".into(),
            title: "db".into(),
            content: "PostgreSQL".into(),
            importance: 70,
            origin: "test".into(),
            source_session_id: "s".into(),
            source_event_id: "e".into(),
            finding_key: "f".into(),
            source_kind: "tool_output".into(),
            verification_state: "unverified".into(),
            ..Default::default()
        },
        &SystemClock,
    )?;
    let v = api::show_v2(std::slice::from_ref(&s), &out.memory_ref.memory_id, None)?;
    let t = Target {
        memory_ref: out.memory_ref,
        expected_rev: v["revision"]["semantic_rev"].as_u64().unwrap(),
        expected_hash: v["revision"]["semantic_hash"].as_str().unwrap().into(),
        body: Some("## db\n\nSQLite".into()),
        status: None,
    };
    Ok((tmp, s, t))
}
fn request(t: Target) -> Request {
    Request {
        decision: "REFINE".into(),
        reason: "explicit correction".into(),
        evidence: vec!["test evidence".into()],
        targets: vec![t],
    }
}
#[test]
fn approval_hash_cas_idempotence_and_compensation() -> Result<()> {
    let (_tmp, s, t) = fixture()?;
    let p = proposals::propose(&s, request(t.clone()), &SystemClock)?;
    assert_eq!(p.state, "pending");
    assert_eq!(
        proposals::propose(&s, p.request.clone(), &SystemClock)?.id,
        p.id
    );
    assert!(proposals::review(&s, &p.id, "approve", "wrong", &SystemClock).is_err());
    let a = proposals::review(&s, &p.id, "approve", &p.summary_hash, &SystemClock)?;
    assert_eq!(a.state, "applied");
    assert_eq!(
        proposals::review(&s, &p.id, "approve", &p.summary_hash, &SystemClock)?.applied[0]
            .semantic_rev,
        a.applied[0].semantic_rev
    );
    assert!(
        api::show_v2(std::slice::from_ref(&s), &t.memory_ref.memory_id, None)?["memory"]["body"]
            .as_str()
            .unwrap()
            .contains("SQLite")
    );
    assert_eq!(
        proposals::review(&s, &p.id, "undo", &p.summary_hash, &SystemClock)?.state,
        "undone"
    );
    assert!(
        api::show_v2(std::slice::from_ref(&s), &t.memory_ref.memory_id, None)?["memory"]["body"]
            .as_str()
            .unwrap()
            .contains("PostgreSQL")
    );
    Ok(())
}
#[test]
fn changed_target_stales_all_and_undo_never_overwrites_new_edit() -> Result<()> {
    let (_tmp, s, t) = fixture()?;
    let p = proposals::propose(&s, request(t.clone()), &SystemClock)?;
    let req = serde_json::from_value(
        json!({"memory_ref":t.memory_ref,"expected_rev":t.expected_rev,"expected_hash":t.expected_hash,"changes":{"body":"external correction"}}),
    )?;
    api::revise_v2(&s, &req, &SystemClock)?;
    assert_eq!(
        proposals::review(&s, &p.id, "approve", &p.summary_hash, &SystemClock)?.state,
        "stale"
    );
    let v = api::show_v2(std::slice::from_ref(&s), &t.memory_ref.memory_id, None)?;
    let mut t = t;
    t.expected_rev = v["revision"]["semantic_rev"].as_u64().unwrap();
    t.expected_hash = v["revision"]["semantic_hash"].as_str().unwrap().into();
    let p = proposals::propose(&s, request(t.clone()), &SystemClock)?;
    proposals::review(&s, &p.id, "approve", &p.summary_hash, &SystemClock)?;
    let path = s
        .working_dir()
        .join(format!("{}.md", t.memory_ref.memory_id));
    let raw = std::fs::read_to_string(&path)?;
    std::fs::write(&path, raw.replace("SQLite", "new external fact"))?;
    assert!(proposals::review(&s, &p.id, "undo", &p.summary_hash, &SystemClock).is_err());
    assert!(std::fs::read_to_string(path)?.contains("new external fact"));
    Ok(())
}
#[test]
fn sleep_replay_bounds_and_stale_input_do_not_advance_cursor() -> Result<()> {
    let (_tmp, s, t) = fixture()?;
    let b = mnemosyne::sleep::export(&s, 0, None, 1)?;
    let before = store::load_memories_unlocked(&s, true)?[0].1.body.clone();
    let report = mnemosyne::sleep::finish(&s, &b, vec![request(t.clone())], &SystemClock)?;
    let again = mnemosyne::sleep::finish(&s, &b, vec![request(t.clone())], &SystemClock)?;
    assert_eq!(report, again);
    assert_eq!(before, store::load_memories_unlocked(&s, true)?[0].1.body);
    assert!(mnemosyne::sleep::export(&s, 0, None, 101).is_err());
    let cursor = mnemosyne::sleep::cursor(&s)?;
    let path = s
        .working_dir()
        .join(format!("{}.md", t.memory_ref.memory_id));
    std::fs::write(
        &path,
        std::fs::read_to_string(&path)?.replace("PostgreSQL", "Oracle SQL"),
    )?;
    assert!(mnemosyne::sleep::finish(&s, &b, vec![], &SystemClock).is_err());
    assert_eq!(cursor, mnemosyne::sleep::cursor(&s)?);
    Ok(())
}
#[test]
fn reconciliation_distinguishes_context_conflict_and_multivalue() -> Result<()> {
    let (_tmp, s, _) = fixture()?;
    let make = |event: &str, env: &str, multi: bool, content: &str| {
        serde_json::from_value::<mnemosyne::reconcile::Request>(
            json!({"fact":{"subject":"service","environment":env,"attribute":"database","multivalued":multi},"value":content,"write":{"type":"codebase","title":"db choice","content":content,"importance":70,"origin":"tests","source_session_id":"s","source_event_id":event,"finding_key":"f","source_kind":"tool_output","verification_state":"unverified"}}),
        )
    };
    mnemosyne::reconcile::write(&s, make("a", "prod", false, "PostgreSQL")?, &SystemClock)?;
    let c = mnemosyne::reconcile::classify(&s, &make("b", "dev", false, "SQLite")?)?;
    assert_eq!(c["candidates"][0]["decision"], "CONTEXTUALIZE");
    let c = mnemosyne::reconcile::classify(&s, &make("b", "prod", false, "SQLite")?)?;
    assert_eq!(c["candidates"][0]["decision"], "CONTRADICT");
    let c = mnemosyne::reconcile::classify(&s, &make("b", "prod", true, "SQLite")?)?;
    assert_eq!(c["candidates"][0]["decision"], "CREATE");
    assert!(
        store::load_memories_unlocked(&s, false)?
            .iter()
            .all(|(_, m)| m.status == "active")
    );
    Ok(())
}

fn another_target(store: &Store, event: &str) -> Result<Target> {
    let outcome = provenance::write_v2(
        store,
        &WriteRequestV2 {
            memory_type: "codebase".into(),
            title: event.into(),
            content: event.into(),
            importance: 70,
            origin: "test".into(),
            source_session_id: "s".into(),
            source_event_id: event.into(),
            finding_key: event.into(),
            source_kind: "tool_output".into(),
            verification_state: "unverified".into(),
            ..Default::default()
        },
        &SystemClock,
    )?;
    let view = api::show_v2(
        std::slice::from_ref(store),
        &outcome.memory_ref.memory_id,
        None,
    )?;
    Ok(Target {
        memory_ref: outcome.memory_ref,
        expected_rev: view["revision"]["semantic_rev"].as_u64().unwrap(),
        expected_hash: view["revision"]["semantic_hash"].as_str().unwrap().into(),
        body: None,
        status: None,
    })
}

#[test]
fn relation_proposals_are_mirrored_and_supersede_has_a_replacement() -> Result<()> {
    let (_tmp, store, mut source) = fixture()?;
    let target = another_target(&store, "replacement-target")?;
    source.body = None;
    assert!(
        proposals::propose(
            &store,
            Request {
                decision: "SUPERSEDE".into(),
                reason: "bad shape".into(),
                evidence: vec![],
                targets: vec![source.clone()],
            },
            &SystemClock
        )
        .is_err()
    );
    let proposal = proposals::propose(
        &store,
        Request {
            decision: "SUPERSEDE".into(),
            reason: "verified replacement".into(),
            evidence: vec![],
            targets: vec![source.clone(), target.clone()],
        },
        &SystemClock,
    )?;
    let applied = proposals::review(
        &store,
        &proposal.id,
        "approve",
        &proposal.summary_hash,
        &SystemClock,
    )?;
    assert!(applied.approved_at.is_some());
    assert_eq!(
        applied.approved_summary_hash.as_deref(),
        Some(proposal.summary_hash.as_str())
    );
    let memories = store::load_memories_unlocked(&store, false)?;
    let first = &memories
        .iter()
        .find(|(_, m)| m.id == source.memory_ref.memory_id)
        .unwrap()
        .1;
    let second = &memories
        .iter()
        .find(|(_, m)| m.id == target.memory_ref.memory_id)
        .unwrap()
        .1;
    assert!(
        first
            .links
            .iter()
            .any(|link| link.id == second.id && link.rel == "supersedes")
    );
    assert!(
        second
            .links
            .iter()
            .any(|link| link.id == first.id && link.rel == "superseded_by")
    );
    assert_eq!(second.status, "superseded");
    assert_eq!(second.extra["invalidated_by"], first.id);
    let fresh = |mut target: Target| -> Result<Target> {
        let view = api::show_v2(
            std::slice::from_ref(&store),
            &target.memory_ref.memory_id,
            None,
        )?;
        target.expected_rev = view["revision"]["semantic_rev"].as_u64().unwrap();
        target.expected_hash = view["revision"]["semantic_hash"].as_str().unwrap().into();
        Ok(target)
    };
    let cycle = proposals::propose(
        &store,
        Request {
            decision: "SUPERSEDE".into(),
            reason: "cyclic replacement".into(),
            evidence: vec![],
            targets: vec![fresh(target.clone())?, fresh(source.clone())?],
        },
        &SystemClock,
    )?;
    assert!(
        proposals::review(
            &store,
            &cycle.id,
            "approve",
            &cycle.summary_hash,
            &SystemClock
        )
        .is_err()
    );
    proposals::review(
        &store,
        &proposal.id,
        "undo",
        &proposal.summary_hash,
        &SystemClock,
    )?;
    let memories = store::load_memories_unlocked(&store, false)?;
    assert!(
        memories
            .iter()
            .all(|(_, m)| m.links.is_empty() && m.status == "active")
    );
    Ok(())
}

#[test]
fn relation_checks_all_endpoints_and_undo_rejects_new_dependency() -> Result<()> {
    let (_tmp, store, mut source) = fixture()?;
    let mut target = another_target(&store, "relation-target")?;
    source.body = None;
    let request = Request {
        decision: "CAUSED_BY".into(),
        reason: "verified cause".into(),
        evidence: vec![],
        targets: vec![source.clone(), target.clone()],
    };
    let proposal = proposals::propose(&store, request.clone(), &SystemClock)?;
    let correction = serde_json::from_value(json!({
        "memory_ref": target.memory_ref, "expected_rev": target.expected_rev,
        "expected_hash": target.expected_hash, "changes":{"body":"external correction"}
    }))?;
    api::revise_v2(&store, &correction, &SystemClock)?;
    assert_eq!(
        proposals::review(
            &store,
            &proposal.id,
            "approve",
            &proposal.summary_hash,
            &SystemClock
        )?
        .state,
        "stale"
    );
    assert!(
        store::load_memories_unlocked(&store, false)?
            .iter()
            .all(|(_, m)| m.links.is_empty())
    );

    let view = api::show_v2(
        std::slice::from_ref(&store),
        &target.memory_ref.memory_id,
        None,
    )?;
    target.expected_rev = view["revision"]["semantic_rev"].as_u64().unwrap();
    target.expected_hash = view["revision"]["semantic_hash"].as_str().unwrap().into();
    let proposal = proposals::propose(
        &store,
        Request {
            targets: vec![source.clone(), target.clone()],
            ..request
        },
        &SystemClock,
    )?;
    proposals::review(
        &store,
        &proposal.id,
        "approve",
        &proposal.summary_hash,
        &SystemClock,
    )?;
    let memories = store::load_memories_unlocked(&store, false)?;
    assert!(
        memories
            .iter()
            .any(|(_, m)| m.links.iter().any(|link| link.rel == "caused_by"))
    );
    assert!(
        memories
            .iter()
            .any(|(_, m)| m.links.iter().any(|link| link.rel == "causes"))
    );

    let third = another_target(&store, "later-dependent")?;
    let path = store
        .working_dir()
        .join(format!("{}.md", third.memory_ref.memory_id));
    let mut memory = store::load_memories_unlocked(&store, false)?
        .into_iter()
        .find(|(_, m)| m.id == third.memory_ref.memory_id)
        .unwrap()
        .1;
    memory.links.push(mnemosyne::schema::Link {
        id: source.memory_ref.memory_id,
        rel: "related".into(),
    });
    store::write_memory(&path, &memory)?;
    assert!(
        proposals::review(
            &store,
            &proposal.id,
            "undo",
            &proposal.summary_hash,
            &SystemClock
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn reviewed_refinement_and_contradiction_add_both_edges() -> Result<()> {
    for (decision, forward, reverse) in [
        ("REFINE", "refines", "refined_by"),
        ("CONTRADICT", "contradicts", "contradicts"),
    ] {
        let (_tmp, store, mut first) = fixture()?;
        first.body = None;
        let second = another_target(&store, decision)?;
        let proposal = proposals::propose(
            &store,
            Request {
                decision: decision.into(),
                reason: "reviewed relation".into(),
                evidence: vec![],
                targets: vec![first.clone(), second.clone()],
            },
            &SystemClock,
        )?;
        proposals::review(
            &store,
            &proposal.id,
            "approve",
            &proposal.summary_hash,
            &SystemClock,
        )?;
        let memories = store::load_memories_unlocked(&store, false)?;
        let a = &memories
            .iter()
            .find(|(_, m)| m.id == first.memory_ref.memory_id)
            .unwrap()
            .1;
        let b = &memories
            .iter()
            .find(|(_, m)| m.id == second.memory_ref.memory_id)
            .unwrap()
            .1;
        assert!(
            a.links
                .iter()
                .any(|link| link.id == b.id && link.rel == forward)
        );
        assert!(
            b.links
                .iter()
                .any(|link| link.id == a.id && link.rel == reverse)
        );
    }
    Ok(())
}
