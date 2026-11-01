//! `reportcraft-cli`: headless ReportCraft. Every subcommand goes through engine commands.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufReader, IsTerminal, Write};
use std::process::ExitCode;

use reportcraft_engine::{EngineError, Session, commands, sample};
use serde_json::{Value as Json, json};

const USAGE: &str = "\
reportcraft-cli <subcommand>

  commands [--json]                         every engine command with its parameters
  render <report.json|--sample NAME> [--data FILE.csv | --sqlite FILE.db --query SQL | --postgres CONN --query SQL] [--format html|pdf|json] [--param NAME=VALUE]... (repeat a name for several values) [--out FILE] [--sub NAME=FILE.csv]... [--sub-query NAME=SQL]... [--query-log] [--timeout SECONDS] [--max-memory MB]
  validate <report.json|--sample NAME> [--data FILE.csv | --sqlite FILE.db --query SQL | --postgres CONN --query SQL] [--json] [--sub NAME=FILE.csv]... [--sub-query NAME=SQL]... [--query-log] [--timeout SECONDS] [--max-memory MB]
                                            errors and warnings, checked against the data when given
  (--sub NAME=FILE.csv and --sub-query NAME=SQL load a dataset for subreports; --sub-query runs on
   the --sqlite or --postgres database, after --query;
   --query-log prints each query run, its time and row count, to stderr;
   --timeout SECONDS limits the query and the render, default 30 and 120;
   --max-memory MB limits the rows read, default 1024)
  schema (--sqlite FILE | --postgres CONN) [--table NAME] [--preview N] [--json]
                                            tables, columns and keys; rows with --preview
  (CONN is a libpq connection string or postgresql:// URL; put the password in PGPASSWORD)
  run [--sample NAME] --cmd ID=JSON...      run commands in order, print each result
  sample <NAME>                             print a sample's report definition
  mcp                                       MCP server on stdio
  version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("reportcraft-cli: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let (sub, rest) = args.split_first().ok_or_else(|| USAGE.to_owned())?;
    match sub.as_str() {
        "commands" => list_commands(rest.iter().any(|a| a == "--json")),
        "render" => render(rest),
        "run" => run_commands(rest),
        "schema" => schema(rest),
        "validate" => validate(rest),
        "sample" => print_sample(rest.first().map(String::as_str).unwrap_or("rentals")),
        "mcp" => {
            let mut s = Session::default();
            reportcraft_mcp::serve(BufReader::new(std::io::stdin()), std::io::stdout(), &mut s).map_err(|e| e.to_string())
        }
        "version" | "--version" => {
            println!("reportcraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown subcommand `{other}`\n\n{USAGE}")),
    }
}

fn exec(s: &mut Session, id: &str, params: Json) -> Result<Json, String> {
    s.execute(id, &params).map_err(|e: EngineError| e.to_string())
}

fn list_commands(as_json: bool) -> Result<(), String> {
    if as_json {
        let list: Vec<Json> = commands().iter().map(|c| json!({"id": c.id, "label": c.label, "menu": c.menu, "params": c.params})).collect();
        println!("{}", Json::Array(list));
    } else {
        for c in commands() {
            println!("{:<18} {:<24} {}", c.id, c.label, c.params);
        }
    }
    Ok(())
}

/// Reads the value after a flag.
fn flag_value<'a>(it: &mut impl Iterator<Item = &'a String>, flag: &str) -> Result<&'a str, String> {
    it.next().map(String::as_str).ok_or_else(|| format!("`{flag}` needs a value"))
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

/// The session with the report and data loaded, the `--param` values, and `--timeout`.
type Loaded = (Session, serde_json::Map<String, Json>, Option<u64>);

