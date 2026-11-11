# Status

> **Last updated:** 2026-11-11 · Stage: foundation (pre-alpha)

## Works
- Report definitions in JSON, validated, with a format version.
- CSV data (quoted fields, separator detection, type inference).
- SQLite data (native builds): one read-only query per report, typed columns, row cap, 30 s time limit
  (`data.loadSqlite`, `reportcraft-cli render … --sqlite FILE --query SQL`).
- PostgreSQL data (native builds): one query in a read-only transaction, read through a cursor in
  batches, statement timeout, row cap; schema discovery across schemas (only what the user may
  read) and previews (`data.loadPostgres`, `data.describePostgres`, `data.previewPostgres`,
  `--postgres CONN` in `render`, `validate` and `schema`). Plain connections only (no TLS yet).
- Saved connections: a SQLite file or a PostgreSQL host, port, user and database saved by name in
  the suite's settings file (craft-settings, `reportcraft/config.toml`, or `REPORTCRAFT_CONFIG_DIR`),
  the password in the system keychain (Secret Service, macOS Keychain, Windows Credential Manager),
  never in a file, a result or the query log. `data.connections`, `data.saveConnection` and
  `data.deleteConnection` (both change nothing without `"confirm": true`), `data.testConnection`;
  every SQLite and PostgreSQL command takes `"saved": NAME` instead of a path or connection string.
  CLI: `reportcraft-cli connections [list|save|delete|test]` (`--password-stdin`), and
  `--connection NAME` in `render`, `validate`, `print` and `schema`.
- SQLite schema discovery: tables, views, columns (declared and ReportCraft types, nullability,
  primary keys) and foreign keys, plus table previews (`data.describeSqlite`, `data.previewSqlite`,
  `reportcraft-cli schema --sqlite FILE [--table T] [--preview N] [--json]`).
- Banded layout: report/page headers and footers, nested groups, detail, sort, sum/count/avg/min/max,
  parameters, page breaks, "page N / M"; HTML output, printable from a browser.
- PDF output (`report.render` with `"format": "pdf"`, `reportcraft-cli render … --format pdf --out
  FILE.pdf`) through craft-libs' `craft-pdf`: selectable, searchable text with embedded font subsets
  (any script the font covers, French spaces and accents included), measured right and centre
  alignment, lines, rectangles and barcodes as vectors. Fonts come from `REPORTCRAFT_FONT` /
  `REPORTCRAFT_FONT_BOLD`, a craft-fonts checkout (`CRAFT_FONTS_DIR`, Inter) or the system
  (Liberation Sans, DejaVu Sans, Noto Sans, Arial); characters the font lacks are reported as
  warnings. The same document gives the same file (no clock in the PDF).
- XLSX and DOCX output (`"format": "xlsx" | "docx"`, `--format xlsx|docx --out FILE`), written with
  `rust_xlsxwriter` and `docx-rs`. XLSX: one continuous sheet (the page header once, no page
  footer, no page breaks), a row per line of the report and a column per left edge, numbers,
  booleans and dates kept typed with Excel number formats from the element formats (`#,##0.00 "€"`,
  `0.0%`, `dd/mm/yyyy`), bold, size and alignment kept. DOCX: the report's pages and margins, a
  paragraph per line at its height and a tab stop per item (left, right or centred as the item is
  aligned), so the text stays editable. Both open in GridCraft and WordCraft; the same report gives
  the same file. Lines, boxes and barcodes are left out with a warning.
- Printing through craft-libs' `craft-print` (CUPS `lp` on Linux and macOS, the Windows print API):
  `file.printers` lists the printers, `file.print` renders the report to PDF and sends it with
  printer, copies (1–99), collation, duplex, grayscale, exact size or shrink to fit, and page ranges
  (`"1-3,5"`, `"8-"`). Nothing is sent without `"confirm": true`: otherwise the result shows the
  job it would send. CLI: `reportcraft-cli printers`, `reportcraft-cli print … [--printer NAME]
  [--copies N] [--duplex long|short|off] [--grayscale] [--fit] [--pages 1-3,5] [--dry-run]`.
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
- Formulas, Excel-style (`reportcraft-expr`): named formulas (`formulas: [{name, expression}]`) are
  fields of every row, usable in elements, groups, sort, totals, links and other formulas (in
  dependency order; a circle is an error); `formula` elements compute over the current row and the
  rows their band summarises (`[amount] / SUM([amount])` is a share of the group in the detail
  band). Operators `+ - * / ^ % &` and comparisons with Excel's precedence; 60 functions: logical
  (`IF`, `IFS`, `SWITCH`, `AND`, `OR`, `NOT`, `IFERROR`, `IS…`), maths (`ROUND`, `ROUNDUP`,
  `ROUNDDOWN`, `INT`, `MOD`, `POWER`, `SQRT`, `ABS`, `SIGN`), aggregates (`SUM`, `AVERAGE`, `COUNT`,
  `COUNTA`, `MIN`, `MAX`, `SUMIF`, `COUNTIF`, `AVERAGEIF` with `">10"`, `"<>"`, `"Ly*"` criteria),
  text (`CONCAT`, `LEFT`, `MID`, `SUBSTITUTE`, `FIND`, `SEARCH`, `PROPER`, `TRIM`…), dates (`DATE`,
  `EDATE`, `EOMONTH`, `DATEDIF`, `DAYS`, `WEEKDAY`, `ISOWEEKNUM`, `TODAY`/`NOW` from the render's
  clock…) and `PARAM("name")` (typed; `PARAM("period.from")` for a range's ends). Blanks, errors
  (`#DIV/0!`, `#VALUE!`, `#NAME?`, `#NUM!`, `#N/A`) and `IFERROR` behave as in Excel; dates are
  their own values (a date plus days, a date minus a date). `report.validate` reports syntax
  errors with their position, unknown fields, argument counts, undeclared parameters and cycles;
  `report.evaluateFormula` tries a formula on the loaded rows.
- Conditions (formulas): `visible_if` on bands (a hidden band takes no room) and on elements (their
  room stays), `page_break_before_if` on bands. TRUE or a non-zero number holds; FALSE, 0 or a blank
  doesn't; an error never hides a band or element and never breaks a page. Hidden rows still count
  in their group's totals. A hidden page header takes no room, a hidden page footer keeps its room.
  `report.validate` checks the conditions as formulas.
- CLI and MCP server reaching every command. 142 tests (plus 8 live PostgreSQL tests run on demand), including 14 golden rendering snapshots; `cargo xtask ci` green.

## Not yet
- No ODBC source yet, no designer UI (see `ROADMAP.md`).
- Not yet built or tested on Windows, macOS, BSD or wasm.
