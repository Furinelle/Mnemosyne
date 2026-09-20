use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const ORDER: &[&str] = &[
    "id",
    "type",
    "source",
    "strength",
    "created",
    "last_accessed",
    "access_count",
    "tags",
    "links",
    "canonical_summary",
    "injection_summary",
    "status",
    "expires",
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Link {
    pub id: String,
    pub rel: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub source: String,
    pub strength: i64,
    pub created: String,
    pub last_accessed: String,
    pub access_count: i64,
    pub tags: Vec<String>,
    pub links: Vec<Link>,
    pub canonical_summary: String,
    pub injection_summary: String,
    pub status: String,
    pub body: String,
    pub expires: String,
    pub extra: Map<String, Value>,
}

impl Memory {
    pub fn title(&self) -> String {
        for line in self.body.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                return line.trim_start_matches('#').trim().to_owned();
            }
        }
        [&self.injection_summary, &self.canonical_summary, &self.id]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default()
    }
}

pub fn is_expired(value: &str) -> bool {
    let value = value.trim();
    if value.len() != 10
        || !value.bytes().enumerate().all(|(i, ch)| {
            if i == 4 || i == 7 {
                ch == b'-'
            } else {
                ch.is_ascii_digit()
            }
        })
    {
        return false;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok_and(|date| date < Local::now().date_naive())
}

fn scalar(raw: &str) -> Value {
    let raw = raw.trim();
    if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
        let mut out = String::new();
        let mut chars = raw[1..raw.len() - 1].chars();
        while let Some(ch) = chars.next() {
            out.push(if ch == '\\' {
                chars.next().unwrap_or('\\')
            } else {
                ch
            });
        }
        return Value::String(out);
    }
    if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 {
        return Value::String(raw[1..raw.len() - 1].to_owned());
    }
    match raw {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => raw
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(raw.to_owned())),
    }
}

fn split_list(raw: &str) -> Vec<Value> {
    let inner = &raw[1..raw.len() - 1];
    if inner.trim().is_empty() {
        return vec![];
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    for (i, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        if Some(ch) == quote {
            quote = None;
            continue;
        }
        if quote.is_none() && (ch == '"' || ch == '\'') {
            quote = Some(ch);
            continue;
        }
        if ch == ',' && quote.is_none() {
            parts.push(scalar(&inner[start..i]));
            start = i + 1;
        }
    }
    parts.push(scalar(&inner[start..]));
    parts
}

fn parse_value(raw: &str) -> Value {
    let raw = raw.trim();
    if raw.starts_with('{')
        && raw.ends_with('}')
        && let Ok(value @ Value::Object(_)) = serde_json::from_str(raw)
    {
        return value;
    }
    if raw.starts_with('[') && raw.ends_with(']') {
        Value::Array(split_list(raw))
    } else {
        scalar(raw)
    }
}

fn parse_header(header: &str) -> Map<String, Value> {
    let lines: Vec<_> = header.lines().collect();
    let mut data = Map::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        if line.trim().is_empty() || line.starts_with(' ') {
            continue;
        }
        let Some((key, raw)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_owned();
        if !raw.trim().is_empty() {
            data.insert(key, parse_value(raw));
            continue;
        }
        let mut items = Vec::new();
        while i < lines.len() && (lines[i].starts_with(' ') || lines[i].trim().is_empty()) {
            let child = lines[i].trim();
            i += 1;
            if let Some(item) = child.strip_prefix("- ") {
                if let Some((k, v)) = item.split_once(':') {
                    let mut map = Map::new();
                    map.insert(k.trim().to_owned(), parse_value(v));
                    items.push(Value::Object(map));
                } else {
                    items.push(parse_value(item));
                }
            } else if let Some((k, v)) = child.split_once(':')
                && let Some(Value::Object(map)) = items.last_mut()
            {
                map.insert(k.trim().to_owned(), parse_value(v));
            }
        }
        data.insert(key, Value::Array(items));
    }
    data
}

fn string(value: Option<Value>, default: &str) -> String {
    match value {
        None => default.to_owned(),
        Some(Value::String(v)) => v,
        Some(Value::Null) => "None".to_owned(),
        Some(v) => v.to_string(),
    }
}

fn number(value: Option<Value>) -> i64 {
    match value {
        Some(Value::Number(v)) => v.as_i64().unwrap_or(0),
        Some(Value::String(v)) => v.parse().unwrap_or(0),
        Some(Value::Bool(v)) => i64::from(v),
        _ => 0,
    }
}

pub fn parse_memory(text: &str) -> Result<Memory> {
    let text = text.replace("\r\n", "\n");
    let (header, body) = if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(pos) = rest.find("\n---\n") {
            (&rest[..pos], &rest[pos + 5..])
        } else if let Some(header) = rest.strip_suffix("\n---") {
            (header, "")
        } else {
            ("", text.as_str())
        }
    } else {
        ("", text.as_str())
    };
    let mut data = parse_header(header);
    let tags = data
        .remove("tags")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(|v| string(Some(v), ""))
        .collect();
    let links = data
        .remove("links")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            let Value::Object(mut obj) = v else {
                return None;
            };
            let id = string(obj.remove("id"), "").trim().to_owned();
            if id.is_empty() {
                return None;
            }
            Some(Link {
                id,
                rel: string(obj.remove("rel"), "").trim().to_owned(),
            })
        })
        .collect();
    Ok(Memory {
        id: string(data.remove("id"), ""),
        memory_type: string(data.remove("type"), "codebase"),
        source: string(data.remove("source"), "agent"),
        strength: number(data.remove("strength")),
        created: string(data.remove("created"), ""),
        last_accessed: string(data.remove("last_accessed"), ""),
        access_count: number(data.remove("access_count")),
        tags,
        links,
        canonical_summary: string(data.remove("canonical_summary"), ""),
        injection_summary: string(data.remove("injection_summary"), ""),
        status: string(data.remove("status"), "active"),
        body: body.trim().to_owned(),
        expires: string(data.remove("expires"), ""),
        extra: data,
    })
}

