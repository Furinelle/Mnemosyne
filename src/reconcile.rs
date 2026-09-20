//! Deterministic candidate classification; similarity never authorizes replacement.
use crate::{
    provenance::{Clock, WriteRequestV2},
    store::{Store, load_memories_unlocked, lock_store},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FactScope {
    pub subject: String,
    pub environment: String,
    pub attribute: String,
    pub multivalued: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub fact: FactScope,
    pub value: String,
    pub write: WriteRequestV2,
}
pub fn classify(store: &Store, r: &Request) -> Result<Value> {
    ensure!(
        !r.fact.subject.trim().is_empty()
            && !r.fact.environment.trim().is_empty()
            && !r.fact.attribute.trim().is_empty()
            && !r.value.trim().is_empty(),
        "Explicit fact scope/value required"
    );
    let _lock = lock_store(store)?;
    let records = load_memories_unlocked(store, false)?;
    let key = serde_json::to_string(&(&r.fact, &r.value, r.write.content.trim()))?;
    let mut candidates = vec![];
    for (path, m) in records {
        let Some(raw) = m.extra.get("fact_key").and_then(Value::as_str) else {
            continue;
        };
        let Ok((fact, value, content)) = serde_json::from_str::<(FactScope, String, String)>(raw)
        else {
            continue;
        };
        if fact.subject != r.fact.subject || fact.attribute != r.fact.attribute {
            continue;
        }
        let same_body = content == r.write.content.trim();
        let decision = if fact.environment != r.fact.environment {
            "CONTEXTUALIZE"
        } else if value != r.value {
            if fact.multivalued || r.fact.multivalued {
                "CREATE"
            } else {
                "CONTRADICT"
            }
        } else if same_body {
            if crate::provenance::read_provenance_unlocked(store, &m.id)?
                .source_events
                .iter()
                .any(|e| {
                    e.origin == r.write.origin
                        && e.source_session_id == r.write.source_session_id
                        && e.source_event_id == r.write.source_event_id
                        && e.finding_key == r.write.finding_key
                })
            {
                "SKIP"
            } else {
                "SUPPORT"
            }
        } else {
            "REFINE"
        };
        candidates.push(json!({"decision":decision,"memory_id":m.id,"expected_revision":crate::revisions::snapshot(store,&path)?,"reason":"Explicit subject/environment/attribute/cardinality comparison; conflicting values remain unverified","evidence":m.canonical_summary}));
    }
    Ok(
        json!({"decision":if candidates.is_empty(){"CREATE"}else{"REVIEW_CANDIDATES"},"fact_key":key,"candidates":candidates,"automatic_replacement":false}),
    )
}
pub fn write(store: &Store, mut r: Request, clock: &impl Clock) -> Result<Value> {
    let report = classify(store, &r)?;
    r.write.fact_key = serde_json::to_string(&(&r.fact, &r.value, r.write.content.trim()))?;
    let outcome = crate::provenance::write_v2(store, &r.write, clock)?;
    Ok(json!({"classification":report,"write":outcome}))
}
