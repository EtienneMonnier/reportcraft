//! `reportcraft-cli`: headless ReportCraft. Every subcommand goes through engine commands.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufReader, IsTerminal, Write};
use std::process::ExitCode;

use reportcraft_engine::{EngineError, Session, commands, sample};
use serde_json::{Value as Json, json};

const USAGE: &str = "\
reportcraft-cli <subcommand>

  commands [--json]                         every engine command with its parameters
  render <report.json|--sample NAME> [--data FILE.csv | (--sqlite FILE.db | --postgres CONN | --sqlserver CONN) --query SQL] [--format html|pdf|xlsx|docx|json] [--param NAME=VALUE]... (repeat a name for several values) [--out FILE] [--sub NAME=FILE.csv]... [--sub-query NAME=SQL]... [--query-log] [--timeout SECONDS] [--max-memory MB]
  validate <report.json|--sample NAME> [--data FILE.csv | (--sqlite FILE.db | --postgres CONN | --sqlserver CONN) --query SQL] [--json] [--sub NAME=FILE.csv]... [--sub-query NAME=SQL]... [--query-log] [--timeout SECONDS] [--max-memory MB]
                                            errors and warnings, checked against the data when given
  (--sub NAME=FILE.csv and --sub-query NAME=SQL load a dataset for subreports; --sub-query runs on
   the same database, after --query;
   --query-log prints each query run, its time and row count, to stderr;
   --timeout SECONDS limits the query and the render, default 30 and 120;
   --max-memory MB limits the rows read, default 1024)
  print <report.json|--sample NAME> [data options as for render] [--param NAME=VALUE]... [--printer NAME] [--copies N] [--duplex long|short|off] [--grayscale] [--fit] [--pages 1-3,5] [--dry-run]
                                            print through the system spooler; --dry-run only shows the job
  printers                                  the system's printers
  send PORT METHOD [JSON]                   one request to the designer started with `reportcraft --control PORT`
                                            (an engine command id, ui.state, ui.screenshot with a path to a .ppm file, app.quit)
  connections [list]                        saved connections (definitions in the settings file, passwords in the keychain)
  connections save NAME (--sqlite FILE | [--sqlserver] --host HOST [--port N] --user USER --database DB [--sslmode MODE] [--password-stdin]) [--description TEXT]
                                            (PostgreSQL unless --sqlserver; ports 5432 and 1433 by default)
  connections delete NAME | connections test NAME
  (--connection NAME --query SQL loads from a saved connection in render, validate and print;
   schema takes --connection NAME too)
  schema (--sqlite FILE | --postgres CONN | --sqlserver CONN) [--table NAME] [--preview N] [--json]
                                            tables, columns and keys; rows with --preview
  (--postgres CONN is a libpq connection string or postgresql:// URL, the password in PGPASSWORD;
   --sqlserver CONN is an ADO.NET connection string, Server=tcp:HOST,1433;User Id=USER;Database=DB,
   the password in SQLCMDPASSWORD)
  run [--sample NAME] --cmd ID=JSON...      run commands in order, print each result
  sample <NAME>                             print a sample's report definition
  mcp [--allow-paths] [--allow-actions]     MCP server on stdio; an agent uses saved connections only and
                                            can't confirm prints or connection changes unless allowed
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
        "print" => print(rest),
        "printers" => printers(),
        "connections" => connections(rest),
        "send" => send(rest),
        "run" => run_commands(rest),
        "schema" => schema(rest),
        "validate" => validate(rest),
        "sample" => print_sample(rest.first().map(String::as_str).unwrap_or("rentals")),
        "mcp" => {
            let mut s = Session::default();
            // An agent gets saved connections only and can't confirm external actions, unless
            // the person starting the server allows more.
            let mut policy = reportcraft_engine::Policy::AGENT;
            for flag in rest {
                match flag.as_str() {
                    "--allow-paths" => policy.raw_sources = true,
                    "--allow-actions" => policy.external_actions = true,
                    other => return Err(format!("unknown option `{other}` (mcp takes --allow-paths, --allow-actions)")),
                }
            }
            reportcraft_mcp::serve_with(BufReader::new(std::io::stdin()), std::io::stdout(), &mut s, policy).map_err(|e| e.to_string())
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
    let (mut database, mut query, mut show_log): (Option<Database>, _, _) = (None, None, false);
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
            "--sqlite" | "--postgres" | "--sqlserver" | "--connection" => {
                if database.is_some() {
                    return Err("give one database: `--sqlite`, `--postgres`, `--sqlserver` or `--connection`".to_owned());
                }
                database = Some(database_of(&mut s, a, flag_value(&mut it, a)?)?);
            }
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
    let mut loaded = match (&database, query) {
        (Some(db), Some(query)) => exec(&mut s, db.kind.load, load((db.key, db.value.clone()), query)).map(drop),
        (None, None) => Ok(()),
        (Some(_), None) => Err("the database needs `--query SQL`".to_owned()),
        (None, Some(_)) => Err("`--query` goes with `--sqlite FILE`, `--postgres CONN`, `--sqlserver CONN` or `--connection NAME`".to_owned()),
    };
    for (name, sql) in sub_queries {
        if loaded.is_err() {
            break;
        }
        let Some(db) = &database else {
            loaded = Err("`--sub-query` needs a database (`--sqlite`, `--postgres`, `--sqlserver` or `--connection`)".to_owned());
            break;
        };
        let mut p = load((db.key, db.value.clone()), sql);
        if let Some(map) = p.as_object_mut() {
            map.insert("dataset".to_owned(), Json::String(name));
        }
        loaded = exec(&mut s, db.kind.load, p).map(drop);
    }
    // Shown even when the query failed: that's when it helps most.
    if show_log {
        eprint!("{}", query_log_text(&exec(&mut s, "data.queryLog", json!({}))?));
    }
    if max_memory.is_some() && s.rows.columns.is_empty() && loaded.is_ok() {
        eprintln!("reportcraft-cli: `--max-memory` applies to database data only");
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
    let file = ["pdf", "xlsx", "docx"].into_iter().find_map(|f| result.get(format!("{f}Base64")).and_then(Json::as_str).map(|b| (f, b)));
    let body = match (result.get("html").and_then(Json::as_str), file) {
        (Some(html), _) => html.as_bytes().to_vec(),
        (_, Some((kind, encoded))) => {
            if out.is_none() && std::io::stdout().is_terminal() {
                return Err(format!("a {} file isn't printed to a terminal: add `--out FILE.{kind}`", kind.to_uppercase()));
            }
            reportcraft_engine::base64::decode(encoded).ok_or_else(|| format!("the engine returned a damaged {} file", kind.to_uppercase()))?
        }
        _ => serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?.into_bytes(),
    };
    match out {
        Some(path) => std::fs::write(&path, body).map_err(|e| format!("{path}: {e}")),
        None => std::io::stdout().write_all(&body).map_err(|e| e.to_string()),
    }
}

/// The commands of one kind of database, and the flag that names it on the command line.
struct Kind {
    name: &'static str,
    flag: &'static str,
    /// The command parameter that holds the flag's value (file or connection string).
    key: &'static str,
    load: &'static str,
    describe: &'static str,
    preview: &'static str,
}

const KINDS: [Kind; 3] = [
    Kind { name: "sqlite", flag: "--sqlite", key: "path", load: "data.loadSqlite", describe: "data.describeSqlite", preview: "data.previewSqlite" },
    Kind {
        name: "postgres",
        flag: "--postgres",
        key: "connection",
        load: "data.loadPostgres",
        describe: "data.describePostgres",
        preview: "data.previewPostgres",
    },
    Kind {
        name: "sqlserver",
        flag: "--sqlserver",
        key: "connection",
        load: "data.loadSqlServer",
        describe: "data.describeSqlServer",
        preview: "data.previewSqlServer",
    },
];

/// A database named on the command line: its kind, and the parameter that reaches it.
struct Database {
    kind: &'static Kind,
    key: &'static str,
    value: String,
}

/// The database a flag names: `--sqlite FILE`, `--postgres CONN`, `--sqlserver CONN`, or
/// `--connection NAME` (a saved connection, whose kind is looked up).
fn database_of(s: &mut Session, flag: &str, value: &str) -> Result<Database, String> {
    if flag == "--connection" {
        let list = exec(s, "data.connections", json!({}))?;
        let kind = list["connections"].as_array().into_iter().flatten().find(|c| c["name"] == json!(value)).and_then(|c| c["kind"].as_str());
        let kind = KINDS
            .iter()
            .find(|k| Some(k.name) == kind)
            .ok_or_else(|| format!("no saved connection `{value}` (see `reportcraft-cli connections`)"))?;
        return Ok(Database { kind, key: "saved", value: value.to_owned() });
    }
    let kind = KINDS.iter().find(|k| k.flag == flag).ok_or_else(|| format!("unknown option `{flag}`"))?;
    Ok(Database { kind, key: kind.key, value: value.to_owned() })
}

/// `connections [list]`, `connections save NAME …`, `connections delete NAME`, `connections test NAME`.
fn connections(args: &[String]) -> Result<(), String> {
    let mut s = Session::default();
    let (action, rest) = args.split_first().map_or(("list", &[][..]), |(a, r)| (a.as_str(), r));
    match action {
        "list" => {
            let r = exec(&mut s, "data.connections", json!({}))?;
            let list = r["connections"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                println!("no saved connection");
            }
            for c in list {
                let key = if c["password"] == json!(true) { "  (password in the keychain)" } else { "" };
                println!(
                    "{:<16} {:<9} {}{key}",
                    c["name"].as_str().unwrap_or(""),
                    c["kind"].as_str().unwrap_or(""),
                    c["target"].as_str().unwrap_or("")
                );
            }
            if let Some(path) = r["settings"].as_str() {
                println!("settings: {path}");
            }
            Ok(())
        }
        "save" => {
            let (name, flags) = rest.split_first().ok_or("`connections save` needs a NAME")?;
            let mut p = json!({"name": name, "confirm": true});
            let mut it = flags.iter();
            while let Some(a) = it.next() {
                let key = match a.as_str() {
                    "--sqlite" => {
                        p["kind"] = json!("sqlite");
                        "path"
                    }
                    "--sslmode" => "sslmode",
                    "--sqlserver" => {
                        p["kind"] = json!("sqlserver");
                        continue;
                    }
                    "--host" | "--user" | "--database" => {
                        if p.get("kind").is_none() {
                            p["kind"] = json!("postgres");
                        }
                        a.trim_start_matches("--")
                    }
                    "--description" => "description",
                    "--port" => {
                        let port = flag_value(&mut it, a)?.parse::<u16>().map_err(|_| "`--port` expects a port number")?;
                        p["port"] = json!(port);
                        continue;
                    }
                    "--password-stdin" => {
                        // Read from stdin, never from the command line (shell history, process list).
                        let mut line = String::new();
                        std::io::stdin().read_line(&mut line).map_err(|e| e.to_string())?;
                        p["password"] = json!(line.trim_end_matches(['\n', '\r']));
                        continue;
                    }
                    other => return Err(format!("unknown option `{other}`")),
                };
                p[key] = json!(flag_value(&mut it, a)?);
            }
            let r = exec(&mut s, "data.saveConnection", p)?;
            println!("saved {} ({})", r["connection"]["name"].as_str().unwrap_or(""), r["connection"]["target"].as_str().unwrap_or(""));
            Ok(())
        }
        "delete" => {
            let name = rest.first().ok_or("`connections delete` needs a NAME")?;
            let r = exec(&mut s, "data.deleteConnection", json!({"name": name, "confirm": true}))?;
            if r["deleted"] == json!(true) {
                println!("deleted {name}")
            } else {
                println!("no saved connection `{name}`")
            }
            Ok(())
        }
        "test" => {
            let name = rest.first().ok_or("`connections test` needs a NAME")?;
            let r = exec(&mut s, "data.testConnection", json!({"name": name}))?;
            println!("{name}: ok, {} table(s) or view(s)", r["tables"]);
            Ok(())
        }
        other => Err(format!("unknown `connections` action `{other}` (list, save, delete, test)")),
    }
}

/// One request to a running designer over its control channel.
fn send(args: &[String]) -> Result<(), String> {
    use std::io::BufRead;
    let port: u16 = args.first().ok_or("`send` needs PORT METHOD [JSON]")?.parse().map_err(|_| "PORT must be a port number")?;
    let method = args.get(1).ok_or("`send` needs PORT METHOD [JSON]")?;
    let params: Json = match args.get(2) {
        Some(t) => serde_json::from_str(t).map_err(|e| format!("the parameters are not JSON: {e}"))?,
        None => json!({}),
    };
    let token = reportcraft_engine::control::read_token(port)
        .map_err(|e| format!("no designer on port {port} ({e}); start one with `reportcraft --control {port}`"))?;
    let stream = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("can't reach the designer on port {port}: {e}"))?;
    let mut reader = std::io::BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut writer = stream;
    let mut line = String::new();
    let mut ask = |request: Json, line: &mut String| -> Result<Json, String> {
        writeln!(writer, "{request}").map_err(|e| e.to_string())?;
        line.clear();
        reader.read_line(line).map_err(|e| e.to_string())?;
        serde_json::from_str(line).map_err(|_| "the designer closed the connection".to_owned())
    };
    let auth = ask(json!({"auth": token}), &mut line)?;
    if auth["ok"] != json!(true) {
        return Err(auth["error"].as_str().unwrap_or("refused").to_owned());
    }
    let reply = ask(json!({"id": 1, "method": method, "params": params}), &mut line)?;
    if reply["ok"] == json!(true) {
        println!("{}", serde_json::to_string_pretty(&reply["result"]).map_err(|e| e.to_string())?);
        Ok(())
    } else {
        Err(reply["error"].as_str().unwrap_or("failed").to_owned())
    }
}