fn format_scalar(value: &Value) -> String {
    let text = match value {
        Value::Null => return "null".to_owned(),
        Value::Bool(_) | Value::Number(_) => return value.to_string(),
        Value::String(v) => v.replace('\n', " "),
        Value::Object(_) | Value::Array(_) => return value.to_string(),
    };
    if text.is_empty()
        || text != text.trim()
        || matches!(text.as_str(), "true" | "false" | "null")
        || text.parse::<i64>().is_ok()
        || text.chars().any(|c| ":#[]{},\\\"'".contains(c))
    {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        text
    }
}

fn append_value(lines: &mut Vec<String>, key: &str, value: &Value) {
    match value {
        Value::Array(items) if items.is_empty() => lines.push(format!("{key}: []")),
        Value::Array(items) if items.iter().all(|v| !v.is_object()) => lines.push(format!(
            "{key}: [{}]",
            items
                .iter()
                .map(format_scalar)
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Value::Array(items) => {
            lines.push(format!("{key}:"));
            for item in items {
                if let Value::Object(map) = item {
                    for (i, (k, v)) in map.iter().enumerate() {
                        lines.push(format!(
                            "{}{}: {}",
                            if i == 0 { "  - " } else { "    " },
                            k,
                            format_scalar(v)
                        ));
                    }
                } else {
                    lines.push(format!("  - {}", format_scalar(item)));
                }
            }
        }
        _ => lines.push(format!("{key}: {}", format_scalar(value))),
    }
}

pub fn serialize_memory(memory: &Memory) -> String {
    let mut data = memory.extra.clone();
    for (key, value) in [
        ("id", Value::from(memory.id.clone())),
        ("type", Value::from(memory.memory_type.clone())),
        ("source", Value::from(memory.source.clone())),
        ("strength", Value::from(memory.strength)),
        ("created", Value::from(memory.created.clone())),
        ("last_accessed", Value::from(memory.last_accessed.clone())),
        ("access_count", Value::from(memory.access_count)),
        ("tags", serde_json::to_value(&memory.tags).unwrap()),
        ("links", serde_json::to_value(&memory.links).unwrap()),
        (
            "canonical_summary",
            Value::from(memory.canonical_summary.clone()),
        ),
        (
            "injection_summary",
            Value::from(memory.injection_summary.clone()),
        ),
        ("status", Value::from(memory.status.clone())),
    ] {
        data.insert(key.to_owned(), value);
    }
    if !memory.expires.is_empty() {
        data.insert("expires".to_owned(), Value::from(memory.expires.clone()));
    }
    let mut lines = vec!["---".to_owned()];
    for key in ORDER {
        if let Some(value) = data.remove(*key) {
            append_value(&mut lines, key, &value);
        }
    }
    for (key, value) in data {
        append_value(&mut lines, &key, &value);
    }
    lines.push("---".to_owned());
    format!("{}\n{}\n", lines.join("\n"), memory.body.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_commas_and_unknown_fields_round_trip() {
        let input = "---\nid: x\ntags: [\"one, two\", three]\nlinks:\n  - id: other\n    rel: supports\ncustom: \"a,b\"\n---\n# Hello\n";
        let memory = parse_memory(input).unwrap();
        assert_eq!(memory.tags, ["one, two", "three"]);
        assert_eq!(memory.links[0].rel, "supports");
        assert_eq!(memory.extra["custom"], "a,b");
        let again = parse_memory(&serialize_memory(&memory)).unwrap();
        assert_eq!(again.tags, memory.tags);
        assert_eq!(again.extra, memory.extra);
        assert_eq!(again.title(), "Hello");
    }
}
