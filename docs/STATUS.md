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
- Barcodes: Code 128, EAN-13 (check digit added or verified) and QR code (UTF-8, levels L/M/Q/H,
  versions 1–40), from a field or a fixed text, drawn as SVG with their quiet zone; a value the
  symbology can't hold is written as text. Every symbology was checked with an independent decoder
  (zxing-cpp), from the encoder and from a rendered page.
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
- Query log: every SQLite and PostgreSQL query the session ran, with its start time, target
  (file, or `user@host:port/database`, never the password), SQL as sent, bound values, duration and
  row count or error; the newest 200 are kept (`data.queryLog`, `data.clearQueryLog`,
  `--query-log` in `render` and `validate`, printed to stderr even when the query fails).
- Subreports: a `subreport` element lays out another report over a named dataset (loaded with
  `"dataset": name`, `data.datasets`; CLI `--sub NAME=FILE.csv`, `--sub-query NAME=SQL`), with
  links that keep the rows matching the current row (`1` matches `"1"`; a missing value matches
  nothing). It flows below its band across columns and pages, band by band; with no rows it is
  hidden (or shows its headers and footers); a missing dataset is an error; reports nest 3 deep.
  `report.validate` checks subreport fields and links against the datasets.
- Query cache: a load with `cacheSeconds` reuses the same query's rows (same source, SQL, values
  and limits) read less than that long ago; at most 8 results and 1 GiB are kept, for the session
  (`data.clearCache`). Loads and `report.render` answer `refreshedAt`; reports can show it
  (`refreshed_at` content) and when they were rendered (`printed_at`), in the locale's date and time
  or a date format. Loaded rows are shared, so a command no longer copies them.
- Cancellation and limits: a running query or render stops on request (`Session::cancel_handle`;
  MCP `notifications/cancelled`), SQLite through its progress handler, PostgreSQL by a cancel sent
  to the server; time limits on queries (30 s) and renders (120 s), memory limit on the rows read
  (1 GiB by default), all configurable (`timeoutSeconds`, `maxMegabytes`, CLI `--timeout`,
  `--max-memory`). A stopped command leaves the session as it was.
- CLI and MCP server reaching every command. 94 tests (plus 8 live PostgreSQL tests run on demand), including 12 golden rendering snapshots; `cargo xtask ci` green.

## Not yet
- No ODBC source yet, no PDF, no designer UI (see `ROADMAP.md`).
- Not yet built or tested on Windows, macOS, BSD or wasm.
