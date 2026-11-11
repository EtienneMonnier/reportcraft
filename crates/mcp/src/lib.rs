//! ReportCraft MCP server (L6): JSON-RPC 2.0 over stdio, one line per message. Every engine
//! command is an MCP tool; its name is the command id with `.` replaced by `_`.
//!
//! An MCP client is an agent, so [`serve`] narrows the session to [`Policy::AGENT`]: databases
//! only through the user's saved connections, and actions outside the application (printing,
//! saving a connection) proposed but never confirmed. [`serve_with`] takes another policy (the
//! CLI's `mcp --allow-paths --allow-actions`). Each tool carries MCP annotations saying whether
//! it only reads, edits the session, or acts outside it.
//!
//! Requests run one at a time. A reader thread keeps reading while one runs, so a
//! `notifications/cancelled` for it stops it (the reply is then an error); one for a request not
//! yet started means that request is skipped without a reply.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, mpsc};

use reportcraft_engine::{Access, Policy, Session, commands};
use serde_json::{Value as Json, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// Longest request line accepted (16 MiB).
const MAX_LINE: usize = 16 * 1024 * 1024;

/// Requests cancelled before they started, remembered at most.
const MAX_CANCELLED: usize = 1_000;

/// What the reader thread shares with the request loop.
#[derive(Default)]
struct InFlight {
    /// The id of the request running now.
    current: Option<Json>,
    /// Ids cancelled before they started.
    cancelled: VecDeque<Json>,
}

/// Serves requests from `input` until it closes, as an agent ([`Policy::AGENT`]).
pub fn serve(input: impl BufRead + Send + 'static, output: impl Write, session: &mut Session) -> std::io::Result<()> {
    serve_with(input, output, session, Policy::AGENT)
}

/// [`serve`] with the given policy for the session (restored when the input closes).
pub fn serve_with(input: impl BufRead + Send + 'static, output: impl Write, session: &mut Session, policy: Policy) -> std::io::Result<()> {
    let before = session.policy;
    session.policy = policy;
    let result = serve_loop(input, output, session);
    session.policy = before;
    result
}

fn serve_loop(input: impl BufRead + Send + 'static, mut output: impl Write, session: &mut Session) -> std::io::Result<()> {
    let state = Arc::new(Mutex::new(InFlight::default()));
    let (lines, incoming) = mpsc::sync_channel::<std::io::Result<String>>(64);
    let reader_state = Arc::clone(&state);
    let handle = session.cancel_handle();
    std::thread::spawn(move || {
        for line in input.lines() {
            if let Ok(text) = &line
                && let Some(request_id) = cancelled_request(text)
            {
                if let Ok(mut st) = reader_state.lock() {
                    if st.current.as_ref() == Some(&request_id) {
                        handle.cancel();
                    } else {
                        if st.cancelled.len() == MAX_CANCELLED {
                            st.cancelled.pop_front();
                        }
                        st.cancelled.push_back(request_id);
                    }
                }
                continue;
            }
            let failed = line.is_err();
            if lines.send(line).is_err() || failed {
                break;
            }
        }
    });
    for line in incoming {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req = match parse(&line) {
            Ok(req) => req,
            Err(reply) => {
                writeln!(output, "{reply}")?;
                output.flush()?;
                continue;
            }
        };
        if let Some(id) = req.get("id")
            && let Ok(mut st) = state.lock()
        {
            if let Some(i) = st.cancelled.iter().position(|c| c == id) {
                st.cancelled.remove(i);
                continue;
            }
            st.current = Some(id.clone());
        }
        let reply = handle_request(&req, session);
        if let Ok(mut st) = state.lock() {
            // Under the lock, so a late cancel can't reach the next request.
            st.current = None;
            session.cancel_handle().reset();
        }
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The request id a `notifications/cancelled` line names, if it is one.
fn cancelled_request(line: &str) -> Option<Json> {
    if line.len() > MAX_LINE || !line.contains("notifications/cancelled") {
        return None;
    }
    let msg: Json = serde_json::from_str(line).ok()?;
    if msg.get("method").and_then(Json::as_str) != Some("notifications/cancelled") {
        return None;
    }
    msg.get("params")?.get("requestId").cloned()
}

fn parse(line: &str) -> Result<Json, Json> {
    if line.len() > MAX_LINE {
        return Err(error(Json::Null, -32600, "request too large"));
    }
    serde_json::from_str(line).map_err(|e| error(Json::Null, -32700, &format!("parse error: {e}")))
}

/// One request line to its reply line (`None` for notifications).
pub fn handle_line(line: &str, session: &mut Session) -> Option<Json> {
    match parse(line) {
        Ok(req) => handle_request(&req, session),
        Err(reply) => Some(reply),
    }
}

fn handle_request(req: &Json, session: &mut Session) -> Option<Json> {
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
                "annotations": annotations(c.access, c.label),
            })
        })
        .collect()
}

