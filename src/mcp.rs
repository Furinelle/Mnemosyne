use crate::{api, input, schema::serialize_memory, store::*};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    sync::OnceLock,
};

const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone, Copy)]
enum Tool {
    Search,
    Write,
    ReadCore,
    Show,
    Link,
    Graph,
    Maintain,
    Prep,
}
impl Tool {
    const ALL: [Self; 8] = [
        Self::Search,
        Self::Write,
        Self::ReadCore,
        Self::Show,
        Self::Link,
        Self::Graph,
        Self::Maintain,
        Self::Prep,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Search => "mnemosyne_search",
            Self::Write => "mnemosyne_write",
            Self::ReadCore => "mnemosyne_read_core",
            Self::Show => "mnemosyne_show",
            Self::Link => "mnemosyne_link",
            Self::Graph => "mnemosyne_graph",
            Self::Maintain => "mnemosyne_maintain",
            Self::Prep => "mnemosyne_prep_context",
        }
    }
}
struct RegisteredTool {
    kind: Tool,
    schema: Value,
}
fn registry() -> &'static [RegisteredTool] {
    static TOOLS: OnceLock<Vec<RegisteredTool>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        let schemas: Vec<Value> =
            serde_json::from_str(include_str!("mcp_tools.json")).expect("embedded tool schema");
        assert_eq!(
            schemas.len(),
            Tool::ALL.len(),
            "tool registry and schemas must agree"
        );
        Tool::ALL
            .into_iter()
            .map(|kind| {
                let mut schema = schemas
                    .iter()
                    .find(|s| s["name"] == kind.name())
                    .expect("registered tool schema")
                    .clone();
                for (key, max) in [("limit", 10000), ("depth", 100)] {
                    if let Some(spec) = schema["inputSchema"]["properties"].get_mut(key) {
                        spec["minimum"] = json!(0);
                        spec["maximum"] = json!(max);
                    }
                }
                RegisteredTool { kind, schema }
            })
            .collect()
    })
}

