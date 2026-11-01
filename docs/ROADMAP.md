# Roadmap

> **Last updated:** 2026-11-01

Items have an id (`R<milestone>.<n>`), an owner (**maintainers** or **open**) and pending changes
(`- [ ]`). Claim a pending change before starting it: `— claimed: <name>, branch <branch>`.
Anything marked **decision** waits for a maintainer.

## R0 — Foundation (done)
- [x] R0.1 Workspace, lints, never-crash rules, layering check (`cargo xtask ci`)
- [x] R0.2 Versioned report model (bands, groups, sort, parameters, aggregates)
- [x] R0.3 `DataSource` seam with in-memory and CSV sources
- [x] R0.4 Banded layout: page breaks, group headers/footers, totals, page numbers; HTML writer
- [x] R0.5 Engine commands, CLI (`commands`, `render`, `run`, `sample`, `mcp`), MCP server

## R1 — Data connections (owner: open)
- [ ] R1.1 SQLite source (read-only), with a query and typed columns
- [ ] R1.2 PostgreSQL source (read-only role, statement timeout, row cap) — first target: Odoo databases
- [ ] R1.3 Named connections stored through craft-settings; secrets in the OS keychain, never in reports
- [ ] R1.4 ODBC source (SQL Server and legacy databases such as Advantage) — **decision**: pure-Rust driver availability
- [ ] R1.5 Report parameters passed to queries as bound parameters (never string-spliced)
- [ ] R1.6 Query log (what ran, when, how many rows)

## R2 — Paginated reports (owner: open)
- [ ] R2.1 Number, date and currency formats on fields (shared rules with GridCraft's numfmt where possible)
- [ ] R2.2 Formula fields (expression language: arithmetic, text, dates, `if`) — **decision**: syntax
- [ ] R2.3 Keep-together, repeat group header on new page, page break before group
- [ ] R2.4 Lines, boxes and images in bands
- [ ] R2.5 Sub-reports
- [ ] R2.6 PDF export through craft-libs `craft-pdf`, fonts through craft-fonts
- [ ] R2.7 XLSX and DOCX export — **decision**: reuse GridCraft/WordCraft writers or own minimal writers
- [ ] R2.8 Printing through craft-libs `craft-print`

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
R1.1 → R1.2 → R1.5. R2.1 before R2.6/R2.7. R3.1 needs R2.6 (shared fonts). R4 and R5 after R3.1.
