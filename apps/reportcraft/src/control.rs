//! The control channel: a localhost JSON-lines server that hands requests to the window.
//!
//! Each connection first authenticates (`{"auth": TOKEN}`, see `reportcraft_engine::control`),
//! then sends requests `{"id", "method", "params"}`, one per line, and gets one reply line each:
//! `{"id", "ok": true, "result"}` or `{"id", "ok": false, "error"}`. The window answers between
//! two frames, so a request sees exactly what the person sees. A line that isn't a JSON object
//! with a string `method` (an HTTP request, binary data) gets one error and the connection closes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value as Json, json};

/// Longest request line accepted.
const MAX_LINE: u64 = 16 * 1024 * 1024;
/// Most connections served at once.
const MAX_CONNECTIONS: usize = 8;
/// How long a request may wait for the window (a render can take a while).
const REPLY_TIMEOUT: Duration = Duration::from_secs(600);

/// A request for the window, and where its answer goes.
pub struct Request {
    pub method: String,
    pub params: Json,
    pub reply: Sender<Result<Json, String>>,
}

/// Listens on `127.0.0.1:port` (0: any free port); returns the port and the requests. `wake` is
/// called after each request is queued, so the window draws a frame and answers it.
pub fn start(port: u16, token: String, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<(u16, Receiver<Request>)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = channel();
    let wake = Arc::new(wake);
    let open = Arc::new(AtomicUsize::new(0));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                open.fetch_sub(1, Ordering::SeqCst);
                let _ = reply_line(&mut &stream, &json!({"ok": false, "error": "too many connections"}));
                continue;
            }
            let (tx, token, wake, open) = (tx.clone(), token.clone(), Arc::clone(&wake), Arc::clone(&open));
            std::thread::spawn(move || {
                serve(stream, &token, &tx, &*wake);
                open.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok((port, rx))
}

fn reply_line(stream: &mut impl Write, value: &Json) -> std::io::Result<()> {
    writeln!(stream, "{value}")?;
    stream.flush()
}

/// Compares without stopping at the first difference, so timing says nothing about the token.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn serve(stream: TcpStream, token: &str, tx: &Sender<Request>, wake: &dyn Fn()) {
    let Ok(mut out) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream.take(u64::MAX));
    let mut line = String::new();
    let mut read_line = |line: &mut String| {
        line.clear();
        reader.get_mut().set_limit(MAX_LINE);
        matches!(reader.read_line(line), Ok(n) if n > 0)
    };
    // Authentication first.
    if !read_line(&mut line) {
        return;
    }
    let auth = serde_json::from_str::<Json>(&line).ok().and_then(|v| v.get("auth").and_then(Json::as_str).map(str::to_owned));
    if !auth.is_some_and(|a| same(&a, token)) {
        let _ = reply_line(&mut out, &json!({"ok": false, "error": "authenticate first: {\"auth\": TOKEN} (reportcraft-cli send reads it)"}));
        return;
    }
    if reply_line(&mut out, &json!({"ok": true})).is_err() {
        return;
    }
    while read_line(&mut line) {
        let request = serde_json::from_str::<Json>(&line).ok().filter(|v| v.get("method").and_then(Json::as_str).is_some());
        let Some(request) = request else {
            let _ = reply_line(&mut out, &json!({"ok": false, "error": "a request is a JSON object with a string `method`"}));
            return;
        };
        let id = request.get("id").cloned().unwrap_or(Json::Null);
        let (reply, answer) = channel();
        let sent = tx.send(Request {
            method: request["method"].as_str().unwrap_or_default().to_owned(),
            params: request.get("params").cloned().unwrap_or_else(|| json!({})),
            reply,
        });
        if sent.is_err() {
            return;
        }
        wake();
        let line = match answer.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(result)) => json!({"id": id, "ok": true, "result": result}),
            Ok(Err(error)) => json!({"id": id, "ok": false, "error": error}),
            Err(_) => json!({"id": id, "ok": false, "error": "the window didn't answer"}),
        };
        if reply_line(&mut out, &line).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(port: u16) -> (TcpStream, BufReader<TcpStream>) {
        let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let r = BufReader::new(s.try_clone().unwrap());
        (s, r)
    }

    fn line(r: &mut BufReader<TcpStream>) -> Json {
        let mut l = String::new();
        r.read_line(&mut l).unwrap();
        serde_json::from_str(&l).unwrap_or(Json::Null)
    }

    #[test]
    fn requests_need_the_token_and_get_the_windows_answer() {
        let (port, rx) = start(0, "s3cret".into(), || {}).unwrap();
        // The "window": answers every request with its method.
        std::thread::spawn(move || {
            for r in rx {
                let answer = if r.method == "boom" { Err("it failed".into()) } else { Ok(json!({"ran": r.method, "params": r.params})) };
                let _ = r.reply.send(answer);
            }
        });
        let (mut s, mut r) = connect(port);
        writeln!(s, r#"{{"auth": "wrong"}}"#).unwrap();
        assert_eq!(line(&mut r)["ok"], json!(false));
        assert_eq!(line(&mut r), Json::Null, "closed after a wrong token");

        let (mut s, mut r) = connect(port);
        writeln!(s, r#"{{"auth": "s3cret"}}"#).unwrap();
        assert_eq!(line(&mut r)["ok"], json!(true));
        writeln!(s, r#"{{"id": 7, "method": "report.get", "params": {{"x": 1}}}}"#).unwrap();
        let answer = line(&mut r);
        assert_eq!(
            (answer["id"].clone(), answer["result"]["ran"].clone(), answer["result"]["params"]["x"].clone()),
            (json!(7), json!("report.get"), json!(1))
        );
        writeln!(s, r#"{{"id": 8, "method": "boom"}}"#).unwrap();
        assert_eq!(line(&mut r)["error"], json!("it failed"));
        writeln!(s, "GET / HTTP/1.1").unwrap();
        assert_eq!(line(&mut r)["ok"], json!(false));
        assert_eq!(line(&mut r), Json::Null, "closed after a line that isn't a request");
    }
}