fn allowed_stores(scope: &str, project_path: Option<&str>) -> Result<Vec<Store>> {
    let explicit = project_path
        .filter(|p| !p.trim().is_empty())
        .map(|value| -> Result<Store> {
            let path = std::path::Path::new(value);
            ensure!(path.is_absolute(), "project_path must be absolute");
            let root = path.canonicalize()?;
            ensure!(
                root.is_dir() && (root.join(".git").exists() || root.join(".mnemosyne").is_dir()),
                "project_path must identify a repository or initialized project"
            );
            let memory_root = root.join(".mnemosyne");
            ensure!(
                memory_root.canonicalize().ok() != global_store().root.canonicalize().ok()
                    || !memory_root.exists(),
                "Global memory is not a project store"
            );
            Ok(Store {
                scope: "project".into(),
                root: memory_root,
            })
        })
        .transpose()?;
    let config = load_config(explicit.as_ref())?;
    let requested = if let Some(project) = explicit {
        match scope {
            "project" => vec![project],
            "global" => vec![global_store()],
            "all" => vec![global_store(), project],
            _ => bail!("Unknown scope"),
        }
    } else {
        ensure!(
            scope != "project" || find_project_store().is_some(),
            "No project context; supply project_path or start the server in an initialized project"
        );
        stores_for_scope(scope)?
    };
    let stores: Vec<_> = requested
        .into_iter()
        .filter(|s| {
            config["mcp"][format!("expose_{}", s.scope)]
                .as_bool()
                .unwrap_or(true)
        })
        .collect();
    ensure!(
        scope == "all" || !stores.is_empty(),
        "MCP access to {scope} scope is disabled"
    );
    Ok(stores)
}
fn string<'a>(a: &'a Value, key: &str, default: &'a str) -> &'a str {
    a[key].as_str().unwrap_or(default)
}
fn tool(kind: Tool, a: &Value) -> Result<Value> {
    let project = a["project_path"].as_str();
    match kind {
        Tool::Search if a["as_of"].is_string() => crate::history::search(
            &allowed_stores(string(a, "scope", "all"), project)?,
            string(a, "query", ""),
            string(a, "as_of", ""),
            a["limit"].as_u64().unwrap_or(5).min(10000) as usize,
            string(a, "type", ""),
            a["include_archive"].as_bool().unwrap_or(false),
            a["include_superseded"].as_bool().unwrap_or(false),
        ),
        Tool::Search => Ok(json!(api::search_entries(
            &allowed_stores(string(a, "scope", "all"), project)?,
            string(a, "query", ""),
            a["limit"].as_u64().unwrap_or(5).min(10000) as usize,
            string(a, "type", ""),
            a["include_archive"].as_bool().unwrap_or(false),
            a["include_superseded"].as_bool().unwrap_or(false),
            false
        )?)),
        Tool::Write => {
            let stores = allowed_stores(string(a, "scope", "project"), project)?;
            ensure!(stores.len() == 1, "Write needs one store");
            if a["version"] == 2 {
                let operation = string(a, "operation", "");
                return if operation == "memory" {
                    Ok(serde_json::to_value(crate::provenance::write_v2(
                        &stores[0],
                        &serde_json::from_value(a["request"].clone())?,
                        &crate::provenance::SystemClock,
                    )?)?)
                } else if operation == "proposal" {
                    Ok(serde_json::to_value(crate::proposals::propose(
                        &stores[0],
                        serde_json::from_value(a["request"].clone())?,
                        &crate::provenance::SystemClock,
                    )?)?)
                } else if ["approve", "reject", "undo"].contains(&operation) {
                    anyhow::bail!("HUMAN_APPROVAL_REQUIRED: use reviewed CLI with summary hash")
                } else if operation == "reconcile" {
                    crate::reconcile::write(
                        &stores[0],
                        serde_json::from_value(a["request"].clone())?,
                        &crate::provenance::SystemClock,
                    )
                } else if operation == "sleep_export" || operation == "sleep_rules" {
                    let r = &a["request"];
                    let cursor = r["cursor"].as_u64().unwrap_or(0) as usize;
                    let limit = r["limit"].as_u64().unwrap_or(50).min(101) as usize;
                    if operation == "sleep_export" {
                        Ok(serde_json::to_value(crate::sleep::export(
                            &stores[0],
                            cursor,
                            r["snapshot"].as_str(),
                            limit,
                        )?)?)
                    } else {
                        crate::sleep::rules(
                            &stores[0],
                            cursor,
                            r["snapshot"].as_str(),
                            limit,
                            &crate::provenance::SystemClock,
                        )
                    }
                } else if operation == "sleep_import" {
                    crate::sleep::finish(
                        &stores[0],
                        &serde_json::from_value(a["request"]["batch"].clone())?,
                        serde_json::from_value(a["request"]["proposals"].clone())?,
                        &crate::provenance::SystemClock,
                    )
                } else if operation == "view_generate" {
                    Ok(serde_json::to_value(crate::views::generate(
                        &stores[0],
                        &serde_json::from_value(a["request"].clone())?,
                        &crate::provenance::SystemClock,
                    )?)?)
                } else if operation == "revise" {
                    if a["request"]["changes"].get("body").is_some() {
                        ensure!(
                            a["request"]["changes"]
                                .as_object()
                                .is_some_and(|m| m.len() == 1),
                            "Body rewrite proposals must be separate from metadata corrections"
                        );
                        let r: api::ReviseRequest = serde_json::from_value(a["request"].clone())?;
                        return Ok(serde_json::to_value(crate::proposals::propose(
                            &stores[0],
                            crate::proposals::Request {
                                decision: "REFINE".into(),
                                reason: "MCP body correction requires review".into(),
                                evidence: vec![],
                                targets: vec![crate::proposals::Target {
                                    memory_ref: r.memory_ref,
                                    expected_rev: r.expected_rev,
                                    expected_hash: r.expected_hash,
                                    body: r.changes.body,
                                    status: None,
                                }],
                            },
                            &crate::provenance::SystemClock,
                        )?)?);
                    }

                    api::revise_v2(
                        &stores[0],
                        &serde_json::from_value(a["request"].clone())?,
                        &crate::provenance::SystemClock,
                    )
                } else {
                    crate::checkpoint::dispatch(
                        &stores[0],
                        operation,
                        &a["request"],
                        &crate::provenance::SystemClock,
                    )
                };
            }
            let mut value = a.clone();
            value["tags"] = json!(
                string(a, "tags", "")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            );
            if value.get("source").is_none() {
                value["source"] = json!("mcp");
            }
            let r = api::write_entry(&stores[0], &serde_json::from_value(value)?)?;
            Ok(json!({"status":r.status,"id":r.id}))
        }
        Tool::ReadCore => {
            let mut result = json!({});
            for s in allowed_stores(string(a, "scope", "all"), project)? {
                result[&s.scope] = json!(read_core(&s)?);
            }
            Ok(result)
        }
        Tool::Show => {
            if a["version"] == 2 {
                let stores = allowed_stores("all", project)?;
                if ["proposal", "view"].contains(&string(a, "kind", "memory")) {
                    ensure!(
                        a.get("revision").is_none(),
                        "Revision only applies to memories"
                    );
                    let store = stores
                        .iter()
                        .find(|s| {
                            if let Some(id) = a["store_id"].as_str() {
                                crate::provenance::read_manifest(s)
                                    .ok()
                                    .flatten()
                                    .is_some_and(|m| m.store_id == id)
                            } else {
                                s.scope == "project"
                            }
                        })
                        .ok_or_else(|| anyhow::anyhow!("Store access required"))?;
                    return if string(a, "kind", "") == "proposal" {
                        Ok(serde_json::to_value(crate::proposals::show(
                            store,
                            string(a, "id", ""),
                        )?)?)
                    } else {
                        Ok(serde_json::to_value(crate::views::inspect(
                            store,
                            string(a, "id", ""),
                        )?)?)
                    };
                }
                if string(a, "kind", "memory") == "history" {
                    ensure!(
                        a.get("revision").is_none(),
                        "Use memory kind for a specific revision"
                    );
                    return crate::history::list(
                        &stores,
                        string(a, "id", ""),
                        a["store_id"].as_str(),
                    );
                }
                if let Some(revision) = a["revision"].as_u64() {
                    ensure!(
                        string(a, "kind", "memory") == "memory",
                        "Revision is only valid for memory show"
                    );
                    return crate::history::show(
                        &stores,
                        string(a, "id", ""),
                        a["store_id"].as_str(),
                        revision,
                    );
                }
                if string(a, "kind", "memory") == "checkpoint" {
                    let store = stores
                        .iter()
                        .find(|s| s.scope == "project")
                        .ok_or_else(|| anyhow::anyhow!("Project access required"))?;
                    return crate::checkpoint::load(
                        store,
                        string(a, "id", ""),
                        &crate::provenance::SystemClock,
                    );
                }
                return api::show_v2(&stores, string(a, "id", ""), a["store_id"].as_str());
            }
            let (_, _, m) =
                find_memory(string(a, "id", ""), &allowed_stores("all", project)?, true)?
                    .ok_or_else(|| anyhow::anyhow!("Memory not found"))?;
            Ok(json!(serialize_memory(&m)))
        }
        Tool::Link => {
            let stores = allowed_stores("all", project)?;
            let rel = string(a, "rel", "related");
            if ["refines", "caused_by", "contradicts", "supersedes"].contains(&rel) {
                let mut targets = Vec::new();
                for key in ["id1", "id2"] {
                    let v = api::show_v2(&stores, string(a, key, ""), None)?;
                    targets.push(crate::proposals::Target {
                        memory_ref: serde_json::from_value(v["memory_ref"].clone())?,
                        expected_rev: v["revision"]["semantic_rev"]
                            .as_u64()
                            .ok_or_else(|| anyhow::anyhow!("Missing revision"))?,
                        expected_hash: v["revision"]["semantic_hash"].as_str().unwrap_or("").into(),
                        body: None,
                        status: None,
                    });
                }
                let store = stores
                    .iter()
                    .find(|s| {
                        crate::provenance::read_manifest(s)
                            .ok()
                            .flatten()
                            .is_some_and(|m| m.store_id == targets[0].memory_ref.store_id)
                    })
                    .ok_or_else(|| anyhow::anyhow!("Upgrade required"))?;
                let decision = match rel {
                    "refines" => "REFINE",
                    "caused_by" => "CAUSED_BY",
                    "contradicts" => "CONTRADICT",
                    _ => "SUPERSEDE",
                };
                Ok(serde_json::to_value(crate::proposals::propose(
                    store,
                    crate::proposals::Request {
                        decision: decision.into(),
                        reason: "MCP semantic relation requires review".into(),
                        evidence: vec![],
                        targets,
                    },
                    &crate::provenance::SystemClock,
                )?)?)
            } else {
                crate::relations::link_entries(
                    &stores,
                    string(a, "id1", ""),
                    string(a, "id2", ""),
                    rel,
                    false,
                )
            }
        }
        Tool::Graph => Ok(json!(crate::relations::graph(
            &allowed_stores("all", project)?,
            string(a, "id", ""),
            a["depth"].as_u64().unwrap_or(1).min(100) as usize,
            string(a, "format", "mermaid")
        )?)),
        Tool::Maintain => {
            let mut result = api::maintain(
                &allowed_stores(string(a, "scope", "all"), project)?,
                a["dry_run"].as_bool().unwrap_or(true),
            )?;
            result.as_object_mut().unwrap().remove("core_candidates");
            Ok(result)
        }
        Tool::Prep => {
            let stores = allowed_stores("all", project)?;
            let extra = crate::checkpoint::active_context(
                &stores,
                a["task_id"].as_str(),
                &crate::provenance::SystemClock,
            )?;
            let bundle = crate::context::prep_bundle(
                &stores,
                string(a, "task", ""),
                a["limit"].as_u64().unwrap_or(5).min(10000) as usize,
                "mcp",
                a["budget"].as_u64().map(|n| n as usize),
                &extra,
            )?;
            if a["version"] == 2 {
                Ok(serde_json::to_value(bundle)?)
            } else {
                Ok(json!(bundle.context))
            }
        }
    }
}
fn error(id: &Value, code: i64, message: impl ToString) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.to_string()}})
}
// The legacy direct-call path remains available until a client opts into the
// initialize handshake. Once started, the handshake must finish before tools.
#[derive(Default)]
enum Phase {
    #[default]
    Legacy,
    Initializing,
    Ready,
}
#[derive(Default)]
struct Session {
    phase: Phase,
}

