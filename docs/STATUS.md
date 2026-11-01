# Status

> **Last updated:** 2026-11-01 · Stage: foundation (pre-alpha)

## Works
- Report definitions in JSON, validated, with a format version.
- CSV data (quoted fields, separator detection, type inference).
- SQLite data (native builds): one read-only query per report, typed columns, row cap, 30 s time limit
  (`data.loadSqlite`, `reportcraft-cli render … --sqlite FILE --query SQL`).
- Banded layout: report/page headers and footers, nested groups, detail, sort, sum/count/avg/min/max,
  parameters, page breaks, "page N / M"; HTML output, printable from a browser.
- CLI and MCP server reaching every command. 22 tests; `cargo xtask ci` green.

## Not yet
- No PostgreSQL or ODBC source yet, no number formats, no PDF, no designer UI (see `ROADMAP.md`).
- Not yet built or tested on Windows, macOS, BSD or wasm.
