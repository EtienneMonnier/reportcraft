# Status

> **Last updated:** 2026-11-01 · Stage: foundation (pre-alpha)

## Works
- Report definitions in JSON, validated, with a format version.
- CSV data (quoted fields, separator detection, type inference).
- SQLite data (native builds): one read-only query per report, typed columns, row cap, 30 s time limit
  (`data.loadSqlite`, `reportcraft-cli render … --sqlite FILE --query SQL`).
- SQLite schema discovery: tables, views, columns (declared and ReportCraft types, nullability,
  primary keys) and foreign keys, plus table previews (`data.describeSqlite`, `data.previewSqlite`,
  `reportcraft-cli schema --sqlite FILE [--table T] [--preview N] [--json]`).
- Banded layout: report/page headers and footers, nested groups, detail, sort, sum/count/avg/min/max,
  parameters, page breaks, "page N / M"; HTML output, printable from a browser.
- `report.validate` / `reportcraft-cli validate`: every error and warning with its location (unknown
  fields, text totals, undeclared parameters, elements past the margin or their band, fields in page
  headers/footers), checked against the loaded data's columns.
- CLI and MCP server reaching every command. 31 tests, including 5 golden rendering snapshots; `cargo xtask ci` green.

## Not yet
- No PostgreSQL or ODBC source yet, no number formats, no PDF, no designer UI (see `ROADMAP.md`).
- Not yet built or tested on Windows, macOS, BSD or wasm.