fn params_valid(params: &Value, allowed: &[&str]) -> bool {
    params.as_object().is_some_and(|p| {
        p.keys().all(|k| allowed.contains(&k.as_str()))
            && p.get("_meta").is_none_or(Value::is_object)
    })
}

impl Session {
    fn process(&mut self, line: &str) -> Option<Value> {
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => return Some(error(&Value::Null, -32700, "Parse error")),
        };
        let Some(object) = request.as_object() else {
            return Some(error(&Value::Null, -32600, "Invalid Request"));
        };
        let id = object.get("id");
        let valid_id = id.is_none_or(|v| v.is_string() || v.is_i64() || v.is_u64());
        if request["jsonrpc"] != "2.0"
            || !request["method"].is_string()
            || !valid_id
            || object
                .keys()
                .any(|k| !["jsonrpc", "id", "method", "params"].contains(&k.as_str()))
        {
            return Some(error(
                if valid_id {
                    id.unwrap_or(&Value::Null)
                } else {
                    &Value::Null
                },
                -32600,
                "Invalid Request",
            ));
        }
        let method = request["method"].as_str().unwrap();
        let empty = json!({});
        let params = object.get("params").unwrap_or(&empty);
        // Notifications are never dispatched as requests: a missing id cannot
        // turn tools/call into an unacknowledged mutation. Unknown notifications
        // are ignored. Only initialized changes local protocol state; progress,
        // cancellation and roots/list_changed are harmless no-ops here because
        // this synchronous server has no outgoing or cancellable jobs.
        let Some(id) = id else {
            if method == "notifications/initialized"
                && params_valid(params, &["_meta"])
                && matches!(self.phase, Phase::Initializing)
            {
                self.phase = Phase::Ready;
            }
            return None;
        };
        if !params.is_object() {
            return Some(error(id, -32602, "Parameters must be an object"));
        }
        let payload = match method {
            "initialize" => {
                if !matches!(self.phase, Phase::Legacy) {
                    return Some(error(
                        id,
                        -32600,
                        "Connection is already initializing or initialized",
                    ));
                }
                if !params_valid(
                    params,
                    &["protocolVersion", "capabilities", "clientInfo", "_meta"],
                ) || params["protocolVersion"].as_str().is_none_or(str::is_empty)
                    || !params["capabilities"].is_object()
                    || !params["clientInfo"].is_object()
                    || params["clientInfo"]["name"]
                        .as_str()
                        .is_none_or(str::is_empty)
                    || !params["clientInfo"]["version"].is_string()
                {
                    return Some(error(
                        id,
                        -32602,
                        "initialize requires protocolVersion, capabilities and clientInfo",
                    ));
                }
                self.phase = Phase::Initializing;
                // Our supported set is exactly this version. The client must
                // disconnect if it cannot use the negotiated fallback version.
                json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{}},"serverInfo":{"name":"mnemosyne","version":env!("CARGO_PKG_VERSION")}})
            }
            "ping" => {
                if !params_valid(params, &["_meta"]) {
                    return Some(error(id, -32602, "Invalid ping parameters"));
                }
                json!({})
            }
            "tools/list" | "tools/call" => {
                if matches!(self.phase, Phase::Initializing) {
                    return Some(error(
                        id,
                        -32002,
                        "Send notifications/initialized before using tools",
                    ));
                }
                if method == "tools/list" {
                    if !params_valid(params, &["cursor", "_meta"])
                        || params.get("cursor").is_some_and(|v| v.as_str() != Some(""))
                    {
                        return Some(error(id, -32602, "Invalid or unsupported tools cursor"));
                    }
                    json!({"tools":registry().iter().map(|entry|&entry.schema).collect::<Vec<_>>()})
                } else {
                    if !params_valid(params, &["name", "arguments", "_meta"])
                        || !params["name"].is_string()
                    {
                        return Some(error(
                            id,
                            -32602,
                            "tools/call requires a tool name and object arguments",
                        ));
                    }
                    let arguments = params.get("arguments").unwrap_or(&empty);
                    if !arguments.is_object() {
                        return Some(error(id, -32602, "Tool arguments must be an object"));
                    }
                    let name = params["name"].as_str().unwrap();
                    let name = if name == "mnemosyne_codex_prep" {
                        "mnemosyne_prep_context"
                    } else {
                        name
                    };
                    let Some(entry) = registry().iter().find(|t| t.kind.name() == name) else {
                        return Some(error(id, -32602, "Unknown tool"));
                    };
                    let write_required = if matches!(entry.kind, Tool::Write) {
                        if arguments["version"] == 2 {
                            json!(["version", "operation", "request"])
                        } else {
                            json!(["type", "importance"])
                        }
                    } else {
                        entry.schema["inputSchema"]["required"].clone()
                    };
                    if let Some(required) = write_required.as_array() {
                        for key in required {
                            if arguments.get(key.as_str().unwrap()).is_none() {
                                return Some(error(
                                    id,
                                    -32602,
                                    format!("Missing required argument: {key}"),
                                ));
                            }
                        }
                    }
                    for (key, value) in arguments.as_object().unwrap() {
                        let spec = &entry.schema["inputSchema"]["properties"][key];
                        let typed = match spec["type"].as_str() {
                            Some("string") => value.is_string(),
                            Some("integer") => value.is_i64() || value.is_u64(),
                            Some("boolean") => value.is_boolean(),
                            Some("object") => value.is_object(),
                            _ => false,
                        };
                        let bounded = spec.get("minimum").is_none_or(|min| {
                            value.as_u64().is_some_and(|n| n >= min.as_u64().unwrap())
                        }) && spec.get("maximum").is_none_or(|max| {
                            value.as_u64().is_some_and(|n| n <= max.as_u64().unwrap())
                        });
                        if !typed
                            || !bounded
                            || spec["enum"]
                                .as_array()
                                .is_some_and(|choices| !choices.contains(value))
                        {
                            return Some(error(
                                id,
                                -32602,
                                format!("Invalid or unknown argument: {key}"),
                            ));
                        }
                    }
                    if matches!(entry.kind, Tool::Write) {
                        let v2 = arguments["version"] == 2;
                        let invalid = arguments.as_object().unwrap().keys().any(|key| {
                            if v2 {
                                !["version", "operation", "request", "scope", "project_path"]
                                    .contains(&key.as_str())
                            } else {
                                ["operation", "request"].contains(&key.as_str())
                            }
                        });
                        if invalid {
                            return Some(error(id, -32602, "Mixed write contract versions"));
                        }
                    }
                    if matches!(entry.kind, Tool::Show)
                        && arguments["version"] != 2
                        && (arguments.get("kind").is_some()
                            || arguments.get("store_id").is_some()
                            || arguments.get("revision").is_some())
                    {
                        return Some(error(id, -32602, "Mixed show contract versions"));
                    }
                    match tool(entry.kind, arguments) {
                        Ok(value) => {
                            let structured = if value.is_object() {
                                value.clone()
                            } else if value.is_array() {
                                json!({"items":value})
                            } else {
                                json!({"text":value})
                            };
                            json!({"content":[{"type":"text","text":serde_json::to_string(&value).unwrap()}],"structuredContent":structured})
                        }
                        Err(failure) => {
                            json!({"isError":true,"content":[{"type":"text","text":failure.to_string()}]})
                        }
                    }
                }
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":payload}))
    }
}