/// Loads the report and data named by the shared options (`REPORT.json`, `--sample`, `--data`,
/// `--sqlite` with `--query`) and hands every other option to `other`.
fn load_inputs<'a>(
    args: &'a [String],
    mut other: impl FnMut(&'a str, &mut std::slice::Iter<'a, String>) -> Result<(), String>,
) -> Result<Loaded, String> {
    let mut s = Session::default();
    let (mut timeout, mut max_memory) = (None, None);
    let number = |v: &str, flag: &str| v.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| format!("`{flag}` expects a positive whole number"));
    let (mut sqlite, mut postgres, mut query, mut show_log) = (None, None, None, false);
    let mut sub_queries: Vec<(String, String)> = Vec::new();
    let named =
        |v: &str, flag: &str| v.split_once('=').map(|(n, x)| (n.to_owned(), x.to_owned())).ok_or_else(|| format!("`{flag}` expects NAME=VALUE"));
    let mut params = serde_json::Map::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--param" => {
                // Repeating a name gives a parameter several values.
                let (k, v) = flag_value(&mut it, a)?.split_once('=').ok_or("`--param` expects NAME=VALUE")?;
                let v = Json::String(v.to_owned());
                match params.get_mut(k) {
                    Some(Json::Array(list)) => list.push(v),
                    Some(first) => *first = Json::Array(vec![first.take(), v]),
                    None => {
                        params.insert(k.to_owned(), v);
                    }
                }
            }
            "--sample" => {
                exec(&mut s, "file.openSample", json!({"name": flag_value(&mut it, a)?}))?;
            }
            "--data" => {
                exec(&mut s, "data.loadCsv", json!({"csv": read(flag_value(&mut it, a)?)?}))?;
            }
            "--sqlite" => sqlite = Some(flag_value(&mut it, a)?.to_owned()),
            "--postgres" => postgres = Some(flag_value(&mut it, a)?.to_owned()),
            "--query" => query = Some(flag_value(&mut it, a)?.to_owned()),
            "--query-log" => show_log = true,
            "--sub" => {
                let (name, path) = named(flag_value(&mut it, a)?, a)?;
                exec(&mut s, "data.loadCsv", json!({"csv": read(&path)?, "dataset": name}))?;
            }
            "--sub-query" => sub_queries.push(named(flag_value(&mut it, a)?, a)?),
            "--timeout" => timeout = Some(number(flag_value(&mut it, a)?, a)?),
            "--max-memory" => max_memory = Some(number(flag_value(&mut it, a)?, a)?),
            path if !path.starts_with("--") => {
                exec(&mut s, "report.load", json!({"json": read(path)?}))?;
            }
            flag => other(flag, &mut it)?,
        }
    }
    let load = |source: (&str, String), query: String| {
        let mut p = json!({"query": query, "params": params, "timeoutSeconds": timeout, "maxMegabytes": max_memory});
        if let Some(map) = p.as_object_mut() {
            map.insert(source.0.to_owned(), Json::String(source.1));
            map.retain(|_, v| !v.is_null());
        }
        p
    };
    let database = match (&sqlite, &postgres) {
        (Some(path), None) => Some(("data.loadSqlite", ("path", path.clone()))),
        (None, Some(connection)) => Some(("data.loadPostgres", ("connection", connection.clone()))),
        _ => None,
    };
    let mut loaded = match (sqlite, postgres, query) {
        (Some(path), None, Some(query)) => exec(&mut s, "data.loadSqlite", load(("path", path), query)).map(drop),
        (None, Some(connection), Some(query)) => exec(&mut s, "data.loadPostgres", load(("connection", connection), query)).map(drop),
        (None, None, None) => Ok(()),
        (Some(_), Some(_), _) => Err("use either `--sqlite` or `--postgres`".to_owned()),
        _ => Err("`--query` goes with `--sqlite FILE` or `--postgres CONNECTION`".to_owned()),
    };
    for (name, sql) in sub_queries {
        if loaded.is_err() {
            break;
        }
        let Some((command, source)) = database.clone() else {
            loaded = Err("`--sub-query` needs `--sqlite FILE` or `--postgres CONNECTION`".to_owned());
            break;
        };
        let mut p = load(source, sql);
        if let Some(map) = p.as_object_mut() {
            map.insert("dataset".to_owned(), Json::String(name));
        }
        loaded = exec(&mut s, command, p).map(drop);
    }
    // Shown even when the query failed: that's when it helps most.
    if show_log {
        eprint!("{}", query_log_text(&exec(&mut s, "data.queryLog", json!({}))?));
    }
    if max_memory.is_some() && s.rows.columns.is_empty() && loaded.is_ok() {
        eprintln!("reportcraft-cli: `--max-memory` applies to `--sqlite` and `--postgres` data only");
    }
    loaded.map(|()| (s, params, timeout))
}

