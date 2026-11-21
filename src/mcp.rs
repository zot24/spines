use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::domain::{ItemOut, NowPlaying};
use crate::error::{Error, Result};
use crate::store::Store;
use crate::PluginId;

pub enum Reply {
    Response(String),
    Notification,
    Bad(String),
}

pub fn reply(store: &Store, now: Option<&NowPlaying>, raw: &str) -> Reply {
    let value: Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => return Reply::Bad("expected a JSON-RPC object".into()),
    };
    let Some(message) = value.as_object() else {
        return Reply::Bad("expected a JSON-RPC object".into());
    };
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Reply::Bad("jsonrpc must be 2.0".into());
    }
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    if id.is_none() {
        return Reply::Notification;
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "spines", "version": "0.1.0" }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(store, now, message.get("params").unwrap_or(&Value::Null)),
        _ => {
            return Reply::Response(rpc_error(id, -32601, "method not found"));
        }
    };
    Reply::Response(rpc_ok(id, result))
}

pub fn stdio(store: &Store) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(store, stdin.lock(), stdout.lock())
}

pub fn serve<R: BufRead, W: Write>(store: &Store, mut reader: R, mut writer: W) -> Result<()> {
    loop {
        let Some(raw) = read_message(&mut reader)? else {
            return Ok(());
        };
        if let Reply::Response(json) = reply(store, None, &raw) {
            write_frame(&mut writer, &json)?;
        }
    }
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "search",
            "Find albums and books. Every word must appear in the title, people, or description.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "plugin": { "type": "string", "enum": ["music", "books"] }
                },
                "required": ["query"]
            }),
        ),
        tool(
            "browse",
            "List one shelf.",
            json!({
                "type": "object",
                "properties": { "plugin": { "type": "string", "enum": ["music", "books"] } },
                "required": ["plugin"]
            }),
        ),
        tool(
            "get_item",
            "Fetch one catalog item by id.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        ),
        tool(
            "now_playing",
            "What the web UI in this process is playing. A separate spines mcp process has no playback state.",
            json!({ "type": "object", "properties": {} }),
        ),
    ]
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema
    })
}

fn call(store: &Store, now: Option<&NowPlaying>, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let outcome = match name {
        "search" => search(store, &args),
        "browse" => browse(store, &args),
        "get_item" => get_item(store, &args),
        "now_playing" => Ok(serde_json::to_value(now).unwrap_or(Value::Null)),
        _ => Err(format!("unknown tool {name}")),
    };
    match outcome {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": pretty(&value) }],
            "isError": false
        }),
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }),
    }
}

fn search(store: &Store, args: &Value) -> std::result::Result<Value, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if query.trim().is_empty() {
        return Err("query is required".into());
    }
    let plugin = plugin_arg(args.get("plugin"))?;
    let words: Vec<String> = query.split_whitespace().map(str::to_string).collect();
    let items = store.items(plugin, &words).map_err(|e| e.to_string())?;
    Ok(rows(&items))
}

fn browse(store: &Store, args: &Value) -> std::result::Result<Value, String> {
    let Some(plugin) = plugin_arg(args.get("plugin"))? else {
        return Err("plugin is required".into());
    };
    let items = store.items(Some(plugin), &[]).map_err(|e| e.to_string())?;
    Ok(rows(&items))
}

fn get_item(store: &Store, args: &Value) -> std::result::Result<Value, String> {
    let id = args.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return Err("id is required".into());
    }
    match store.item(id).map_err(|e| e.to_string())? {
        Some(item) => serde_json::to_value(ItemOut::from(&item)).map_err(|e| e.to_string()),
        None => Err(format!("no item {id}")),
    }
}

fn plugin_arg(value: Option<&Value>) -> std::result::Result<Option<PluginId>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(name) = value.as_str() else {
        return Err("plugin must be music or books".into());
    };
    if name.is_empty() {
        return Ok(None);
    }
    PluginId::parse(name)
        .map(Some)
        .ok_or_else(|| "plugin must be music or books".into())
}

fn rows(items: &[crate::domain::Item]) -> Value {
    let shown: Vec<Value> = items
        .iter()
        .take(80)
        .map(|item| {
            json!({
                "id": item.id,
                "plugin": item.plugin,
                "title": item.title,
                "subtitle": item.subtitle,
                "year": item.year
            })
        })
        .collect();
    json!({
        "count": items.len(),
        "truncated": items.len() > shown.len(),
        "items": shown
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn rpc_ok(id: Option<Value>, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn rpc_error(id: Option<Value>, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<String>> {
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            let len: usize = rest
                .trim()
                .parse()
                .map_err(|_| Error::msg("mcp content length is not a number"))?;
            loop {
                line.clear();
                let n = reader.read_line(&mut line)?;
                if n == 0 {
                    return Err(Error::msg("mcp frame ended early"));
                }
                if line == "\n" || line == "\r\n" {
                    break;
                }
            }
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        return Ok(Some(trimmed.to_string()));
    }
}

fn write_frame<W: Write>(writer: &mut W, json: &str) -> Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", json.len())?;
    writer.write_all(json.as_bytes())?;
    writer.flush()?;
    Ok(())
}
