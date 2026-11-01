# Roadmap

> **Last updated:** 2026-11-01

Items have an id (`R<milestone>.<n>`), an owner (**maintainers** or **open**) and pending changes
(`- [ ]`). Each item points to the requirement it serves in [specification.md](specification.md)
(§ numbers) and has a GitHub issue titled `… (R<id>)`. Claim a pending change before starting
it: `— claimed: <name>, branch <branch>`.
Anything marked **decision** waits for a maintainer.

## R0 — Foundation (done)
- [x] R0.1 Workspace, lints, never-crash rules, layering check (`cargo xtask ci`)
- [x] R0.2 Versioned report model (bands, groups, sort, parameters, aggregates)
- [x] R0.3 `DataSource` seam with in-memory and CSV sources
- [x] R0.4 Banded layout: page breaks, group headers/footers, totals, page numbers; HTML writer
- [x] R0.5 Engine commands, CLI (`commands`, `render`, `run`, `sample`, `mcp`), MCP server

## R1 — Data connections (owner: open)
- [x] R1.1 SQLite source (read-only), with a query and typed columns (#1)
  - [ ] Pick the database file through a named connection once R1.3 lands, so an MCP agent can only open files the user chose (specification §11)
- [x] R1.2 PostgreSQL source (read-only role, statement timeout, row cap) — first target: Odoo databases (#2)
  - [ ] TLS connections (`sslmode=require`): only plain connections today — **decision**: rustls-based connector
  - [ ] Read `~/.pgpass` / `PGPASSFILE` as libpq does (PGPASSWORD works today)
  - [ ] Run the live PostgreSQL tests in CI (needs a PostgreSQL service in craft-common-actions or a local job)
- [ ] R1.3 Named connections stored through craft-settings; secrets in the OS keychain, never in reports
- [ ] R1.4 ODBC source (SQL Server and legacy databases such as Advantage) — **decision**: pure-Rust driver availability
- [x] R1.5 Report parameters passed to queries as bound parameters (never string-spliced) (#5)
- [x] R1.6 Query log (what ran, when, how many rows) (#6)
  - [ ] Show the log in the designer's data panel (R3)
  - [ ] Keep the log across runs (a file next to the settings) once named connections land (R1.3)
- [x] R1.7 Schema discovery: tables, views, columns, types, keys and relationships, with data preview (§2 FR-DATA-002) (#26)
  - [x] PostgreSQL schema discovery with R1.2 (same `SchemaSource` trait)
- [x] R1.8 Typed parameters: number, boolean, date, date range, single and multiple choice, with defaults, allowed values, validation and dependent parameters (§7) (#27)
  - [ ] Dependent parameters: choices computed from a query that uses other parameters (e.g. items of the chosen branch)
- [x] R1.9 Cancel long-running queries and renders; time and memory limits (§10, §13) (#28)
  - [ ] CLI: Ctrl+C sends PostgreSQL a cancel before exiting (today the server stops the statement at its time limit)
  - [ ] Designer: a Stop button and progress while a query or render runs (R3)
  - [ ] MCP: progress notifications for long commands
  - [ ] CPU limit (one core per render today; revisit with a server profile)
- [ ] R1.10 Optional cache of query results, with the refresh time shown (§2 FR-DATA-004, §5)

## R2 — Paginated reports (owner: open)
- [x] R2.1 Number, date and currency formats on fields (shared rules with GridCraft's numfmt where possible) (#7)
  - [ ] Day and month names in the report's language (`%A`, `%B` are English today)
  - [ ] Boolean format (`Yes`/`No`, `Oui`/`Non`) and a text format (upper case, prefix/suffix)
  - [ ] Dates stored as numbers (Unix time, Excel serials) read by the date format
- [ ] R2.2 Formula fields (expression language: arithmetic, text, dates, `if`) — **decision**: syntax
- [x] R2.3 Keep-together, repeat group header on new page, page break before group (#9)
  - [ ] Keep a band with the next one (a group header never alone at the bottom of a page)
  - [ ] Widow/orphan control for detail rows (at least N rows of a group on a page)
- [x] R2.4 Lines and boxes in bands (#10)
  - [ ] Images in bands: from a field (blob or path) or embedded in the report, with size limits and ATTRIBUTION rules
  - [ ] Text colour and background on text elements
- [ ] R2.5 Sub-reports
- [ ] R2.6 PDF export through craft-libs `craft-pdf`, fonts through craft-fonts
- [ ] R2.7 XLSX and DOCX export — **decision**: reuse GridCraft/WordCraft writers or own minimal writers
- [ ] R2.8 Printing through craft-libs `craft-print`
- [x] R2.9 Paper sizes (A5, Letter, custom), landscape, multiple columns (§4 FR-PRINT-002) (#30)
  - [x] Columns filled across then down (needed by labels, R2.12)
  - [ ] Different first page (no page header on page one, other margins)
- [ ] R2.10 Conditional visibility and suppression of sections and elements, conditional page breaks (§4 FR-PRINT-004) — needs R2.2
- [ ] R2.11 Barcodes and QR codes (§4 FR-PRINT-003)
- [x] R2.12 Labels: detail repeated across a grid of labels per page (§18.5) (#33)
  - [ ] Start printing at a given label (to reuse a partly used sheet)
- [x] R2.13 Golden rendering tests: reference reports whose page count and geometry are checked on every change (§13, §15) (#34)
  - [ ] Add a golden PDF check once PDF export lands (R2.6): page count and text positions read back from the file
- [x] R2.14 `report.validate` command: structured diagnostics for a definition without rendering it (§11, §15) (#35)
  - [ ] Check expressions once formula fields land (R2.2): syntax, types, unknown names

## R3 — Designer (owner: maintainers)
- [ ] R3.1 `ui-egui` crate (L6) on craft-ui and the suite's egui fork; banded canvas with live preview
- [ ] R3.2 Field list, drag-and-drop of fields into bands, property panel
- [ ] R3.3 Control channel (`--control PORT`, `reportcraft-cli send`) and offscreen snapshot, as in GridCraft
- [ ] R3.4 Undo/redo for definition edits
- [ ] R3.5 Interface translations through craft-i18n

## R4 — Formats and migration (owner: open)
- [ ] R4.1 Odoo QWeb templates: edit visually and preview on sample data — **decision**: scope
- [ ] R4.2 Power BI text project format (PBIP/TMDL) import, from public documentation
- [ ] R4.3 Crystal Reports `.rpt` read-only import — **decision**: only if a clean-room route exists

## R5 — Dashboards (owner: maintainers)
- [ ] R5.1 KPI tiles, charts, pivot tables, filters
- [ ] R5.2 Odoo o-spreadsheet dashboard import (format from public documentation)

## Order and dependencies
R1.1 → R1.2 → R1.5; R1.7 after R1.1; R1.8 before R1.5. R2.1 before R2.6/R2.7; R2.2 before R2.10; R2.13 early, before layout grows (R2.3, R2.9). R3.1 needs R2.6 (shared fonts). R4 and R5 after R3.1.