/// One line per query: when, where, how long, how many rows (or the error), then the SQL and its values.
fn query_log_text(log: &Json) -> String {
    let mut out = String::new();
    for e in log["entries"].as_array().into_iter().flatten() {
        let text = |k: &str| e[k].as_str().unwrap_or("").to_owned();
        let outcome = match e["rows"].as_u64() {
            Some(n) if e["fromCache"] == json!(true) => format!("{n} row(s) from the cache"),
            Some(n) => format!("{n} row(s)"),
            None => format!("failed: {}", text("error")),
        };
        out.push_str(&format!("query  {}  {} {}  {} ms  {outcome}\n", text("at"), text("source"), text("target"), e["durationMs"]));
        out.push_str(&format!("       {}\n", text("sql").split_whitespace().collect::<Vec<_>>().join(" ")));
        let values: Vec<String> =
            e["values"].as_array().into_iter().flatten().map(|v| v.as_str().map_or_else(|| "NULL".to_owned(), |s| format!("{s:?}"))).collect();
        if !values.is_empty() {
            out.push_str(&format!("       values: {}\n", values.join(", ")));
        }
    }
    out
}

fn render(args: &[String]) -> Result<(), String> {
    let (mut format, mut out) = ("html".to_owned(), None);
    let (mut s, params, timeout) = load_inputs(args, |a, it| {
        match a {
            "--format" => format = flag_value(it, a)?.to_owned(),
            "--out" => out = Some(flag_value(it, a)?.to_owned()),
            other => return Err(format!("unknown option `{other}`")),
        }
        Ok(())
    })?;
    let mut render = json!({"format": format, "params": params});
    if let (Some(t), Some(map)) = (timeout, render.as_object_mut()) {
        map.insert("timeoutSeconds".to_owned(), json!(t));
    }
    let result = exec(&mut s, "report.render", render)?;
    for w in result.get("warnings").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str) {
        eprintln!("warning: {w}");
    }
    let body = match (result.get("html").and_then(Json::as_str), result.get("pdfBase64").and_then(Json::as_str)) {
        (Some(html), _) => html.as_bytes().to_vec(),
        (_, Some(pdf)) => {
            if out.is_none() && std::io::stdout().is_terminal() {
                return Err("a PDF isn't printed to a terminal: add `--out FILE.pdf`".to_owned());
            }
            reportcraft_engine::base64::decode(pdf).ok_or("the engine returned a damaged PDF")?
        }
        _ => serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?.into_bytes(),
    };
    match out {
        Some(path) => std::fs::write(&path, body).map_err(|e| format!("{path}: {e}")),
        None => std::io::stdout().write_all(&body).map_err(|e| e.to_string()),
    }
}

fn run_commands(args: &[String]) -> Result<(), String> {
    let mut s = Session::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sample" => {
                exec(&mut s, "file.openSample", json!({"name": flag_value(&mut it, a)?}))?;
            }
            "--cmd" => {
                let spec = flag_value(&mut it, a)?;
                let (id, params) = spec.split_once('=').unwrap_or((spec, "{}"));
                let params: Json = serde_json::from_str(params).map_err(|e| format!("{id}: parameters are not JSON: {e}"))?;
                println!("{}", exec(&mut s, id, params)?);
            }
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    Ok(())
}

fn print_sample(name: &str) -> Result<(), String> {
    let (report, _) = sample::by_name(name).ok_or_else(|| format!("no sample `{name}` (samples: {})", sample::NAMES.join(", ")))?;
    println!("{}", report.to_json().map_err(|e| e.to_string())?);
    Ok(())
}