fn print(args: &[String]) -> Result<(), String> {
    let mut job = serde_json::Map::new();
    let mut dry_run = false;
    let (mut s, params, timeout) = load_inputs(args, |a, it| {
        match a {
            "--printer" | "--duplex" | "--pages" => {
                job.insert(a.trim_start_matches("--").to_owned(), json!(flag_value(it, a)?));
            }
            "--copies" => {
                let n = flag_value(it, a)?.parse::<u64>().map_err(|_| "`--copies` expects a whole number")?;
                job.insert("copies".to_owned(), json!(n));
            }
            "--grayscale" | "--fit" => {
                job.insert(a.trim_start_matches("--").to_owned(), json!(true));
            }
            "--dry-run" => dry_run = true,
            other => return Err(format!("unknown option `{other}`")),
        }
        Ok(())
    })?;
    job.insert("params".to_owned(), Json::Object(params));
    job.insert("confirm".to_owned(), json!(!dry_run));
    if let Some(t) = timeout {
        job.insert("timeoutSeconds".to_owned(), json!(t));
    }
    let r = exec(&mut s, "file.print", Json::Object(job))?;
    for w in r.get("warnings").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str) {
        eprintln!("warning: {w}");
    }
    let command: Vec<&str> = r["command"].as_array().into_iter().flatten().filter_map(Json::as_str).collect();
    let printer = r["printer"].as_str().unwrap_or("the default printer");
    println!("{} page(s) of {}, {} copy(ies), to {printer}", r["pages"], r["of"], r["copies"]);
    println!("job: {}", command.join(" "));
    match r["spooler"].as_str() {
        Some(msg) => println!("{}", msg.trim()),
        None => println!("dry run: nothing was sent"),
    }
    Ok(())
}