/// MCP tool annotations: reads are read-only; external actions reach outside the application.
fn annotations(access: Access, title: &str) -> Json {
    match access {
        Access::Read => json!({"title": title, "readOnlyHint": true, "openWorldHint": false}),
        Access::Edit => json!({"title": title, "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}),
        Access::External => json!({"title": title, "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true}),
    }
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
        let tools = list["result"]["tools"].as_array().unwrap();
        let tool = |name: &str| tools.iter().find(|t| t["name"] == json!(name)).unwrap()["annotations"].clone();
        assert_eq!(tool("report_validate")["readOnlyHint"], json!(true));
        assert_eq!(tool("data_loadCsv")["readOnlyHint"], json!(false));
        assert_eq!(tool("file_print")["openWorldHint"], json!(true));
        let r = handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"file_openSample","arguments":{}}}"#, &mut s).unwrap();
        assert_eq!(r["result"]["isError"], json!(false));
    }

    #[test]
    fn a_cancel_notification_stops_the_running_request() {
        let dir = std::env::temp_dir().join(format!("reportcraft-mcp-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("x.db");
        std::fs::write(&db, b"").unwrap();
        let (reader, mut writer) = std::io::pipe().unwrap();
        let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";
        let call = move |id: u32| {
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
                   "params": {"name": "data_loadSqlite", "arguments": {"path": db.to_string_lossy(), "query": endless}}})
        };
        let cancel = |id: u32| json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": id}});
        let feeder = std::thread::spawn(move || {
            // Request 1 is cancelled while it runs; request 2 before it starts.
            writeln!(writer, "{}", call(1)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            writeln!(writer, "{}\n{}\n{}", cancel(2), cancel(1), call(2)).unwrap();
            writeln!(writer, r#"{{"jsonrpc":"2.0","id":3,"method":"ping"}}"#).unwrap();
        });
        let mut out = Vec::new();
        #[allow(clippy::disallowed_methods)] // native test
        let started = std::time::Instant::now();
        // A person's policy: this test is about cancellation, and it opens a file by its path.
        serve_with(std::io::BufReader::new(reader), &mut out, &mut Session::default(), Policy::PERSON).unwrap();
        feeder.join().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let replies: Vec<Json> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0]["id"], json!(1));
        assert_eq!(replies[0]["result"]["isError"], json!(true));
        assert!(replies[0]["result"]["content"][0]["text"].as_str().unwrap().contains("stopped on request"));
        assert_eq!(replies[1]["id"], json!(3), "request 2 was skipped");
    }

    #[test]
    fn an_agent_reaches_saved_connections_only_and_the_policy_is_restored() {
        let line = |id: u32, name: &str, args: Json| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}});
        let input = format!(
            "{}\n{}\n",
            line(1, "data_loadSqlite", json!({"path": "/etc/passwd", "query": "SELECT 1"})),
            line(2, "file_print", json!({"confirm": true}))
        );
        let mut out = Vec::new();
        let mut s = Session::default();
        serve(std::io::Cursor::new(input.into_bytes()), &mut out, &mut s).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("saved connections only") && text.contains("only a person"), "{text}");
        assert_eq!(s.policy, Policy::PERSON, "the session's own policy comes back");
    }

    #[test]
    fn garbage_and_notifications() {
        let mut s = Session::default();
        assert_eq!(handle_line("{", &mut s).unwrap()["error"]["code"], json!(-32700));
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &mut s).is_none());
    }
}
