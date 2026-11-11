//! Repository tasks: `cargo xtask ci` (every quality gate) and `cargo xtask layers`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Every crate and its layer. A crate may depend only on crates of a lower layer.
/// Register a new crate here.
const LAYERS: &[(&str, u8)] = &[
    ("reportcraft-core", 0),
    ("reportcraft-barcode", 0),
    ("reportcraft-expr", 0),
    ("reportcraft-model", 1),
    ("reportcraft-data", 2),
    ("reportcraft-layout", 3),
    // Native data sources: they implement `reportcraft-data`'s trait, so they sit above it.
    ("reportcraft-sqlite", 3),
    ("reportcraft-postgres", 3),
    // Writers over `reportcraft-layout`'s Document (HTML lives in layout itself).
    ("reportcraft-pdf", 4),
    ("reportcraft-engine", 5),
    ("reportcraft-mcp", 6),
    ("reportcraft-ui-egui", 6),
    ("reportcraft-cli", 7),
    ("reportcraft", 7),
];

/// Below this layer, no crate may use a UI toolkit (the UI crate is swappable).
const UI_LAYER: u8 = 6;
const UI_CRATES: &[&str] = &["egui", "eframe", "winit", "rfd", "egui_extras"];

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "layers" => layers(),
        "ci" => ci(),
        _ => Err("usage: cargo xtask <ci|layers>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn cargo(args: &[&str]) -> Result<(), String> {
    println!("$ cargo {}", args.join(" "));
    let status =
        Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())).args(args).current_dir(root()).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("cargo {} failed", args.join(" "))) }
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])?;
    cargo(&["test", "--workspace"])?;
    layers()?;
    println!("xtask ci: all gates passed");
    Ok(())
}

fn manifests() -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for dir in ["crates", "apps"] {
        let Ok(entries) = std::fs::read_dir(root().join(dir)) else { continue };
        for e in entries.flatten() {
            let m = e.path().join("Cargo.toml");
            if m.is_file() {
                out.push(m);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The package name and dependency names of a manifest (enough TOML for our own files).
fn read_manifest(path: &Path) -> Result<(String, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut name, mut deps, mut section) = (String::new(), Vec::new(), String::new());
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        if section == "package" && key == "name" {
            name = value.trim().trim_matches('"').to_owned();
        } else if section.ends_with("dependencies") && !section.starts_with("dev") {
            deps.push(key.split('.').next().unwrap_or(key).to_owned());
        }
    }
    Ok((name, deps))
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

fn layers() -> Result<(), String> {
    let mut problems = Vec::new();
    for m in manifests()? {
        let (name, deps) = read_manifest(&m)?;
        let Some(layer) = layer_of(&name) else {
            problems.push(format!("{name}: not registered in xtask LAYERS"));
            continue;
        };
        for d in &deps {
            if let Some(dl) = layer_of(d)
                && dl >= layer
            {
                problems.push(format!("{name} (L{layer}) depends on {d} (L{dl})"));
            }
            if layer < UI_LAYER && UI_CRATES.contains(&d.as_str()) {
                problems.push(format!("{name} (L{layer}) depends on UI crate {d}"));
            }
        }
    }
    if problems.is_empty() {
        println!("xtask layers: ok");
        Ok(())
    } else {
        Err(format!("layering violations:\n  {}", problems.join("\n  ")))
    }
}
