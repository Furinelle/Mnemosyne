use crate::{api, schema::serialize_memory, store::*};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

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
fn tool(name: &str, a: &Value) -> Result<Value> {
    let project = a["project_path"].as_str();
    match name {
        "mnemosyne_search" => Ok(json!(api::search_entries(
            &allowed_stores(string(a, "scope", "all"), project)?,
            string(a, "query", ""),
            a["limit"].as_u64().unwrap_or(5).min(10000) as usize,
            string(a, "type", ""),
            a["include_archive"].as_bool().unwrap_or(false),
            a["include_superseded"].as_bool().unwrap_or(false),
            false
        )?)),
        "mnemosyne_write" => {
            let stores = allowed_stores(string(a, "scope", "project"), project)?;
            ensure!(stores.len() == 1, "Write needs one store");
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
        "mnemosyne_read_core" => {
            let mut result = json!({});
            for s in allowed_stores(string(a, "scope", "all"), project)? {
                result[&s.scope] = json!(read_core(&s)?);
            }
            Ok(result)
        }
        "mnemosyne_show" => {
            let (_, _, m) =
                find_memory(string(a, "id", ""), &allowed_stores("all", project)?, true)?
                    .ok_or_else(|| anyhow::anyhow!("Memory not found"))?;
            Ok(json!(serialize_memory(&m)))
        }
        "mnemosyne_link" => crate::relations::link_entries(
            &allowed_stores("all", project)?,
            string(a, "id1", ""),
            string(a, "id2", ""),
            string(a, "rel", "related"),
            false,
        ),
        "mnemosyne_graph" => Ok(json!(crate::relations::graph(
            &allowed_stores("all", project)?,
            string(a, "id", ""),
            a["depth"].as_u64().unwrap_or(1).min(100) as usize,
            string(a, "format", "mermaid")
        )?)),
        "mnemosyne_maintain" => {
            let mut result = api::maintain(
                &allowed_stores(string(a, "scope", "all"), project)?,
                a["dry_run"].as_bool().unwrap_or(true),
            )?;
            result.as_object_mut().unwrap().remove("core_candidates");
            Ok(result)
        }
        "mnemosyne_prep_context" | "mnemosyne_codex_prep" => Ok(json!(crate::context::prep(
            &allowed_stores("all", project)?,
            string(a, "task", ""),
            a["limit"].as_u64().unwrap_or(5).min(10000) as usize,
            "mcp"
        )?)),
        _ => bail!("Tool not found: {name}"),
    }
}
fn error(id: &Value, code: i64, message: impl ToString) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message.to_string()}})
}
pub fn process_line(line: &str) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(&Value::Null, -32700, e)),
    };
    let id = &request["id"];
    if !request.is_object() {
        return Some(error(id, -32600, "Invalid Request"));
    }
    let result = match request["method"].as_str().unwrap_or("") {
        "notifications/initialized" => return None,
        "initialize" => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"mnemosyne","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => {
            json!({"tools":serde_json::from_str::<Value>(include_str!("mcp_tools.json")).expect("embedded schema")})
        }
        "tools/call" => {
            let p = &request["params"];
            let empty = json!({});
            let a = p.get("arguments").unwrap_or(&empty);
            if !p.is_object() || !a.is_object() {
                return Some(error(id, -32602, "Tool arguments must be an object"));
            }
            let name = p["name"].as_str().unwrap_or("");
            let schemas: Value = serde_json::from_str(include_str!("mcp_tools.json")).unwrap();
            let schema = schemas.as_array().unwrap().iter().find(|s| {
                s["name"] == name
                    || (name == "mnemosyne_codex_prep" && s["name"] == "mnemosyne_prep_context")
            });
            let Some(schema) = schema else {
                return Some(error(id, -32601, "Unknown tool"));
            };
            if let Some(required) = schema["inputSchema"]["required"].as_array() {
                for key in required {
                    if a.get(key.as_str().unwrap()).is_none() {
                        return Some(error(
                            id,
                            -32602,
                            format!("Missing required argument: {key}"),
                        ));
                    }
                }
            }
            for (key, value) in a.as_object().unwrap() {
                let spec = &schema["inputSchema"]["properties"][key];
                if spec.is_null() {
                    return Some(error(id, -32602, format!("Unknown argument: {key}")));
                }
                let valid = match spec["type"].as_str() {
                    Some("string") => value.is_string(),
                    Some("integer") => value.is_i64() || value.is_u64(),
                    Some("boolean") => value.is_boolean(),
                    _ => true,
                };
                if !valid
                    || spec["enum"]
                        .as_array()
                        .is_some_and(|values| !values.contains(value))
                {
                    return Some(error(id, -32602, format!("Invalid argument: {key}")));
                }
            }
            match tool(name, a) {
                Ok(payload) => {
                    let structured = if payload.is_object() {
                        payload.clone()
                    } else if payload.is_array() {
                        json!({"items":payload})
                    } else {
                        json!({"text":payload})
                    };
                    json!({"content":[{"type":"text","text":serde_json::to_string(&payload).unwrap()}],"structuredContent":structured})
                }
                Err(e) => return Some(error(id, -32603, e)),
            }
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
pub fn serve() -> Result<()> {
    let input = io::stdin();
    let mut output = io::stdout().lock();
    for line in input.lock().lines() {
        if let Some(response) = process_line(&line?) {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn serve_sse() -> Result<()> {
    use std::{
        collections::HashMap,
        io::Read,
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };
    use tiny_http::{Method, Response, Server, StatusCode};
    let config = load_config(None)?;
    let host = config["mcp"]["sse"]["host"].as_str().unwrap_or("127.0.0.1");
    let port = config["mcp"]["sse"]["port"].as_u64().unwrap_or(3700);
    ensure!((1..=65535).contains(&port), "Invalid SSE port");
    let server = Server::http((host, port as u16))
        .map_err(|_| anyhow::anyhow!("Unable to bind MCP SSE listener"))?;
    let sessions: Arc<Mutex<HashMap<String, mpsc::SyncSender<String>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    eprintln!("Mnemosyne MCP SSE listening on configured address");
    for mut request in server.incoming_requests() {
        if request.method() == &Method::Get && request.url() == "/sse" {
            let mut map = sessions.lock().unwrap();
            if map.len() >= 64 {
                drop(map);
                request.respond(Response::empty(StatusCode(503)))?;
                continue;
            }
            let id = uuid::Uuid::new_v4().simple().to_string();
            let (tx, rx) = mpsc::sync_channel(32);
            map.insert(id.clone(), tx);
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
            let tx = sessions.lock().unwrap().get(&id).cloned();
            let Some(tx) = tx else {
                request.respond(Response::empty(StatusCode(404)))?;
                continue;
            };
            let mut body = String::new();
            if request
                .as_reader()
                .take(1_048_577)
                .read_to_string(&mut body)
                .is_err()
            {
                request.respond(Response::empty(StatusCode(400)))?;
                continue;
            }
            if body.len() > 1_048_576 {
                request.respond(Response::empty(StatusCode(413)))?;
                continue;
            }
            if let Some(response) = process_line(&body)
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
        assert!(process_line(r#"{"method":"notifications/initialized"}"#).is_none());
        assert_eq!(process_line(r#"{"id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"type":"codebase","importance":"high","content":"x"}}}"#).unwrap()["error"]["code"],-32602);
        assert_eq!(
            process_line(r#"{"id":1,"method":"tools/list"}"#).unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
    }
}