/// Stateless compatibility helper. Transport servers use a persistent Session.
pub fn process_line(line: &str) -> Option<Value> {
    Session::default().process(line)
}

pub fn serve() -> Result<()> {
    let limit = input::max_bytes()?;
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut output = io::stdout().lock();
    let mut session = Session::default();
    while let Some(bytes) = input::line(&mut reader, limit)? {
        let response = match std::str::from_utf8(&bytes) {
            Ok(line) => session.process(line),
            Err(_) => Some(error(&Value::Null, -32700, "Invalid UTF-8 message")),
        };
        if let Some(response) = response {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn serve_sse() -> Result<()> {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };
    use tiny_http::{Method, Response, Server, StatusCode};
    let config = load_config(None)?;
    let host = config["mcp"]["sse"]["host"].as_str().unwrap_or("127.0.0.1");
    let port = config["mcp"]["sse"]["port"].as_u64().unwrap_or(3700);
    ensure!((1..=65535).contains(&port), "Invalid SSE port");
    ensure!(
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()),
        "Non-loopback SSE is disabled: use an authenticated local gateway; session_id is not authentication"
    );
    let limit = input::max_bytes()?;
    let configured = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let authorities = [
        configured,
        format!("localhost:{port}"),
        format!("127.0.0.1:{port}"),
        format!("[::1]:{port}"),
    ];
    let server = Server::http((host, port as u16))
        .map_err(|_| anyhow::anyhow!("Unable to bind MCP SSE listener"))?;
    type Client = (mpsc::SyncSender<String>, Arc<Mutex<Session>>);
    let sessions: Arc<Mutex<HashMap<String, Client>>> = Arc::new(Mutex::new(HashMap::new()));
    eprintln!("Mnemosyne MCP SSE listening on configured address");
    for mut request in server.incoming_requests() {
        // Native clients may omit Origin. Browser requests must use this local
        // origin; validate Host too so DNS rebinding cannot name an attacker host.
        let hosts: Vec<_> = request
            .headers()
            .iter()
            .filter(|h| h.field.equiv("Host"))
            .collect();
        let origins: Vec<_> = request
            .headers()
            .iter()
            .filter(|h| h.field.equiv("Origin"))
            .collect();
        if hosts.len() != 1
            || !authorities
                .iter()
                .any(|a| a.eq_ignore_ascii_case(hosts[0].value.as_str()))
            || origins.len() > 1
            || origins.first().is_some_and(|origin| {
                !authorities.iter().any(|a| {
                    origin
                        .value
                        .as_str()
                        .eq_ignore_ascii_case(&format!("http://{a}"))
                })
            })
        {
            request.respond(Response::empty(StatusCode(403)))?;
            continue;
        }
        if request.method() == &Method::Get && request.url() == "/sse" {
            let mut map = sessions.lock().unwrap();
            if map.len() >= 64 {
                drop(map);
                request.respond(Response::empty(StatusCode(503)))?;
                continue;
            }
            let id = uuid::Uuid::new_v4().simple().to_string();
            let (tx, rx) = mpsc::sync_channel(32);
            map.insert(id.clone(), (tx, Arc::new(Mutex::new(Session::default()))));
            drop(map);
            let sessions = Arc::clone(&sessions);
            std::thread::spawn(move || {
                let mut writer = request.into_writer();
                let result = (|| -> io::Result<()> {
                    write!(
                        writer,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\nevent: endpoint\ndata: /messages?session_id={id}\n\n"
                    )?;
                    writer.flush()?;
                    loop {
                        let text = match rx.recv_timeout(Duration::from_secs(15)) {
                            Ok(text) => text,
                            Err(mpsc::RecvTimeoutError::Timeout) => ": keepalive\n\n".into(),
                            Err(_) => break,
                        };
                        writer.write_all(text.as_bytes())?;
                        writer.flush()?;
                    }
                    Ok(())
                })();
                let _ = result;
                sessions.lock().unwrap().remove(&id);
            });
        } else if request.method() == &Method::Post
            && request.url().starts_with("/messages?session_id=")
        {
            let id = request
                .url()
                .trim_start_matches("/messages?session_id=")
                .to_owned();
            let client = sessions.lock().unwrap().get(&id).cloned();
            let Some((tx, state)) = client else {
                request.respond(Response::empty(StatusCode(404)))?;
                continue;
            };
            if request.body_length().is_some_and(|size| size > limit) {
                request.respond(Response::empty(StatusCode(413)))?;
                continue;
            }
            let bytes = match input::read_bytes(request.as_reader(), limit) {
                Ok(bytes) => bytes,
                Err(failure) => {
                    let status = if failure.to_string().contains("INPUT_TOO_LARGE") {
                        413
                    } else {
                        400
                    };
                    request.respond(Response::empty(StatusCode(status)))?;
                    continue;
                }
            };
            let Ok(body) = std::str::from_utf8(&bytes) else {
                request.respond(Response::empty(StatusCode(400)))?;
                continue;
            };
            if let Some(response) = state.lock().unwrap().process(body)
                && tx
                    .try_send(format!("event: message\ndata: {response}\n\n"))
                    .is_err()
            {
                request.respond(Response::empty(StatusCode(503)))?;
                continue;
            }
            request.respond(Response::empty(StatusCode(202)))?;
        } else {
            request.respond(Response::empty(StatusCode(404)))?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_errors_and_notifications() {
        assert_eq!(process_line("{").unwrap()["error"]["code"], -32700);
        assert!(
            process_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none()
        );
        assert_eq!(process_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"type":"codebase","importance":"high","content":"x"}}}"#).unwrap()["error"]["code"],-32602);
        assert_eq!(
            process_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
    }
}
