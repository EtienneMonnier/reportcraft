# ReportCraft — instructions for agents and contributors

ReportCraft is a clean-room, open-source, Rust-native reporting tool: paginated reports in the
spirit of Crystal Reports and SSRS first, interactive dashboards in the spirit of Power BI next.
It follows the conventions of the ArtCraft Crafting Apps (`storytold/gridcraft`, `deckcraft`,
`wordcraft`…) so that it can live beside them: same workspace layout, lints, never-crash rules,
command model, CLI and MCP server. Display name **ReportCraft**; machine names `reportcraft-*`.

## Start every session here
1. Read `docs/STATUS.md` (what works today) and pick work from `docs/ROADMAP.md`. What the
   product must do is in `docs/specification.md` (functional requirements); how it is built is here.
2. Read `docs/pitfalls.md` before your first change. `plan/` is gitignored (local only): it holds
   clean-room observations (`plan/observed/`) and working notes.

## Clean room — absolute rule
- Crystal Reports, SSRS, Power BI and similar products may only be **observed black-box**: run
  them with synthetic data, read their **public** documentation, write down what you see in
  `plan/observed/`. Screenshots stay there and are never committed.
- **Never** read, disassemble or decompile anything inside those products, never commit files they
  produced (`.rpt`, `.pbix`, exported PDFs), never copy their templates, icons or wording beyond
  feature names. Customer reports are not test fixtures.
- Never copy GPL/AGPL/LGPL code. Odoo (QWeb) and o-spreadsheet are LGPL: their *formats* and
  *behaviour* may be studied from documentation, their code may not be copied.
- No vendor iconography, fonts, templates or sample files, ever. Every asset is original, CC0 or
  openly licensed and has a row in `ATTRIBUTION.md`. Prefer art and samples defined in code.

## Never crash
People trust ReportCraft with their numbers. **This outranks feature work.**
- No panics in non-test code: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`,
  `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- Production crate roots begin with
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`,
  and every crate's `Cargo.toml` has `[lints] workspace = true`.
- Errors are `Result<T, E>` through the crate's error type and `?`.
- Input-derived numbers are hostile (report files, data, SQL results, CLI/MCP params): use `get()`
  instead of indexing, checked/saturating arithmetic, and the caps in `reportcraft_core::limits`.
- Bound recursion. `Session::execute` catches escaped panics and restores the session: a safety
  net, not a licence. Every crash fix lands with a regression test
  (`every_command_survives_empty_params` runs every command with empty parameters).

## Non-negotiables
- **Everything is a command.** User-visible behaviour = a `CommandSpec` in
  `crates/engine/src/cmd.rs` (id, label, menu, params doc, `run`) + tests. Ids follow the menus
  (`file.new`, `data.loadCsv`, `report.render`). The CLI, MCP server and the future UI and control
  channel reach all of them.
- **Programmatic calls never open dialogs**: empty params run with defaults or return an error.
- **Layering** is enforced by `cargo xtask layers`: `core` (L0) → `model` (L1) → `data` (L2) →
  `layout` (L3) → `engine` (L5) → `mcp`/`ui-egui` (L6) → `apps/*` (L7). Nothing below L6 depends on
  egui/eframe/winit/rfd. Register every new crate in `xtask/src/main.rs`.
- **Extend through seams.** A new data source is a `DataSource` implementation; a new output is a
  writer over `layout::Document`. No new arm in a `match` over source kinds.
- **Versioned files.** The report format carries `format_version`; changing it needs a migration.
- **Read-only data access** by default. Credentials never in reports, logs, tests or the repo.
- **Time goes through `chrono`.** Never hand-roll calendar conversion.
- **Rust only.** Never break wasm for L0–L3.
- **Reuse the suite.** Shared code comes from `storytold/craft-libs` (craft-pdf, craft-print,
  craft-ui, craft-settings, craft-i18n…) pinned by `rev`, fonts from `storytold/craft-fonts`
  (optional, `CRAFT_FONTS_DIR`), translations from `storytold/craft-i18n` (optional,
  `CRAFT_I18N_DIR`). Don't reimplement what they provide.

## Quality gates
Before every commit: `cargo xtask ci` (fmt, clippy `-D warnings`, tests, layers).

## Git workflow
- Never commit to `main`. Branch from an up-to-date `main`: `feature/`, `bugfix/`, `docs/`,
  `test/`, `refactor/` or `chore/`, plus the roadmap id (`feature/r1.2-sqlite-source`).
- One roadmap item per PR, small and reviewable, no drive-by refactors. PRs are squash-merged, so
  **the PR title is the commit message**: plain English, present tense, what changed for the person
  or developer, with the issue number when there is one — for example
  `Group footers show the sum of their own rows (#12)` or `CSV: a quoted separator stays in its field`.
  Tooling commits may use a short prefix: `ci: …`, `docs: …`, `packaging: …`.
- Done means: gates pass, new behaviour has tests, `docs/ROADMAP.md` ticked and `docs/STATUS.md`
  updated in the same PR. Report what you verified and what you didn't.

## Running it
- `cargo run -p reportcraft-cli -- render --sample rentals --out /tmp/rentals.html`
- `cargo run -p reportcraft-cli -- commands` · `… run --sample rentals --cmd 'data.preview={"limit":3}'`
- `cargo run -p reportcraft-cli -- mcp` (MCP server on stdio; tools are command ids with `_`).

## CI workflows
`.github/workflows/` only calls the shared workflows of `storytold/craft-common-actions` at
`@main`. Don't add jobs of our own, and don't change CI in a feature or fix PR.
