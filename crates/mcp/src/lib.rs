//! ReportCraft MCP server (L6): JSON-RPC 2.0 over stdio, one line per message. Every engine
//! command is an MCP tool; its name is the command id with `.` replaced by `_`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufRead, Write};

use reportcraft_engine::{Session, commands};
use serde_json::{Value as Json, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// Longest request line accepted (16 MiB).
const MAX_LINE: usize = 16 * 1024 * 1024;

/// Serves requests from `input` until it closes.
pub fn serve(input: impl BufRead, mut output: impl Write, session: &mut Session) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_line(&line, session) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// One request line to its reply line (`None` for notifications).
pub fn handle_line(line: &str, session: &mut Session) -> Option<Json> {
    if line.len() > MAX_LINE {
        return Some(error(Json::Null, -32600, "request too large"));
    }
    let req: Json = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(Json::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = req.get("id").cloned()?;
    let method = req.get("method").and_then(Json::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Json::Null);
    Some(match method {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "reportcraft", "version": env!("CARGO_PKG_VERSION")},
            }),
        ),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({"tools": tool_list()})),
        "tools/call" => call(id, &params, session),
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

fn tool_list() -> Vec<Json> {
    commands()
        .iter()
        .map(|c| {
            json!({
                "name": tool_name(c.id),
                "description": format!("{} ({} menu). Parameters: {}", c.label, c.menu, c.params),
                "inputSchema": {"type": "object", "additionalProperties": true},
            })
        })
        .collect()
}

fn tool_name(id: &str) -> String {
    id.replace('.', "_")
}

fn call(id: Json, params: &Json, session: &mut Session) -> Json {
    let name = params.get("name").and_then(Json::as_str).unwrap_or("");
    let Some(spec) = commands().iter().find(|c| tool_name(c.id) == name) else {
        return error(id, -32602, &format!("unknown tool `{name}`"));
    };
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let (text, is_error) = match session.execute(spec.id, &args) {
        Ok(v) => (v.to_string(), false),
        Err(e) => (e.to_string(), true),
    };
    ok(id, json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

fn ok(id: Json, result: Json) -> Json {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Json, code: i64, message: &str) -> Json {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_calls_tools() {
        let mut s = Session::default();
        let list = handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &mut s).unwrap();
        assert!(list.to_string().contains("report_render"));
        let r = handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"file_openSample","arguments":{}}}"#, &mut s).unwrap();
        assert_eq!(r["result"]["isError"], json!(false));
    }

    #[test]
    fn garbage_and_notifications() {
        let mut s = Session::default();
        assert_eq!(handle_line("{", &mut s).unwrap()["error"]["code"], json!(-32700));
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &mut s).is_none());
    }
}
