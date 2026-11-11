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
  aligned), so the text stays editable; backgrounds, lines, boxes and barcodes are pictures anchored
  behind the text at their place (diagonal lines are left out with a warning). Both open in
  GridCraft and WordCraft; the same report gives the same file. XLSX leaves lines, boxes and
  barcodes out with a warning.
- Printing through craft-libs' `craft-print` (CUPS `lp` on Linux and macOS, the Windows print API):
  `file.printers` lists the printers, `file.print` renders the report to PDF and sends it with
  printer, copies (1–99), collation, duplex, grayscale, exact size or shrink to fit, and page ranges
  (`"1-3,5"`, `"8-"`). Nothing is sent without `"confirm": true`: otherwise the result shows the
  job it would send. CLI: `reportcraft-cli printers`, `reportcraft-cli print … [--printer NAME]
  [--copies N] [--duplex long|short|off] [--grayscale] [--fit] [--pages 1-3,5] [--dry-run]`.
- Text colour and background on elements (`style.color`, `style.background`, `#rgb` or `#rrggbb`),
  fixed or for each row from a formula (`color_formula`: `IF([amount] < 0, "#c00", "")`; a result
  that isn't a colour keeps the style's), in HTML, PDF, DOCX and XLSX.
- Charts (`"kind": "chart"`): bars, a line or a pie of a field totalled (sum, count, average,
  min, max) by category over the rows the band summarises, with a title, values in the element's
  format and a legend with shares for pies; drawn in HTML, PDF, DOCX and the designer's preview
  (XLSX leaves them out with a warning); inserted and edited in the designer.
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
- Group pagination: a group header kept with its first row (`keep_with_next`: never alone at the
  bottom of a page or column), keep a group on one page when it fits, repeat its header on the pages it
  continues on, start each group on a new page; never a blank page or an endless break.
- `report.validate` / `reportcraft-cli validate`: every error and warning with its location (unknown
  fields, text totals, undeclared parameters, elements past the margin or their band, fields in page
  headers/footers), checked against the loaded data's columns.
- Formats on fields and totals: number (decimals, grouping), currency (symbol before or after),
  percent, date (locale pattern or `strftime` pattern; ISO, SQL and RFC 3339 inputs), following the
  report's `locale` (`en`, `en-US`, `en-GB`, `fr`, `fr-CH`, `de`, `de-CH`, `es`, `it`, `nl`, `pt`), day and
  month names included (`%A %-d %B` writes "mercredi 11 novembre" in `fr`).
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
  band). Operators `+ - * / ^ % &` and comparisons with Excel's precedence; 61 functions: logical
  (`IF`, `IFS`, `SWITCH`, `AND`, `OR`, `NOT`, `IFERROR`, `IS…`), maths (`ROUND`, `ROUNDUP`,
  `ROUNDDOWN`, `INT`, `MOD`, `POWER`, `SQRT`, `ABS`, `SIGN`), aggregates (`SUM`, `AVERAGE`, `COUNT`,
  `COUNTA`, `MIN`, `MAX`, `SUMIF`, `COUNTIF`, `AVERAGEIF` with `">10"`, `"<>"`, `"Ly*"` criteria),
  text (`CONCAT`, `LEFT`, `MID`, `SUBSTITUTE`, `FIND`, `SEARCH`, `PROPER`, `TRIM`…), dates (`DATE`,
  `EDATE`, `EOMONTH`, `DATEDIF`, `DAYS`, `WEEKDAY`, `ISOWEEKNUM`, `TODAY`/`NOW` from the render's
  clock…) and `PARAM("name")` (typed; `PARAM("period.from")` for a range's ends). Blanks, errors
  (`#DIV/0!`, `#VALUE!`, `#NAME?`, `#NUM!`, `#N/A`) and `IFERROR` behave as in Excel; dates are
  their own values (a date plus days, a date minus a date). Numbers written into text follow the
  report's locale (`"Average " & 10.8` is `Average 10,8` in French), and `TEXT(value, "#,##0.00 €")`
  or `TEXT(date, "dddd d mmmm")` takes Excel format codes, written with `.` and `,` as in English
  Excel and shown with the locale's separators and day and month names. `report.validate` reports syntax
  errors with their position, unknown fields, argument counts, undeclared parameters and cycles;
  `report.evaluateFormula` tries a formula on the loaded rows.
- Conditions (formulas): `visible_if` on bands (a hidden band takes no room) and on elements (their
  room stays), `page_break_before_if` on bands. TRUE or a non-zero number holds; FALSE, 0 or a blank
  doesn't; an error never hides a band or element and never breaks a page. Hidden rows still count
  in their group's totals. A hidden page header takes no room, a hidden page footer keeps its room.
  `report.validate` checks the conditions as formulas.
- MCP agents work within what the user set up (§11): `reportcraft-cli mcp` reaches databases only
  through saved connections and can propose, but not confirm, printing or changing a connection
  (`--allow-paths`, `--allow-actions` widen it). Every tool carries MCP annotations (read-only,
  edits the session, or acts outside the application).
- Designer (`cargo run -p reportcraft`, R3.1, R3.2): opens the sample, a report file, CSV data or
  rows from a saved connection; shows every band to scale with its elements (`[field]`,
  `SUM([amount])`, `=formula`…); fields and parameters dragged from the field list onto a band,
  elements moved (into another band too) and resized with the mouse on a 0.5 mm grid, deleted with
  Delete, inserted from the Insert menu (text, field, formula, total, page number, line, box); the
  properties panel edits the selected element (position, size, content, style, colours, format,
  conditions), band (height, conditions, keep with next) or the report (title, language, paper,
  orientation, margins); a window edits the groups, sort, named formulas and parameters
  (`report.setStructure`), the field list takes values for the parameters, and Report ▸ Check lists
  `report.validate`'s findings; File ▸ Print… prints (printer, copies, pages, two-sided, black and
  white, shrink to fit; Check shows the job without sending it); Data ▸ Connections… saves, tests
  and deletes connections (the password to the keychain); barcodes and charts are inserted and
  edited like the rest; previews the rendered pages live after every change; saves the report and
  exports PDF, HTML, XLSX and DOCX. Undo and redo (Edit menu, Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y)
  every change to the report, also as `edit.undo` / `edit.redo` for the CLI and MCP (100 steps).
  The interface follows the system's language or `--lang CODE` (View ▸ Language), through
  craft-i18n's catalogs embedded at build time (`CRAFT_I18N_DIR=…/craft-i18n`; English without
  them); French is complete (ReportCraft's own strings in `i18n/fr.tsv`, the shared ones in
  craft-i18n). `--screenshot FILE.ppm` saves the window and quits. `--control PORT` lets scripts and agents drive
  the open window over 127.0.0.1 (a token in a file only its user can read; `reportcraft-cli send
  PORT METHOD [JSON]`): any engine command run as an agent would (saved connections only, no
  confirmed external action), `ui.state`, `ui.screenshot`, `app.quit`.
- CLI and MCP server reaching every command. 181 tests (plus 8 live PostgreSQL tests run on demand), including 16 golden rendering snapshots; `cargo xtask ci` green.

## Not yet
- No ODBC source yet (see `ROADMAP.md`).
- Not yet built or tested on Windows, macOS, BSD or wasm.