fn printers() -> Result<(), String> {
    let r = exec(&mut Session::default(), "file.printers", json!({}))?;
    let list = r["printers"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("no printer found");
    }
    for p in list {
        let mark = if p["default"] == json!(true) { "*" } else { " " };
        let description = p["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!("  ({d})")).unwrap_or_default();
        println!("{mark} {}{description}", p["name"].as_str().unwrap_or(""));
    }
    Ok(())
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
    let (mut database, mut table, mut preview, mut as_json): (Option<Database>, _, _, _) = (None, None, None, false);
    let mut s = Session::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sqlite" | "--postgres" | "--sqlserver" | "--connection" => {
                if database.is_some() {
                    return Err("give one database: `--sqlite`, `--postgres`, `--sqlserver` or `--connection`".to_owned());
                }
                database = Some(database_of(&mut s, a, flag_value(&mut it, a)?)?);
            }
            "--table" => table = Some(flag_value(&mut it, a)?.to_owned()),
            "--preview" => preview = Some(flag_value(&mut it, a)?.parse::<u64>().map_err(|_| "`--preview` expects a number of rows")?),
            "--json" => as_json = true,
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    let db = database.ok_or("`schema` needs one of `--sqlite FILE`, `--postgres CONN`, `--sqlserver CONN` or `--connection NAME`")?;
    let (describe, preview_cmd) = (db.kind.describe, db.kind.preview);
    let source = json!({db.key: db.value});
    let with = |extra: Json| {
        let mut p = source.clone();
        if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        p
    };
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
