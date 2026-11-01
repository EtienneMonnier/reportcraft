//! `reportcraft-cli`: headless ReportCraft. Every subcommand goes through engine commands.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{BufReader, Write};
use std::process::ExitCode;

use reportcraft_engine::{EngineError, Session, commands, sample};
use serde_json::{Value as Json, json};

const USAGE: &str = "\
reportcraft-cli <subcommand>

  commands [--json]                         every engine command with its parameters
  render <report.json|--sample NAME> [--data FILE.csv] [--format html|json] [--param NAME=VALUE]... [--out FILE]
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

fn render(args: &[String]) -> Result<(), String> {
    let mut s = Session::default();
    let (mut format, mut out, mut params) = ("html".to_owned(), None, serde_json::Map::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--sample" => {
                exec(&mut s, "file.openSample", json!({"name": flag_value(&mut it, a)?}))?;
            }
            "--data" => {
                exec(&mut s, "data.loadCsv", json!({"csv": read(flag_value(&mut it, a)?)?}))?;
            }
            "--format" => format = flag_value(&mut it, a)?.to_owned(),
            "--out" => out = Some(flag_value(&mut it, a)?.to_owned()),
            "--param" => {
                let (k, v) = flag_value(&mut it, a)?.split_once('=').ok_or("`--param` expects NAME=VALUE")?;
                params.insert(k.to_owned(), Json::String(v.to_owned()));
            }
            path if !path.starts_with("--") => {
                exec(&mut s, "report.load", json!({"json": read(path)?}))?;
            }
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    let result = exec(&mut s, "report.render", json!({"format": format, "params": params}))?;
    let body = match result.get("html").and_then(Json::as_str) {
        Some(html) => html.to_owned(),
        None => serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?,
    };
    match out {
        Some(path) => std::fs::write(&path, body).map_err(|e| format!("{path}: {e}")),
        None => std::io::stdout().write_all(body.as_bytes()).map_err(|e| e.to_string()),
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
