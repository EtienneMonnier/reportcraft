//! Embeds the interface translations (craft-i18n's recipe, `docs/integration.md` in
//! <https://github.com/storytold/craft-i18n>): with `CRAFT_I18N_DIR` set to a craft-i18n checkout,
//! one entry per locale that has a ReportCraft catalog, with the shared catalog of that locale.
//! ReportCraft's own catalogs are in this repository (`i18n/<locale>.tsv`) until they move to
//! craft-i18n (`locales/<locale>/reportcraft.tsv`, which wins when both exist). Unset, the table is
//! empty and the interface is English. `CRAFT_I18N_REQUIRED=1` makes a missing checkout an error.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).unwrap_or_default();
    let own = manifest.join("../../i18n");
    println!("cargo::rerun-if-changed={}", own.display());
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v == "1");
    let mut src = String::from(
        "/// (locale, shared catalog, ReportCraft's catalog), one entry per locale; \"\" for a missing file.\npub static CRAFT_I18N: &craft_i18n::Embedded = &[\n",
    );
    match std::env::var_os("CRAFT_I18N_DIR").map(PathBuf::from) {
        Some(dir) if dir.join("locales").is_dir() => {
            println!("cargo::rerun-if-changed={}", dir.join("locales").display());
            for (locale, shared, app) in locales(&dir.join("locales"), &own) {
                let _ = writeln!(src, "    ({locale:?}, {}, {}),", include(shared.as_deref()), include(app.as_deref()));
            }
        }
        Some(dir) => {
            let msg = format!("CRAFT_I18N_DIR={} is not a craft-i18n checkout (no locales/): the interface is English only", dir.display());
            if required {
                println!("cargo::error={msg}");
            } else {
                println!("cargo::warning={msg}");
            }
        }
        None if required => println!("cargo::error=CRAFT_I18N_REQUIRED=1 but CRAFT_I18N_DIR is not set"),
        None => {}
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_i18n.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::warning=writing {}: {e}", out.display());
    }
}

/// `include_str!` of an absolute path (written with `{:?}` so Windows backslashes are escaped), or "".
fn include(path: Option<&Path>) -> String {
    match path {
        Some(p) => {
            println!("cargo::rerun-if-changed={}", p.display());
            format!("include_str!({:?})", p.display().to_string())
        }
        None => "\"\"".to_owned(),
    }
}

/// Every locale with a ReportCraft catalog (craft-i18n's, else this repository's), and its files.
fn locales(locales: &Path, own: &Path) -> Vec<(String, Option<PathBuf>, Option<PathBuf>)> {
    let Ok(entries) = std::fs::read_dir(locales) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let code = e.file_name().to_string_lossy().into_owned();
        let in_checkout = e.path().join("reportcraft.tsv");
        let in_repo = own.join(format!("{code}.tsv"));
        let app = if in_checkout.is_file() {
            Some(in_checkout)
        } else if in_repo.is_file() {
            Some(in_repo)
        } else {
            None
        };
        if app.is_none() {
            continue;
        }
        let shared = Some(e.path().join("common.tsv")).filter(|p| p.is_file());
        out.push((code, shared, app));
    }
    out.sort();
    out
}
