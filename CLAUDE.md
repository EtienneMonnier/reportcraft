Read `AGENTS.md`, then `docs/pitfalls.md`.

New workspace crates must include `[lints] workspace = true` in `Cargo.toml` and be registered in
`xtask/src/main.rs`; production crate roots must begin with the required crate-level lint attribute.
