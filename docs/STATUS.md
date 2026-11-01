# Status

> **Last updated:** 2026-11-01 · Stage: foundation (pre-alpha)

## Works
- Report definitions in JSON, validated, with a format version.
- CSV data (quoted fields, separator detection, type inference).
- SQLite data (native builds): one read-only query per report, typed columns, row cap, 30 s time limit
  (`data.loadSqlite`, `reportcraft-cli render … --sqlite FILE --query SQL`).
- PostgreSQL data (native builds): one query in a read-only transaction, read through a cursor in
  batches, statement timeout, row cap; schema discovery across schemas (only what the user may
  read) and previews (`data.loadPostgres`, `data.describePostgres`, `data.previewPostgres`,
  `--postgres CONN` in `render`, `validate` and `schema`). Plain connections only (no TLS yet).
- SQLite schema discovery: tables, views, columns (declared and ReportCraft types, nullability,
  primary keys) and foreign keys, plus table previews (`data.describeSqlite`, `data.previewSqlite`,
  `reportcraft-cli schema --sqlite FILE [--table T] [--preview N] [--json]`).
- Banded layout: report/page headers and footers, nested groups, detail, sort, sum/count/avg/min/max,
  parameters, page breaks, "page N / M"; HTML output, printable from a browser.
- Lines (horizontal, vertical, diagonal) and rectangles (border, fill) in any band, drawn as SVG in
  the HTML output; colours are `#rgb`/`#rrggbb` only.
- Pages: A3, A4, A5, Letter, Legal or custom sizes, portrait or landscape, 1 to 16 columns with a
  gap (group and detail bands flow down a column then the next; report and page headers and
  footers span the page).
- Labels: `column_order: across` fills a row of columns with detail bands, then the next row;
  group headers and footers start a new row.
- Group pagination: keep a group on one page when it fits, repeat its header on the pages it
  continues on, start each group on a new page; never a blank page or an endless break.
- `report.validate` / `reportcraft-cli validate`: every error and warning with its location (unknown
  fields, text totals, undeclared parameters, elements past the margin or their band, fields in page
  headers/footers), checked against the loaded data's columns.
- Formats on fields and totals: number (decimals, grouping), currency (symbol before or after),
  percent, date (locale pattern or `strftime` pattern; ISO, SQL and RFC 3339 inputs), following the
  report's `locale` (`en`, `en-US`, `en-GB`, `fr`, `fr-CH`, `de`, `de-CH`, `es`, `it`, `nl`, `pt`).
- Typed parameters: text, integer, number, boolean, date, date range; allowed values, several
  values, required, defaults; checked before rendering, listed by `report.parameters`, shown with
  formats (a date range writes both ends).
- Parameters in queries, bound and never spliced: `:name`, `:range.from` / `:range.to`, `IN (:list)`
  (`data.loadSqlite` / `data.loadPostgres` take `params`; the CLI's `--param` reaches the query).
- CLI and MCP server reaching every command. 67 tests (plus 8 live PostgreSQL tests run on demand), including 10 golden rendering snapshots; `cargo xtask ci` green.

## Not yet
- No ODBC source yet, no PDF, no designer UI (see `ROADMAP.md`).
- Not yet built or tested on Windows, macOS, BSD or wasm.
