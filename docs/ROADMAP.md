# Roadmap

> **Last updated:** 2026-11-11

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
  - [x] Pick the database file through a named connection (R1.3: `"saved": NAME`)
  - [x] Restrict MCP agents to saved connections (specification §11; see R1.3)
- [x] R1.2 PostgreSQL source (read-only role, statement timeout, row cap) — first target: Odoo databases (#2)
  - [ ] TLS connections (`sslmode=require`): only plain connections today — **decision**: rustls-based connector
  - [ ] Read `~/.pgpass` / `PGPASSFILE` as libpq does (PGPASSWORD works today)
  - [ ] Run the live PostgreSQL tests in CI (needs a PostgreSQL service in craft-common-actions or a local job)
- [x] R1.3 Named connections stored through craft-settings; secrets in the OS keychain, never in reports (#3)
  - [x] Let an MCP session use saved connections only (no raw file path or connection string), as §11 asks
  - [ ] Saved connections in report files by name (`"connection": "odoo-prod"`), so a report knows where its data lives
  - [ ] A connections panel in the designer (R3), with a password field that never shows the kept one
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
- [x] R1.10 Optional cache of query results, with the refresh time shown (§2 FR-DATA-004, §5) (#29)
  - [ ] Keep the cache across runs (on disk, next to the settings) once named connections land (R1.3)
  - [ ] Warn about stale data in the designer and dashboards (§5), with a per-report refresh policy

## R2 — Paginated reports (owner: open)
- [x] R2.1 Number, date and currency formats on fields (shared rules with GridCraft's numfmt where possible) (#7)
  - [x] Day and month names in the report's language (`%A`, `%B`, `%a`, `%b`, `%p`)
  - [ ] Boolean format (`Yes`/`No`, `Oui`/`Non`) and a text format (upper case, prefix/suffix)
  - [ ] Dates stored as numbers (Unix time, Excel serials) read by the date format
- [x] R2.2 Formula fields (expression language: arithmetic, text, dates, `if`) — decided: Excel-style formulas (`IF`, `SUM`, `ROUND`…, fields as `[table.field]`), the syntax most report users already know; no embedded scripting language (a shared report must not run code) (#8)
  - [x] `TEXT(value, format)` and numbers turned into text in the report's locale (`10,8` in French)
  - [ ] Totals in named formulas (a computed field that reads its group's sum), and running totals (`RUNNINGSUM`-style, page totals)
  - [x] Formulas on element properties: visibility and page breaks (R2.10), colour and background (R2.4)
  - [ ] `PAGENUMBER()` / `TOTALPAGES()` in formulas (needs a second pass, as "page N / M" does)
  - [ ] A formula editor in the designer with the function list, field names and live errors (R3)
- [x] R2.3 Keep-together, repeat group header on new page, page break before group (#9)
  - [x] Keep a band with the next one (a group header never alone at the bottom of a page): `keep_with_next` on group headers
  - [ ] `keep_with_next` on other bands (a detail row with the group footer, a report header with the first group)
  - [ ] Widow/orphan control for detail rows (at least N rows of a group on a page)
- [x] R2.4 Lines and boxes in bands (#10)
  - [ ] Images in bands: from a field (blob or path) or embedded in the report, with size limits and ATTRIBUTION rules
  - [x] Text colour and background on text elements, fixed or from a formula
  - [x] DOCX: fill an element's whole box with its background
- [x] R2.5 Sub-reports (#11)
  - [ ] Repeat a subreport's header on the pages it continues on
  - [ ] Expose a subreport's totals to the parent (§8 "results to the parent")
  - [ ] Pass parent fields to the subreport's query as parameters (one query per parent row), besides links
  - [ ] Subreports in labels (columns across) and in page headers/footers
- [x] R2.6 PDF export through craft-libs `craft-pdf`, fonts through craft-fonts (#12)
  - [ ] Embed craft-fonts at build time (`CRAFT_FONTS_DIR` read by a `build.rs`, as the suite does) instead of finding fonts at run time
  - [ ] Font fallback per character (a CJK or Arabic value in a Latin report), using craft-fonts' script fonts
  - [ ] Fonts named in the report definition (family, italic), with the font choice in the designer (R3)
  - [ ] PDF in the browser build: fonts passed as bytes by the page
  - [ ] PDF/A for archiving (Krilla validators), document outline from groups, links
- [x] R2.7 XLSX and DOCX export — decided: `rust_xlsxwriter` and `docx-rs` from crates.io (MIT/Apache-2.0); GridCraft and WordCraft writers are tied to their apps' models (#13)
  - [x] Lines and boxes in DOCX (pictures anchored behind the text)
  - [ ] Diagonal lines in DOCX; cell borders for lines and boxes in XLSX
  - [x] Barcodes as images in DOCX
  - [ ] Barcodes as images in XLSX
  - [ ] A "data only" XLSX: detail rows as a table with a header row (no group or report bands), for further analysis
  - [ ] One sheet per group, or per subreport, as an option
  - [ ] XLSX and DOCX in the browser build (the writers build for wasm; the clock-free path needs checking)
- [x] R2.8 Printing through craft-libs `craft-print` (#14)
  - [ ] The printer's own options (tray, paper type: `craft_print::printer_options`) and a print dialog in the designer (R3)
  - [ ] Page ranges for PDF, XLSX and DOCX exports too (`select_pages` exists)
  - [x] Mark commands that act outside the app (print, later publish) in the MCP tool list, and ask the user before an agent prints
  - [ ] Print progress and cancellation once the spooler has the job (craft-print reports Windows progress only)
- [x] R2.9 Paper sizes (A5, Letter, custom), landscape, multiple columns (§4 FR-PRINT-002) (#30)
  - [x] Columns filled across then down (needed by labels, R2.12)
  - [ ] Different first page (no page header on page one, other margins)
- [x] R2.10 Conditional visibility and suppression of sections and elements, conditional page breaks (§4 FR-PRINT-004) — needs R2.2 (#31)
  - [ ] Hide a band whose elements all come out blank (no formula needed)
  - [ ] `keep_together` and the space estimate of a group that count only the bands that will show
  - [x] Conditional colour and background on elements (`color_formula`, `background_formula`)
  - [ ] Conditional bold and format
  - [ ] Page break after a band, and "new page after N rows"
- [x] R2.11 Barcodes and QR codes (§4 FR-PRINT-003) (#32)
  - [ ] More symbologies on demand: Code 39, EAN-8, UPC-A, GS1-128 (FNC1), DataMatrix
  - [ ] QR numeric and alphanumeric modes (smaller symbols for digits and capitals)
  - [ ] Warn when modules come out narrower than a printer can render (about 0.25 mm)
- [x] R2.12 Labels: detail repeated across a grid of labels per page (§18.5) (#33)
  - [ ] Start printing at a given label (to reuse a partly used sheet)
- [x] R2.13 Golden rendering tests: reference reports whose page count and geometry are checked on every change (§13, §15) (#34)
  - [ ] Add a golden PDF check once PDF export lands (R2.6): page count and text positions read back from the file
- [x] R2.14 `report.validate` command: structured diagnostics for a definition without rendering it (§11, §15) (#35)
  - [x] Check expressions once formula fields land (R2.2): syntax, unknown fields and functions, argument counts, undeclared parameters, cycles
  - [ ] Check formula types (text where a number is needed) from the data's column types

## R3 — Designer (owner: maintainers)
- [x] R3.1 `ui-egui` crate (L6) on craft-ui and the suite's egui fork; banded canvas with live preview (#15)
  - [ ] craft-ui's controls, icons, themes and panes (the first version uses plain egui on the suite's fork), and the suite's menu bar (craft-menu)
  - [ ] Bold and the report's fonts in the preview (egui's default fonts have no bold face; craft-fonts)
  - [ ] Render the preview off the UI thread for large data, with progress and Stop (R1.9)
  - [x] Parameters panel: values for the report's parameters, used by the preview and the exports
  - [ ] Saving and opening connections from the designer (today: `reportcraft-cli connections save`)
  - [ ] Packaging: desktop entry, icon, RPM (as the suite's apps)
- [x] R3.2 Field list, drag-and-drop of fields into bands, property panel (#16)
  - [x] Groups, sort, named formulas and parameters edited in the designer (Report ▸ Groups, sort, formulas and parameters; `report.setStructure`)
  - [ ] Several elements selected at once; align, distribute, copy and paste
  - [ ] Band height dragged on the canvas (today in the band's properties)
  - [ ] Subreports, barcodes and parameters edited in the panel (today shown, edited in the file)
- [x] R3.3 Control channel (`--control PORT`, `reportcraft-cli send`) and offscreen snapshot, as in GridCraft (#17)
  - [ ] An MCP server connected to the running window (`reportcraft-cli mcp --connect PORT`), as GridCraft's
  - [ ] Snapshots without a window (an offscreen renderer for CI)
- [x] R3.4 Undo/redo for definition edits (#18)
  - [ ] Name each step in the menu ("Undo move element")
  - [ ] Keep the history with the file (today it lasts for the session)
- [x] R3.5 Interface translations through craft-i18n (#19)
  - [ ] Move `i18n/fr.tsv` to craft-i18n (`locales/fr/reportcraft.tsv`, with its `tools/sync_app.py`) and add other languages there
  - [ ] Remember the chosen language (craft-settings) and offer right-to-left languages once the layout mirrors
  - [ ] Translate engine messages (errors and diagnostics are English today)

## R4 — Formats and migration (owner: open)
- [ ] R4.1 Odoo QWeb templates: edit visually and preview on sample data — **decision**: scope
- [ ] R4.2 Power BI text project format (PBIP/TMDL) import, from public documentation
- [ ] R4.3 Crystal Reports `.rpt` read-only import — **decision**: only if a clean-room route exists

## R5 — Dashboards (owner: maintainers)
- [ ] R5.1 KPI tiles, charts, pivot tables, filters
  - [x] Charts in reports: bars, line, pie of a field totalled by category, over the rows a band summarises (#23)
  - [ ] More charts: several series, stacked bars, scatter, axes with ticks and gridlines, legends placed at will
  - [ ] Charts in XLSX exports (native Excel charts through rust_xlsxwriter)
  - [ ] KPI tiles, pivot tables, filters and an interactive dashboard page
- [ ] R5.2 Odoo o-spreadsheet dashboard import (format from public documentation)

## Order and dependencies
R1.1 → R1.2 → R1.5; R1.7 after R1.1; R1.8 before R1.5. R2.1 before R2.6/R2.7; R2.2 before R2.10; R2.13 early, before layout grows (R2.3, R2.9). R3.1 needs R2.6 (shared fonts). R4 and R5 after R3.1.