fn schema(args: &[String]) -> Result<(), String> {
    let (mut path, mut connection, mut table, mut preview, mut as_json) = (None, None, None, None, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sqlite" => path = Some(flag_value(&mut it, a)?.to_owned()),
            "--postgres" => connection = Some(flag_value(&mut it, a)?.to_owned()),
            "--table" => table = Some(flag_value(&mut it, a)?.to_owned()),
            "--preview" => preview = Some(flag_value(&mut it, a)?.parse::<u64>().map_err(|_| "`--preview` expects a number of rows")?),
            "--json" => as_json = true,
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    let (describe, preview_cmd, source) = match (path, connection) {
        (Some(p), None) => ("data.describeSqlite", "data.previewSqlite", json!({"path": p})),
        (None, Some(c)) => ("data.describePostgres", "data.previewPostgres", json!({"connection": c})),
        _ => return Err("`schema` needs `--sqlite FILE` or `--postgres CONNECTION`".to_owned()),
    };
    let with = |extra: Json| {
        let mut p = source.clone();
        if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        p
    };
    let mut s = Session::default();
    if let (Some(n), Some(t)) = (preview, table.as_deref()) {
        let rows = exec(&mut s, preview_cmd, with(json!({"table": t, "limit": n})))?;
        println!("{}", if as_json { rows.to_string() } else { preview_text(&rows) });
        return Ok(());
    }
    if preview.is_some() {
        return Err("`--preview` needs `--table NAME`".to_owned());
    }
    let described = exec(&mut s, describe, with(json!({"table": table})))?;
    if as_json {
        println!("{described}");
        return Ok(());
    }
    let tables = match described.get("tables").and_then(Json::as_array) {
        Some(list) => list.clone(),
        None => vec![described],
    };
    for t in &tables {
        println!("{} ({})", t["name"].as_str().unwrap_or(""), t["kind"].as_str().unwrap_or(""));
        for c in t["columns"].as_array().into_iter().flatten() {
            let pk = c["primary_key"].as_u64().map_or(String::new(), |p| format!(" PK{p}"));
            let null = if c["nullable"].as_bool().unwrap_or(true) { "" } else { " NOT NULL" };
            println!(
                "  {:<24} {:<16} {}{pk}{null}",
                c["name"].as_str().unwrap_or(""),
                c["declared_type"].as_str().unwrap_or(""),
                c["field_type"].as_str().unwrap_or("")
            );
        }
        for fk in t["foreign_keys"].as_array().into_iter().flatten() {
            let cols = |v: &Json| v.as_array().into_iter().flatten().filter_map(Json::as_str).collect::<Vec<_>>().join(", ");
            println!("  ({}) -> {}({})", cols(&fk["columns"]), fk["references_table"].as_str().unwrap_or(""), cols(&fk["references_columns"]));
        }
    }
    Ok(())
}

fn preview_text(rows: &Json) -> String {
    let header: Vec<&str> = rows["columns"].as_array().into_iter().flatten().filter_map(|c| c["name"].as_str()).collect();
    let mut out = header.join("\t");
    for r in rows["rows"].as_array().into_iter().flatten() {
        let cells: Vec<String> = r
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| v.as_str().map_or_else(|| if v.is_null() { String::new() } else { v.to_string() }, str::to_owned))
            .collect();
        out.push('\n');
        out.push_str(&cells.join("\t"));
    }
    out
}

fn validate(args: &[String]) -> Result<(), String> {
    let mut as_json = false;
    let (mut s, _, _) = load_inputs(args, |a, _| {
        if a == "--json" {
            as_json = true;
            Ok(())
        } else {
            Err(format!("unknown option `{a}`"))
        }
    })?;
    let result = exec(&mut s, "report.validate", json!({}))?;
    if as_json {
        println!("{result}");
    } else {
        for d in result["diagnostics"].as_array().into_iter().flatten() {
            println!(
                "{:<7} {:<28} {}",
                d["severity"].as_str().unwrap_or(""),
                d["location"].as_str().unwrap_or(""),
                d["message"].as_str().unwrap_or("")
            );
        }
        let data = if result["dataChecked"] == json!(true) { "checked against the data" } else { "no data loaded: fields not checked" };
        println!("{} error(s), {} warning(s); {data}", result["errors"], result["warnings"]);
    }
    if result["valid"] == json!(true) { Ok(()) } else { Err("the report has errors".to_owned()) }
}
