# Decisions

## Follow the Crafting Apps' conventions from the first commit

ReportCraft mirrors GridCraft's workspace (layers, `cargo xtask`, lints, `rustfmt.toml`,
`clippy.toml`), its command model and CLI/MCP shape, and craft-anything's contribution rules, so
it can be proposed to the suite (storytold/artcraft#2638) without restructuring. The bundle id and
repository location are left to the maintainers if it is adopted.

## Report header before page header on page one

On the first page the report header prints above the page header, as users of banded report
tools expect; later pages start with the page header.

## Hand-written CSV and JSON-RPC, no CLI argument crate

Kept dependencies to `serde`, `serde_json`, `thiserror`, `log`, as the sibling apps' lower layers
do. Revisit when the CLI grows.

## SQLite through rusqlite, in its own native-only crate (R1.1)

SQLite is read with `rusqlite` and its bundled SQLite build, so every platform gets the same
engine without a system library. The bundled C library does not build for wasm, so the source
lives in its own crate, `reportcraft-sqlite`, placed at L3 because it implements the L2
`DataSource` trait; the engine depends on it only off wasm, and `data.loadSqlite` answers
"unsupported" in the web build. Our crates stay free of `unsafe`; rusqlite's FFI is safe to use
from safe code, as AGENTS.md requires of dependencies.

Read-only is enforced three ways: the file is opened read-only (as a `file:` URI whose `?`, `#`
and `%` are escaped, so a file name cannot add URI parameters), the statement must be a single
one that SQLite reports as read-only, and `ATTACH` is limited to zero databases. `PRAGMA
query_only` is set as well. A progress handler stops the query at its time limit (30 s by default,
1 h at most); rows are capped by `limits::MAX_ROWS`.

Column types come from the declared type (SQLite affinity rules); expressions without one take the
type of the values returned. Blobs are shown as `<binary, N bytes>` until images land (R2.4).

Open: `data.loadSqlite` takes a path, so an MCP agent can open any database file the user's
account can read. Named connections (R1.3) will replace the path, as specification §11 requires.

## Schema discovery is a trait of its own (R1.7)

`SchemaSource` (tables and views with columns, keys and foreign keys, plus a table preview) is
separate from `DataSource`: a CSV file has rows but no schema, and a database has a schema
before any query is written. Each connector implements what it has; PostgreSQL will implement
both. SQLite reads `sqlite_schema` and the table-valued `pragma_table_info` and
`pragma_foreign_key_list` functions, so discovery runs on the same read-only, time-limited
connection as queries. A foreign key that names no target column is resolved to the target's
primary key. Nullability follows SQLite exactly: a primary-key column accepts NULL unless it says
NOT NULL, except a single `INTEGER PRIMARY KEY` (the rowid). Previews only accept a name that
the schema lists, and quote it, so a table name can never inject SQL.

## Diagnostics: `check` collects, `validate` guards (R2.14)

`Report::validate` stays the guard that rendering relies on: it stops at the first structural
problem. `Report::check` collects every finding as a `Diagnostic` (severity, location such as
`detail.elements[2]` or `groups[0].footer`, message), including warnings and, when the data's
columns are known, unknown fields and totals over text. `report.validate` returns them with
counts and whether the data was checked, so an agent can fix a definition before rendering it.
A definition that does not parse is a diagnostic at location `file`, not a command error.

## PostgreSQL: read-only transaction, cursor, text values (R1.2)

The `postgres` crate (pure Rust, synchronous) keeps our crates free of `unsafe` and of a system
libpq. Every query runs in a `READ ONLY` transaction that is rolled back, with `statement_timeout`
and `lock_timeout` set locally. The query must prepare as a single statement (the extended
protocol refuses several), which also gives the column types; it is then read through
`DECLARE … NO SCROLL CURSOR`, which accepts only `SELECT`/`VALUES`, fetched 1,000 rows at a time
up to the row cap, so a large result is never loaded whole. Rows come back in PostgreSQL's text
format and are converted by column type, which covers `numeric` without a decimal dependency;
values that don't fit (`NaN`, `Infinity`) stay text. Dates stay text until R2.1 formats them.

Schema discovery reads `pg_class`, `pg_attribute` and `pg_constraint` and lists only relations
the user may `SELECT` (`has_table_privilege`). Names in `public` are bare, others are
`schema.table`. A foreign key to a table the user cannot read is kept, its target shown only by
number. Previews accept only a listed name, quoted.

The connection string is never written to errors, logs or `Debug` output; a password left out of
it is read from `PGPASSWORD`, as libpq does. Only plain connections are supported for now (no
TLS); that, `.pgpass`, and named connections (R1.3) are follow-ups. The live tests run against a
scratch database named by `REPORTCRAFT_TEST_POSTGRES` and are skipped without it.

The engine has one set of commands per database kind (`data.loadSqlite`, `data.loadPostgres`…)
sharing the describe/preview/limit helpers. When named connections land (R1.3), they should
collapse into kind-independent commands taking a connection name.

## Formats: per element, locales as a typed table (R2.1)

An element may carry a `format` (`number`, `currency`, `percent`, `date`); the report carries a
`locale` tag. Both fields are optional with defaults (`None`, `en`), so files written before them
still read the same and the format version stays 1. Locales are a table of conventions (decimal
and grouping separators, the space before `%` or a trailing currency symbol, the date pattern),
matched by full tag then language; an unknown tag writes as `en` and `report.validate` warns.
French uses a narrow no-break space to group thousands and a no-break space before units.

Values a format doesn't fit are written as they are (text in a number format, a number in a
date format), never dropped or zeroed; `-0.004` at two decimals is `0.00`, not `-0.00`. A count
ignores a currency or date format on its field. Date patterns are `strftime`-style through
chrono and are checked before use (an unknown directive is a validation error, never a panic);
dates are read from ISO 8601, SQL (`2026-10-01 14:05:00+02`) and RFC 3339 text. Day and month
names are English for now (follow-up under R2.1).

The sample's amounts now have two decimals and grouping; the golden snapshots changed only in
those texts (reviewed), and a French reference report was added.

## Typed parameters, resolved once per run (R1.8)

A parameter declares a `type` (text, integer, number, boolean, date, date_range), optional
`choices`, `multiple` and `required`. Older files, with only a name, label and default, read as
text parameters. Supplied values arrive as text (a JSON number or boolean is accepted and
converted; an array gives several values), are parsed by type, compared with the choices as
parsed values (`010` equals `10`), and resolved with the defaults once, before rendering; any
problem stops the render with every message. A multiple parameter's default lists values
separated by `;`. Values for undeclared names still pass through as text, as before, and
`report.validate` warns about them. Resolved values keep their type for the query binding of
R1.5; the report shows their canonical text through the element's format (dates ISO, ranges
`from..to`, lists joined by `, `).

## Report parameters in queries: `:name`, bound (R1.5)

Queries name report parameters as `:name`, a date range as `:name.from` / `:name.to`, and a list
parameter expands in place into one placeholder per value (`IN (:branches)`), the same in every
database. `reportcraft_data::bind` scans the SQL, skipping string literals (including `E'…'`
with backslashes), quoted identifiers, `--` and nested `/* */` comments, dollar-quoted strings
and `::` casts, and writes only placeholders into the text: `?1`… for SQLite (bound natively),
`(($1::text)::bigint)`… for PostgreSQL, where every value travels as text and is cast to the
parameter's type, so the server never has to guess a type and `numeric` stays exact. A name the
report doesn't declare is an error, so a typo never silently becomes NULL; a declared parameter
without a value binds NULL. Parameters are resolved only when the query uses them. Both
connectors check that the number of placeholders matches the number of values. PostgreSQL
accepts bound values in `DECLARE … CURSOR` through the extended protocol, so results are still
read in batches.

## Group pagination and the "fresh page" rule (R2.3)

Groups gain `keep_together`, `repeat_header` and `page_break_before` (optional, default off, so
existing files and snapshots are unchanged). The layout tracks the open groups: a new page prints
the page header, then the headers of the open groups that repeat, outermost first, reading the
group's first row. `keep_together` computes the group's height (header, nested groups or rows,
footer) and moves it to a new page only if it doesn't fit in the room left but does fit on a new
page below the page and repeated headers; a taller group flows from where it is. A page that holds
only page and repeated headers is "fresh": no break is taken there (a break would not give more
room), which rules out blank pages and endless breaks, and `page_break_before` on a group that
already starts a page does nothing. A group's footer that flows to a new page shows the group's
repeated header above it. `report.validate` warns when repeated headers leave no room for a row.

## Paper, orientation and columns (R2.9)

`PageSetup` gains `paper` (A3, A4, A5, Letter, Legal), `orientation`, `columns` and
`column_gap_mm`, all optional. `width_mm`/`height_mm` stay the source of truth when no paper is
named, so earlier files read the same; landscape puts the long side across whichever way the size
was given. `PageSetup::size()` is the one place that answers "how big is the page".

With several columns, group headers and footers and the detail band go into the current column
(their elements are positioned within the column's width) and flow down, then into the next
column below the page header, repeating the open groups' headers, then onto a new page. Report and
page headers and footers span the page; the report footer is placed below the lowest column. With
one column the layout is unchanged, and the existing snapshots did not move. `report.validate`
checks column bands against the column width.

## Lines and rectangles; colours are a closed format (R2.4)

Two content kinds draw shapes over the element's box: `line` (top-left to bottom-right corner,
so a zero height is a horizontal rule) and `rect` (border and/or fill). Laid-out items carry an
optional `shape` instead of text; the HTML writer draws a page's shapes as one SVG layer under
the text. Colours are accepted only as `#rgb` or `#rrggbb`, checked when the report is validated
and again by the writer, which draws nothing for anything else: a colour string can never carry
markup or CSS into an output. Images are a follow-up (source, size limits, attribution).

## Labels are columns filled across (R2.12)

Labels reuse the columns of R2.9 with `column_order: across`: detail bands fill a row of
columns left to right, the row's height is its tallest label, then the next row starts; a row
that doesn't fit starts a new page. Group headers and footers close a partly filled row and sit
in the first column, so labels of different groups never share a row. Report headers and
footers still span the page. With `down` (the default) or one column, nothing changes.

## Cancellation is a shared flag, dropped when the command ends (R1.9)

`reportcraft_core::Cancel` is an atomic flag shared by its clones, plus an optional deadline
(native only: the web build has no clock). Long work checks it between steps: SQLite through its
progress handler, PostgreSQL through a watcher thread that sends the server a cancel request
(the statement is stopped there, not just abandoned), the layout every 256 rows. A raised flag
gives `Error::Cancelled`; a passed deadline gives `Error::TooLarge` like the existing time
limits. The session lowers the flag when a command ends, so a stop request never outlives the
command it was meant for. The MCP server reads messages on a separate thread so a
`notifications/cancelled` reaches the running request; one for a request not yet started makes
the server skip it without a reply, as the protocol allows. Memory is limited by an estimate of
the rows' size (enum plus text), cheap to keep up to date and close enough to stop a runaway
query long before the process runs out of memory.

## Barcodes: own encoders, module grids in the document (R2.11)

The encoders (`reportcraft-barcode`, L0, no dependencies) are written from the public standards
rather than pulled in: they are small, have no unsafe code or transitive dependencies, and the
model can check a fixed value when a report is validated. An encoder returns a grid of modules;
the layout puts it in the document as a `barcode` shape with its quiet zone, and each writer draws
it at the box's size (HTML: one SVG path in module units on a white ground). Linear codes stretch
to the box width and print their text below; QR codes are square, as large as the box allows.
A value that can't be encoded (a letter in an EAN-13, an emoji in Code 128) is printed as text,
so a wrong label is visible rather than blank. Tests decode the output with zxing-cpp outside the
repository; inside, the encoders are checked against the standards' worked examples and
capacities.

## The query cache is opt-in, per load, for the session (R1.10)

A load reuses cached rows only when it says how old they may be (`cacheSeconds`); otherwise it
reads the database, as before. The key is everything that shapes the result: source kind, target
(the file, or `user@host:port/database`), the SQL as sent, the bound values, the row and memory
limits. The cache lives in the session (at most 8 results, 1 GiB), so it helps the designer, the
MCP server and the future control channel, not a one-shot CLI run; a disk cache waits for a
settings location (R1.3). Rows are held in an `Arc`, shared by the cache and the session, which
also stops `Session::execute` from copying the loaded rows before every command. Every load says
when its rows were read (`refreshedAt`, local time), and a report can print it.

## Subreports are laid out apart, then placed block by block (R2.5)

A subreport element holds a whole report definition and the name of a dataset. The session keeps
named datasets beside the report's own rows, loaded by the same commands with `"dataset"`; links
pick the dataset rows matching the current row, through an index built once per element (values
compared as text, so an integer id matches the same id read as text; nulls match nothing). The
subreport is laid out by the same writer on an endless page as wide as its element, without page
header or footer, recording where each band (and each block of a nested subreport) starts. The
parent cuts it at those marks and places the blocks below the band's top, moving to the next
column or page between blocks; a block never splits, and one taller than a page is placed anyway
on a fresh page, as bands are. The band then ends below the subreport, keeping the room it had
under the element. Page numbers stay in the main report (a subreport has no pages of its own),
and nesting is capped at three reports. With no matching rows a subreport is hidden by default,
or shows its headers and footers with zero totals (`hide_when_empty: false`).

## PDF goes through craft-pdf; fonts are found, not shipped (R2.6)

The PDF writer is its own crate (L4) over `layout::Document`, like the HTML writer, and uses the
suite's `craft-pdf` (`write`, `write-simple-text`) with the patched Krilla, both pinned to the
craft-libs revision GridCraft uses (Etienne approved the git dependency and the root
`[patch.crates-io]`). Text is shaped with rustybuzz (the version Krilla uses, so one copy) and drawn
as glyph runs: the measured width gives right and centre alignment, and text wider than its box
starts at the left edge and is clipped to the box, as the HTML writer's CSS does. The baseline
follows the CSS line box (line height 1.2, half the leading above the ascender), so the PDF and the
HTML pages match. No font lives in the repository: `REPORTCRAFT_FONT`/`REPORTCRAFT_FONT_BOLD`, then
a craft-fonts checkout (`CRAFT_FONTS_DIR`, Inter), then common system fonts; none found is an error
that says what to set. A character the font lacks becomes `.notdef` and a warning, not a silent
gap. The engine returns the PDF base64-encoded in the command result so the MCP server and the CLI
share one path. CI installs `fonts-liberation` for the writer's tests, which fail on CI (but skip
locally) when no font is found.

## Formulas: an Excel subset, evaluated by our own crate (R2.2)

Formulas use the syntax report authors already know from spreadsheets (decided 2026-11-01):
`IF([qty] > 10, [price] * 0.9, [price])`, fields in brackets, `,` or `;` between arguments. The
parser and evaluator are a crate of their own (`reportcraft-expr`, L0, chrono only) so the model
can check formulas and the layout can run them, on every target including wasm. No existing crate
fit: spreadsheet engines are tied to a cell grid, and general expression crates have other syntax
and no Excel semantics. Formulas can only compute: there is no function that reads a file, the
network or the clock beyond the render's own time, so a shared report never runs code.

Semantics follow Excel where it has an answer, so a formula behaves as its author expects:
operator precedence (negation before `^`), blanks as 0 or `""`, case-insensitive text comparison,
errors as values that flow until `IFERROR`, `ROUND` on the 15 significant digits Excel keeps
(`ROUND(2.675, 2)` is 2.68), numbers shown with 15 digits (`0.1 + 0.2` is `0.3`), `DATE` adding
1900 to years below 1900. Where Excel's answer comes from its cell grid we chose the report's
equivalent: a field given directly to `SUM`, `COUNT`, `AVERAGE`, `MIN`, `MAX`, `COUNTA`, `SUMIF`,
`COUNTIF` or `AVERAGEIF` is a range — that field over the rows the band summarises (the group in a
group band and in the detail band, every row in the report header and footer). Dates are values
of their own rather than serial numbers: a date plus days is a date, a date minus a date is days,
and text holding an ISO date (as data sources write them) is read as a date where one is
expected; `YEAR(45000)` is `#VALUE!`. An empty date stays empty through date functions, so a
missing return date prints nothing rather than 1900.

Named formulas are computed once per row before sorting and grouping, as extra columns, so they
work wherever a field does (groups, sort, totals, subreport links); they are evaluated in
dependency order and a circle is a validation error. They may not summarise rows (that would make
a row's value depend on its group, which depends on the sort); totals go in formula elements.
Aggregates that don't read the current row are remembered for the rows being summarised, so a
share-of-group in the detail band costs one sum per group, not one per row. Hostile input is
bounded: formulas are at most 8 KiB and 100 levels deep (checked while parsing, so evaluation
recursion is bounded too), 255 arguments, 256 named formulas, 32 KiB of text per result.

## Office export: lines of text, typed cells, tab stops (R2.7)

Both writers read the laid-out `Document`, as the HTML and PDF writers do, so an export shows
what the report shows. Items whose tops line up (within 0.25 mm) form a line. The layout now
keeps, next to an item's text, the typed value behind it and its format (`Item::cell`), and the
document records its margin, so a writer can keep a number a number.

A spreadsheet is for working on the figures, not for looking at pages: XLSX is written from a
continuous layout (`render_continuous`: one endless page as wide as the body, page header once,
no page footer or page breaks, one column), a row per line and a column per distinct left edge,
column widths from the gaps between edges. Numbers, booleans and dates are typed cells with the
element format turned into an Excel code (`#,##0.00 "€"`, `0.0%`, a `strftime` pattern as
`dd/mm/yyyy`); a pattern Excel can't express is written as text. Excel shows the codes with the
reader's own separators.

A Word document is for sending or editing a page: DOCX keeps the pages and margins. Absolute
frames would be faithful but awkward to edit, so each line is a paragraph with an exact height
that puts its text where the layout did (the text sits at the bottom of the line, so the
paragraph ends one text line below the items' top), and each item sits on a tab stop at its
position: left, right or centred as it is aligned. Rendered by WordCraft, the sample matches the
PDF to a couple of pixels. The bottom margin keeps 3 mm of slack so rounding never pushes a
footer onto a page of its own. Both files carry a fixed creation date, so the same report gives
the same bytes. Lines, boxes and barcodes are left out with a warning for now. The office crate
is a native-only dependency of the engine, like the database sources.

## Conditions are formulas on bands and elements (R2.10)

A band or element shows on a formula (`visible_if`) and a band can start a new page on one
(`page_break_before_if`), read like a formula element in the same band (current row, the rows it
summarises, parameters). There is no separate condition language: one formula engine, one set of
diagnostics. A condition holds for TRUE or a non-zero number; FALSE, 0 and a blank don't. An
error never hides anything or breaks a page, so a mistake in a condition shows up as data on the
page rather than as data silently missing.

Hiding is about printing, not about data: a hidden detail row still counts in its group's totals
(filtering rows belongs in the query or, later, a report filter). A hidden band takes no room; a
hidden element keeps its room, so the band's other elements don't move. Page footers keep their
room when hidden so the body ends at the same height on every page; a hidden page header frees its
room. `keep_together` still estimates a group's height from every band, so a group with hidden
bands may move to a new page a little early (noted in the roadmap).

## Printing goes through craft-print, and only on confirmation (R2.8)

Printing reuses the suite's spooler crate (`craft-print`, same craft-libs revision as
`craft-pdf`): the report is rendered to PDF exactly as an export, then handed to CUPS (`lp`) or
the Windows print API, with copies, collation, duplex, grayscale and scaling as job options.
Page ranges (`1-3,5`, `8-`) pick pages of the laid-out document, so a printed page carries the
same "page N / M" as in the full report.

A print job leaves the application and uses paper, so `file.print` only sends it with
`"confirm": true`; without it, the command renders the document and answers with the job it
would send (printer, pages, copies, the `lp` command line). That keeps three promises at once:
programmatic calls never open dialogs, the empty-parameters crash test never prints, and an MCP
agent proposes a print that a person confirms (§11, §18.3). The CLI's `print` confirms unless
`--dry-run` is given, because the person typed it. Copies are capped at 99. The printer list and
job come from the system, so printing is native-only; the browser build answers that it isn't
available.

## Saved connections: definitions in settings, passwords in the keychain (R1.3)

A connection is saved by name with what it takes to reach the database, nothing more: a SQLite
file, or a PostgreSQL host, port, user and database. Definitions live in the suite's settings
file through `craft-settings` (the folder the suite's rules choose, `REPORTCRAFT_CONFIG_DIR` to
override), so they are hand-editable TOML that survives updates. Passwords go to the system
keychain through `keyring` (MIT/Apache-2.0, crates.io; craft-libs has no keychain crate yet). Its
default backends are pure Rust (Secret Service over zbus, the macOS Keychain, the Windows
Credential Manager), so CI needs no system library. The keychain sits behind a small
`SecretStore` trait so tests use memory and never touch a user's keychain. The PostgreSQL
connection string is built only to hand to the driver; results, errors and the query log show
`user@host:port/database`.

The CLI reads a password from stdin (`--password-stdin`), never from its arguments, which end up
in shell history and process lists. Saving and deleting change the user's settings and keychain,
so, like printing, they do nothing without `"confirm": true` and otherwise say what they would do:
an MCP agent can propose a connection that a person confirms. Letting an MCP session use *only*
saved connections (no raw paths or connection strings) is the next step for §11 and is listed in
the roadmap. The connections crate is native-only (L3); the browser build lists none.

## An MCP client is an agent: saved connections only, no confirmed actions (§11)

Every command declares what it touches (`Access`: reads, edits the session, or acts outside the
application), and the MCP tool list turns that into MCP annotations, so a client can tell a
preview from a print. The session carries a `Policy`. A person (CLI, the future app) may do
everything; the MCP server narrows the session to `Policy::AGENT` for as long as it serves:
databases only through saved connections (a raw file path or connection string is refused with
a message that says how to save one), and external actions (`file.print`,
`data.saveConnection`, `data.deleteConnection`) can be called without `confirm`, which answers
what would happen, but `"confirm": true` is refused. That is the propose → approve flow of §18.3
without a dialog in the engine: the agent proposes, a person confirms in the app or at the
command line. Whoever starts the server can widen it (`mcp --allow-paths --allow-actions`), a
choice made by a person rather than by the agent. Inline data (`data.loadCsv` with the CSV text,
`report.load` with JSON) stays allowed: it reaches nothing the agent didn't already have.

## Day and month names come from chrono's locale data (R2.1)

`%A`, `%B`, `%a`, `%b` and `%p` follow the report's language through chrono's `unstable-locales`
feature (the `pure-rust-locales` tables, pure Rust and wasm-safe) rather than names of our own:
one source of truth, maintained upstream, already used by the date code. Each entry of the
locale table names its chrono locale (`fr` → `fr_FR`, `de-CH` → `de_CH`); an unknown locale
writes English, as it writes `en` numbers. The names are what the locale data says, lower-case
in French or Italian, for instance. Excel exports keep their own codes (`dddd`, `mmmm`), which
Excel shows in the reader's language.

## Formulas write text in the report's language; format codes stay English (R2.2)

When a formula turns a number into text (`&`, `CONCAT`, `LEFT`…), it uses the report's decimal
separator, as Excel's General format does in a French workbook; `VALUE` reads either separator.
`TEXT(value, code)` takes an Excel format code. Codes are written as in English Excel (`.` for
decimals, `,` for thousands) whatever the report's language, so a report is portable and the
code means the same everywhere; the result uses the locale's separators and day and month names.
The evaluator gets these from the caller (`Env::locale`), like everything else it reads, so the
expression crate stays free of the report model. A code it doesn't support gives `#VALUE!`
rather than a guess.

## Colours are style values; conditional colours are formulas that return one (R2.4)

A text element's style can carry a colour and a background (`#rgb` or `#rrggbb`, saved as
`#rrggbb`), held as a small `Color` value so the style stays a plain copyable value and every
writer gets validated bytes, never a string it would have to trust. A conditional colour is not a
new rule language: `color_formula` and `background_formula` are formulas whose result, when it is
a colour, replaces the style's for that row (`IF([amount] < 0, "#c00", "")`), so one formula
engine covers visibility, page breaks and colours. Anything else (blank, text, an error) keeps
the style's colour. HTML writes CSS, PDF fills the glyphs and a rectangle behind the box, DOCX
sets the run's colour and shading, XLSX the cell's font and fill. Golden snapshots print colours
only when set, so reports without them keep their snapshots.

## A band kept with the next one asks for the room of what follows (R2.3)

`keep_with_next` on a band means it is never printed alone at the bottom of a page or column.
On a group header, "what follows" is known without laying anything out twice: the headers of
the inner groups and one detail row. Before placing the header, the layout asks for that much
room below it; when it isn't there, the header moves to the next column or page. A fresh page
never moves on, so a header whose followers can't fit anywhere is still placed rather than
looping. It is opt-in so existing reports keep their pages; the designer can turn it on by
default for new group headers.

## DOCX draws shapes as pictures anchored behind the text (R2.7)

Word has no "rectangle at this exact place behind the text" that docx-rs can write, but it can
anchor a picture to the page. Backgrounds, line segments, box fills and borders are 1 × 1 PNGs of
their colour stretched to their box; barcodes are black-and-white PNGs of the module grid with
their quiet zone, several pixels per module so a viewer's smoothing never blurs a module (the
EAN-13 and QR code of the test report decode with zxing-cpp from WordCraft's rendering). The PNGs
are written by a small encoder of our own (stored deflate blocks, CRC-32 and Adler-32), so no
image crate comes in. docx-rs writes every anchor in front of the text with square wrapping and
the same drawing id; the writer adjusts the built XML before packing it (behind the text, no
wrapping, one id per drawing) and names the pictures itself, so the same report still gives the
same bytes. Diagonal lines are not drawn yet.

## The designer starts small, on the suite's egui fork, and only runs commands (R3.1)

The designer is `reportcraft-ui-egui` (L6) and the `reportcraft` app (L7), on egui and eframe 0.37
from the suite's fork at the revisions craft-libs pins (root `[patch.crates-io]`, as GridCraft
does), rendering through wgpu. The first version is deliberately small so the engine can be
tried by hand: a banded canvas drawn to scale from the report model, a live preview drawn from the
same `Document` the writers read, menus for files, data and exports, and a read-only properties
panel. Every action is an engine command (`Session::execute`), so the designer can't do anything
the CLI and MCP can't, and the preview renders again only after a command that edits the
session (`Access::Edit`). File dialogs are behind a `Dialogs` trait: the app answers with rfd, tests
with fixed paths, so the designer is tested headless by running egui frames without a window.
craft-ui's controls and craft-menu come next, with editing (R3.2); starting with plain egui kept
this first step independent of craft-libs' UI revision, which is newer in GridCraft than ours.
`--screenshot` lets agents and CI look at the window without a screen grabber.

## Designer edits are engine commands, applied when an edit is finished (R3.2)

Editing a report adds five commands, so the CLI and MCP agents can do what the designer does:
`report.addElement`, `report.setElement` (with `move_to` another band), `report.removeElement`,
`report.setBand` and `report.setProperties`. Bands are named as diagnostics name them (`detail`,
`groups[0].header`). Each command edits a copy of the report and keeps it only if it still
validates, so a refused edit (an element past the page, a formula that doesn't parse) leaves the
report as it was and the status line says why. The canvas doesn't edit anything itself: it reports
events (select, move, resize, drop) that the designer turns into commands. A drag is measured
from where the button was pressed, so the element follows the pointer, and lands on a 0.5 mm
grid. The properties panel edits a working copy and sends one command when an edit is finished
(focus left, a drag stopped, a choice made), not one per frame, so the preview renders once per
edit. Undo (R3.4) will record these commands.

## Undo keeps whole report definitions, recorded by the engine (R3.4)

Undo lives in the session, not in the designer: after any command that succeeded and left the
report different, the engine keeps the report as it was (at most 100, the oldest dropped), and
`edit.undo` / `edit.redo` swap it back. A report definition is small (a few kilobytes even with
subreports), so whole copies are simpler and safer than inverse operations, and every way of
changing the report (designer, CLI `run`, MCP) is undoable alike, including `report.load`. Data
loads, read-only commands and refused edits don't touch the history, and a new edit clears what
was undone. Text boxes keep their own undo while they have the keyboard.

## The control channel drives the open window as an agent would (R3.3)

`reportcraft --control PORT` listens on 127.0.0.1 only, for JSON lines, as GridCraft's does. A
connection must first give a token the app writes, at start, to a file only its user can read
(`$XDG_RUNTIME_DIR/reportcraft-control-PORT.token`, removed on exit), so another account on the
machine, or a web page aiming at localhost, can't drive the window. Requests are engine command
ids plus `ui.state`, `ui.screenshot` and `app.quit`; the window answers them between two frames,
so a request sees exactly what the person sees and the canvas and preview follow. Commands run
under the agent policy (saved connections only, no confirmed external action): whoever holds the
token is a program, not the person at the screen. The server is plain std (a thread per
connection, at most eight, lines up to 16 MiB, a reply within ten minutes); the network code is in
the app (L7), the token helpers in the engine so `reportcraft-cli send` shares them.

## The designer speaks the suite's languages through craft-i18n (R3.5)

Interface text goes through craft-libs' craft-i18n, wired as its recipe says: the catalogs are an
optional build input (`CRAFT_I18N_DIR`, a checkout of storytold/craft-i18n), embedded by the
designer's `build.rs` with `include_str!`; without it the interface is English, and
`CRAFT_I18N_REQUIRED=1` makes a missing checkout an error for releases. Every visible string is
English source text in `tl!` (or `tf!` with `{placeholders}`), so shared words (File, Undo, Paper…)
come translated from craft-i18n's common catalog. ReportCraft's own strings have no catalog in
craft-i18n yet, so they live in this repository (`i18n/fr.tsv`, craft-i18n's format and clean-room
rule) until they are synced there; the build takes craft-i18n's `reportcraft.tsv` when it exists,
else ours, and only offers a language that has one. The language is the system's, or `--lang`, or
View ▸ Language; offered languages are left-to-right ones for now. Engine messages stay English.

## Charts are laid out as shapes and text, not by a chart library (R5.1)

A chart element totals a field by category over the rows its band summarises, as formulas and
totals do, and the layout draws it with the primitives every writer already has: rectangles for
bars and markers, segments for lines and axes, text for titles, values and legends, plus one new
primitive, a filled polygon, for pie wedges (arcs in steps of at most 5 degrees). So HTML (SVG),
PDF (paths), DOCX (pictures anchored behind the text; the wedge is a transparent PNG) and the
designer's preview (one mesh per wedge, so no seams) draw charts without knowing what a chart is,
and golden snapshots check their geometry. Categories keep the order they first appear in (the
report's sort), at most 60. Colours come from a fixed palette of eight with contrast on white.
Several series, axes with ticks and native Excel charts are next steps.

## Database connections stay pure Rust; PostgreSQL is encrypted with rustls (R1.2, R1.4)

Etienne decided on 2026-11-11: connections in pure Rust for now. So no ODBC yet (it needs the
system's driver manager and each vendor's C driver; Advantage has no other route, so it waits),
SQL Server will come through a Rust driver (tiberius), and TLS goes through rustls with the ring
provider (no system library; ring compiles a little C and assembly with the project, as the
bundled SQLite does). PostgreSQL follows libpq's `sslmode`, so connection strings mean what
people expect: `prefer` (the default) and `require` encrypt without checking the certificate, as
libpq does; `verify-full` (and `verify-ca`, held to the same standard) checks it against the
system's certificate authorities (`rustls-native-certs`, which honours `SSL_CERT_FILE`) and the
server's name; `disable` sends nothing encrypted. The driver only knows `disable`, `prefer` and
`require`, so the strict modes are turned into `require` plus verification before parsing. A
failed handshake says which check failed and what each mode does. Saved connections keep an
`sslmode`; the designer offers it as a plain choice.

## SQL Server through tiberius, read-only by three locks (R1.4)

Following the pure-Rust decision above, SQL Server is read with tiberius (TDS 7.3, rustls, MIT or
Apache-2.0), not ODBC. Its API is async; the source runs its own single-threaded tokio runtime for
each call, so it stays synchronous like the other sources and the engine knows nothing of async.
The connection string is ADO.NET's, the form SQL Server people already have; a password left out of
it may come from `SQLCMDPASSWORD`, as `sqlcmd` does, so it needn't be on a command line. Saved
connections use the same `sslmode` words as PostgreSQL: `disable` sends only the login encrypted
(SQL Server always encrypts it), `prefer` and `require` encrypt without checking the certificate
(`TrustServerCertificate=true`, as most servers have a self-signed one), `verify-full` checks it.

SQL Server has no read-only transaction, so read-only rests on three locks: the query text is
scanned first (one statement, starting with `SELECT` or `WITH`, comments, strings and `[names]`
skipped, none of the words that write, run code, change the session or end the transaction), it
then runs inside a transaction that is always rolled back (with `XACT_ABORT` and a lock wait limit),
and the documentation asks for a login with only `db_datareader`, the real guarantee. The scan
refuses some harmless queries (a column named `Insert` must be written `[Insert]`); that is the
price of refusing before connecting. Parameters are bound as `@P1`, `@P2`… (`:name` in the
report's query, as elsewhere), each with its own type. The time limit and Stop send a TDS
attention, which stops the statement on the server. Values follow the column types; dates and
times come as ISO text, as PostgreSQL's and SQLite's do, so formats and formulas treat them alike.
In the CLI, the database options became one table of kinds (`--sqlite`, `--postgres`,
`--sqlserver`, `--connection`), so a new database adds a row, not a branch in every match.

## A report names its connection, never where it points (R1.3)

A report's `data` holds the name of a saved connection and the queries run on it, nothing else:
the host, file, user and password stay in each computer's settings and keychain, so a report can
be shared without its credentials (specification §12) and each person runs it through their own
connection saved under the same name, with their own database rights. A name that isn't saved
here is a warning in `report.validate`, and loading says how to save it, rather than an error when
the file is opened. Subreport datasets get their queries in the main report's `data.datasets`, on
the same connection, so one name covers the whole report; a subreport's own `data` is ignored with
a warning. Loading is all or nothing: if a dataset query fails, the rows already loaded stay as
they were. Options such as the time limit stay with the run (CLI flags, command parameters), not
the report, for now. The CLI loads a report's data only when no data option was given, so
`--data` and `--query` keep overriding it, and `validate` reports a failed load instead of
stopping. The field is optional, so older files read unchanged and `format_version` stays 1.

## Pictures in reports (R2.4)

An image element holds a PNG or JPEG picture, embedded in the report file as base64 or read from
a field of the current row. Only these two formats, because every writer takes them without a
conversion we would have to trust: Krilla embeds JPEG data as is and decodes PNG, rust_xlsxwriter
and the browser read both, and docx-rs writes PNG (so a JPEG is decoded and written again as PNG
for DOCX). A picture is recognised by its first bytes and its size read from its header in
`reportcraft-core` (no decoding, no dependency, wasm-safe), so the layout can keep its proportions
(`fit: contain`, the default, centred) or stretch it (`fit: stretch`) without decoding anything.
Limits: 16 MB and 50 million pixels a picture, checked before any pixel is allocated.

Decoding lives in `reportcraft_layout::pixels`, with the decoders Krilla already uses (`png`,
`zune-jpeg`), so there is one copy of each. Writers that need pixels use it (DOCX for JPEG, the
designer's textures, sampled down to 1,024 pixels a side), and the PDF writer decodes each distinct
picture once before handing it to Krilla: Krilla decodes PNGs while it writes the file, and one
damaged picture would fail the whole export. A picture damaged past its header is left out with a
warning instead. In the layout a picture's bytes sit behind an `Arc`, so a logo in the page header
is one copy however many pages show it, and the PDF writes it once (Krilla also deduplicates by
content; docx-rs does the same for identical media).

A field gives a picture as text: base64, or a `data:image/…;base64,` URI. That is what Odoo's API
and CSV exports give, and it keeps `Value` without a bytes variant (one more arm in every match over
values, for one use). Database blobs that hold a PNG or JPEG (SQLite `BLOB`, PostgreSQL `bytea`,
SQL Server `varbinary`) now come from those sources as `data:` URIs; other blobs keep their former
text (`<binary, N bytes>`, `0x…`). Pictures from a path in a field are left for later: the engine
reads no file itself today, and a path would need the same allowed-paths rules as data files.
The designer embeds a picture through its file dialog; agents and scripts pass base64 to
`report.addElement` or `report.setElement`, so no new command reads files.

## Picture files: a folder per connection, read by the engine (R2.4)

Odoo keeps pictures out of the database: `ir_attachment` holds a relative path (`store_fname`,
such as `3f/3f5a…`) into the database's filestore on the server's disk. A field that names a
picture by a path is read relative to a *picture folder*, never as an absolute path, so a report
stays portable and a shared report can't read arbitrary files. The folder belongs to the saved
connection (each computer says where that database's filestore is, as it says where the database
is) and applies to rows read through it; it can also be set for the session from the CLI or the
engine (`data.setPictureFolder`), which an MCP agent limited to saved connections may not do.
Only files inside the folder are read: the path must be relative with plain components, and the
file is resolved with its links before the check, so a link can't lead out. Each file is capped
like any picture, and a render reads at most 512 MB of distinct files (repeats come from a cache).

The layout reads no file: it asks a `PictureFiles` the caller gives it (`render_with_files`) for
values that aren't base64 or a `data:` URI, and the engine passes one over the folder. So the
layout stays wasm-safe and testable, and other sources of files (a URL, an Odoo API) can come
later behind the same seam.

Odoo 20 stores its product pictures as WebP, so WebP joined PNG and JPEG: Krilla embeds it
(decoded, as it does PNG), and `image-webp`, already in the lock file through Krilla, decodes it
for DOCX and the designer. In the demo database every `image_128` … `image_1920` attachment points
to the same full-size file, so a catalogue of 14 products made a 4.9 MB PDF. Pictures more than
twice as large as their box needs at 300 dpi are therefore sampled down (each pixel the average of
those it covers) and embedded as PNG in PDF and DOCX: that catalogue is 0.6 MB, with no visible
loss. Smaller pictures keep their bytes (a JPEG stays a JPEG).


## Pictures in XLSX: anchored on the row they stand beside (R2.4, R2.7)

A spreadsheet has no page to place a picture on: `rust_xlsxwriter` anchors an image to a cell, with
an offset inside it. Each image or barcode is anchored on the sheet row of the first line of text
on its page that starts within the picture's height (a product photo beside its name), or on a row
of its own when no text stands beside it. The column is the one starting at the picture's left
edge (picture edges count when columns are made, so a picture never sits on a text cell's left).
The row grows to hold the picture, and the text on it is centred vertically, as it reads beside a
logo or a photo. Rows stay one per line of text: making a picture span several rows would change
every report's sheet for the sake of a few, and break the "a row per line" reading of the export.

Images and barcodes share the code DOCX already used (`pictures.rs`): pictures are converted to PNG
and sampled down to their print size once, barcodes drawn black and white with their quiet zone.
Lines, boxes and charts are still left out of XLSX with a warning; cell borders could stand in for
lines and boxes later.

## Odoo QWeb templates: imported and previewed, not written back (R4.1)

Scope decided with the maintainer on 2026-11-14: a QWeb report template is *read* into a
ReportCraft report and previewed on sample rows; ReportCraft doesn't write QWeb back. Writing back
would mean keeping Odoo's HTML, Bootstrap classes and Python expressions intact through every edit,
which a banded model can't hold; the value is in moving a report over, then editing it here.

Only the format is used, as Odoo's documentation describes it (`t-foreach`, `t-as`, `t-if`,
`t-field`, `t-call`…); no Odoo code is read into the converter or copied (Odoo is LGPL). The tests
use templates written for them. Real templates were tried as local input only, never committed.

How the template maps to the model:
- **Data.** A converted report reads one row per line of each document, columns named as the
  template writes them (`o.partner_id.name`, `line.quantity`), the document's columns repeated on
  its lines, `o.id` telling documents apart. Keeping the template's names makes each column
  traceable to the template and lets SQL alias to it (`AS "o.name"`); the formula language already
  reads dotted names (`[o.amount_total]`).
- **Bands.** `t-foreach="docs"` becomes a group on `o.id` with a page break. The first loop in a
  document's table (whatever it loops over: `o.invoice_line_ids`, `lines_to_report`, a
  `t-set` name) is the detail band, since the report's data gives the lines; each of its rows is a
  part of the detail band, under its conditions. What comes before goes in the group header, what
  follows in the group footer. Other loops are shown once, with a warning.
- **Layout.** HTML is a flow; the model is positioned. Blocks (`p`, `div`, headings) are lines
  stacked down the band; their inline text and fields are placed left to right with widths
  estimated from the characters (a field last on its line takes the rest of it). A right-aligned
  or centred line with several pieces becomes one formula (`"Total: " & TEXT([o.amount_total],
  "#,##0.00")`) so the alignment holds. Tables get equal columns from the header row (a
  `colspan="99"` row doesn't widen the table). It is a starting point to edit, not a pixel copy.
- **Expressions.** Python isn't evaluated. Field paths, Odoo's formatting helpers around one field,
  dictionary keys (`line['label']`), truth tests, comparisons with a literal and `and`/`or` chains
  convert; a `t-elif`/`t-else` shows when the branches before it don't. Names that aren't a loop's
  variable (`company`, `is_pro_forma`) aren't fields. The rest is listed with its template line and
  shown in place as `{expression}`, so nothing disappears silently.
- **Sample data.** Types are guessed from names (`*_id` text, `quantity`/`price`/`amount` numbers,
  `id` integers), amounts get two decimals, pictures are blank.

The converter is its own crate at L2 (`reportcraft-qweb`, on `roxmltree`, already in the lock file
through the PDF stack): it needs only the model, stays wasm-safe, and caps its input (4 MB,
200,000 nodes, no DTD, nesting 64 deep, `t-call` chains 8 deep, 512 elements per band).

## Page bands read the page's row; Odoo's page layouts become them (R4.1)

Page headers and footers used to have no current row, only parameters. They now read one: the
header the row its page starts with (the row being placed when the page broke, or the group's row
when a group footer moved to the new page), the footer the last row drawn on its page. That is
what banded report users expect from a field placed in a page band, and it is what lets a page
header carry a letterhead per document when each document starts a page. Page bands still
summarise no rows: a page total needs the rows of the page, a later change (roadmap). Existing
reports change only where a page band held a field, which used to print blank.

`web.external_layout` and `web.internal_layout` are wrappers from Odoo's report module, never in
the file being imported. Their documented content (the company's logo, name and address above,
its contacts and page numbers below; for the internal one a line with the date, company and page
numbers) is rebuilt in a layout of our own, not copied from Odoo's templates. The company is read
through the document (`o.company_id.name`…), so a report over documents of several companies
shows each its own, and the columns are added to the sample data like any other. Page numbers
count the whole report, not each document, and the company's layout choices (boxed, colours,
tagline, footer text) aren't followed yet: both are on the roadmap.

## Nested QWeb loops become linked subreports; what lies below a subreport follows it (R4.1)

A QWeb template reads records through records (an invoice's payments, a delivery's lines and each
line's serial numbers); the banded model reads rows. The first loop over a document's records
stays the detail band (R4.1). Another loop over a path of the document or of the line becomes a
subreport over a dataset of its own, named after the path (`o.payment_ids` → `o_payment_ids`),
with the same naming of columns (`pay.amount`), the document's (and line's) columns it reads
repeated on each row, and links on `o.id` (and the line's `id`) so a query can return all the
items at once. A subreport rather than an inner group: the items are other records than the
lines, with their own count, and a group can't hold rows that aren't lines. Its sample rows (two
per document or line, the same ids as the main rows) come with the import, so the preview works
at once. Only one loop per band becomes a subreport, as the model allows one per band, and only
one level deep; the rest is shown once with a warning. A condition every element of a nested row
shares is set on its band, so a hidden row takes no room.

Bands used to draw every element at its place and let a subreport flow over what lay below it.
Now the elements whose top is below the subreport's bottom are drawn after it, at the same
distance under its end (never higher than the band put them), on the next page when they don't
fit: a total under a list of payments stays under it. Elements beside the subreport keep their
place. No golden snapshot had an element below a subreport, so none changed.


## Odoo's title lines group the lines by section (R4.1)

Odoo keeps section and subsection titles (and notes) as lines of the document, with
`display_type` saying which; its templates test it on each line and print the section's total
on the title's row, computed in Python. The banded model has a better word for "a title over the
lines that follow, with their total": a group. When the detail band has content shown on a title
line and hidden on a product's (decided by a small three-valued reading of the conditions: the
line's type alone settles `AND(…, [line.display_type] = "product")`, not `[line.amount] > 0`), the
converter adds a group per section, and per subsection inside it. Its header is a copy of what the
detail shows on a title line, in bold as Odoo prints titles, shown when the group starts with a
title; the detail hides title lines, which would otherwise print twice.

Grouping needs a key that sorts in document order, which a line's own fields don't give (section
ids follow creation, not position). So the data carries `line.section_number` (0 before the first
section, then 1, 2… in the order of the lines, the title included) and `line.subsection_number`
(the same within a section) — a window count in SQL over the move's lines. The layout sorts by
group keys and keeps the query's order inside them, so the lines stay in sequence.

The section's total is Python (`section_subtotal`, `_get_section_totals(…)`): an unconverted
expression, or the `t-set` name of one, that speaks of a section's total becomes
`SUM([line.price_subtotal])`, which a group header computes over its group's rows; a warning says
so, since a company whose prices include taxes prints `price_total` there.

Converting Odoo 20's invoice also needed: `t-set` names for conditions (`is_section`), `in (…)`
and parentheses, flags (`collapse_prices`, `is_…`) read by their value and not by being blank,
conditions on the `t` elements around cells (alternatives of a `t-if`/`t-elif` chain start at
the same column), the PDF branch of `t-attf-class` (Odoo hides screen-only cells with
`… if report_type == 'html' else 'd-none'`), a computed `colspan` filling the row, and two spans
of a cell under different conditions laid over each other rather than joined. Sample rows take,
for a field compared with a text, that text, so the preview opens the branch the template
expects (`o.document_tax_mode == 'tax_excluded'`).

## A line's records joined into a text read one column (R4.1)

Templates show a line's taxes, lots or tags as one text: `', '.join(line.tax_ids.mapped('name'))`
or a generator over the records. Repeating them as a subreport would draw a list where the
template draws a phrase. The converter reads such an expression (directly, or through the `t-set`
that names it) as one text column named after the records and the field
(`line.tax_ids.tax_label`), consistent with the other columns, and notes the separator so the
query can fill it (`string_agg(t.name, ', ')` in a subquery, aliased to that name). The column is
text even when its name speaks of taxes or amounts. A filter after `if` is dropped: in Odoo's
templates it leaves out blanks, which `string_agg` leaves out too; other filters, and values that
aren't a plain field (`str(t.amount)`), aren't read yet.

## Groups that number their own pages (R4.1)

Odoo renders each document of a batch on its own and joins the PDFs, so an invoice of two pages
prints "1 / 2" and "2 / 2" whatever comes before it. A banded report lays every document out in
one run, so "page N / M" counted the whole run. A group now says it numbers its own pages
(`restart_page_numbers`, off by default, so existing reports don't change): the page a group
starts on is its page 1, and M is the number of pages up to the next group that starts the count
again. With `page_break_before` (as the documents of an imported template have) no page is
shared; without it, a page two groups share counts for the one that starts on it — the page footer
there already reads the new group's row. Crystal-style "reset page number after group" with a
separate "total pages of the group" was considered: one flag covers both, since a per-group
number with a whole-report total reads wrong. Imported QWeb templates set it on the document
group, as Odoo prints them.

## Calculations on fields become formulas (R4.1)

Templates compute in Python where a banded report computes in formulas: `line.price_unit * 1.2`,
`o.amount_total - o.amount_residual`, `move.quantity != move.product_uom_qty`. The converter
reads the part of Python that means the same in the formula language — fields of the documents
and lines, numbers, `+ - * / **`, parentheses, `round`, `abs`, `min`, `max`, and at most one
comparison (a chained `a < b < c` means something else in a formula) — token by token, and
checks the result parses as a formula before using it. What would change meaning is refused and
shown as written, as before: `%` (modulo in Python, percent in a formula), `//`, text joined with
`+` (a formula joins with `&`), method calls. A calculation reading an amount gets two decimals,
like the amount fields themselves; a field whose root isn't a loop's variable (`company.rate`)
still isn't a column.

## The queries of an imported template come from the database's description of its fields (R4.1)

An imported template reads columns named as it writes them (`o.partner_id.name`,
`line.tax_ids.tax_label`); filling them from Odoo by hand meant writing a join per segment and
knowing which fields are stored, related, translated or many2many. The import now writes those
queries when it is given a saved PostgreSQL connection to the Odoo database
(`file.importQweb` with `connection`, `import-qweb --connection`).

What a field is comes from `ir_model_fields`, the database's own description of its models, never
from Odoo's code (LGPL) or a list kept here: its type, relation, inverse field, relation table,
whether it is stored, the field it relates to, whether it is translated. Reading it is the
engine's work, read-only and model by model: the converter (L2, no I/O) proposes the queries over
what it has been given and lists the models it still needs; the engine reads them and asks again,
each round following the paths one join further (at most 12 rounds and 64 models). The converter
stays testable with a description written by hand, and the description columns whose type changed
between Odoo versions (`translate`, `company_dependent`) are read through `to_jsonb` so the same
query works on Odoo 16 to 20.

How the paths become SQL:
- A many2one field is a `LEFT JOIN` on its model's table, once per field; a record shown alone
  (`t-field="o.partner_id"`) shows its `name`. A related field that isn't stored follows its path
  (`product_id.name` → `product_tmpl_id.name`). A translated field reads the language
  (`coalesce(name ->> 'fr_FR', name ->> 'en_US')`). Pictures are attachments: the query reads the
  attachment's file (`store_fname`), which the connection's picture folder resolves (R2.4).
- Records joined into a text are a subquery with `string_agg` over the one2many or the relation
  table of the many2many, in the records' sequence; the separator is the template's.
- The lines are the document's one2many, in their sequence then id. When the template loops over
  Python (`lines_to_report`), the field is given (`lines`) or, for the few documents whose line
  field is documented (an invoice's `invoice_line_ids`, an order's `order_line`…), taken with a
  warning. An invoice's line field shows only some of the move's lines, by a domain the database
  doesn't describe; that one is known (its `display_type` values), so the tax and payment term
  lines don't print. A company's address fields are its partner's, which the database doesn't say
  either.
- Section numbers (R4.1, sections) are window counts over the document's lines; a subsection's
  restarts at each section, which a second level of the query computes.
- A nested loop's dataset joins its records to the document (and the line) and reads its columns
  the same way; a loop the query can't follow reads no rows, so its subreport stays hidden.
- What can't be read — a field computed in Python and not stored, a company-dependent value, an
  unknown field — reads `NULL` with a warning naming it, so the report still runs. A computed
  label (`display_name`, `tax_label`) reads the record's `name` instead, with a warning.

The documents are those of a required, multiple `ids` parameter (`WHERE d.id IN (:ids)`): Odoo
prints the records chosen in a list, and printing every invoice of a database by default would be
the wrong surprise. Table names follow Odoo's default (`res.partner` → `res_partner`); models that
choose another table, company-dependent values and datetimes in the user's time zone are on the
roadmap. The test reads a database shaped like Odoo's with made-up rows (a live test on a scratch
server); no row of a real database was read or committed.

## Text that grows and bands that shrink (R2.10)

The first import of a real Odoo invoice (R4.1) showed two things a banded layout of fixed boxes
can't do and HTML does without being asked: a line's description of four lines was cut to its first
words, and an invoice's footer, where most blocks are conditional, kept the room of every hidden
block, so the totals went to a page of their own. Both are now options of the report model, as
Crystal Reports and Access have them: `can_grow` on an element, `can_shrink` on a band. They are
off by default, so a report that places its boxes by hand keeps exactly the layout it had.

- A text that can grow is cut into lines at its line breaks, then between words (a word wider than
  the box between letters); it takes as many lines as it needs at the font's line height (1.2
  times its size, as the writers draw text), and never less than its box. Each line becomes an item
  of its own, so the PDF, HTML, DOCX and XLSX writers need no change.
- The band is cut into strips at its elements' top and bottom edges. Below an element that grew,
  everything moves down by its growth (the largest when several end on the same line); in a band
  that can shrink, a strip covered only by elements that aren't drawn (hidden by their condition,
  or a text with nothing to show) closes up. A strip that also holds a drawn element keeps its room,
  so a blank cell next to a filled one changes nothing. This is computed per row, before the band is
  placed, so a page break sees the band's real height.
- Text is measured in the layout (L3), which has no fonts: the widths are Liberation Sans' (Arial's
  metrics, the first font the PDF writer looks for), kept in a table of the Latin-1 characters,
  and a line fills at most 96 % of the box so a slightly wider font isn't cut. Measuring with the
  fonts the PDF embeds would mean the layout depending on the writer's fonts; it is on the roadmap.
- The QWeb importer marks the bands it makes as able to shrink and the text fields that end a line
  (a table cell, a value after its label) as able to grow, which is what the template's HTML does.
  Amounts don't grow: a number cut onto two lines would be misread.

## The external layout's blocks, grid rows and the contact widget (R4.1)

Odoo's invoice doesn't write its title or the customer's address itself: it sets them as blocks
of HTML (`<t t-set="address">…</t>`, `layout_document_title`, `information_block`) and calls
`web.external_layout`, whose own templates place them. The importer used to drop a `t-set` with a
body, so a real invoice came out with no title and no address. Such a block is now kept with the
condition it was set under, shown where its name is written out (`t-out="address"`), and placed
by the external layout as Odoo's documented layout places it: the information block on the left
half, the address in the right five twelfths, then the title as a heading. The layout's templates
aren't in the file and aren't read; their arrangement is written here from what they print.

- **Grid rows**: a `<div class="row">` whose children are columns (`col`, `col-6`, `col-md-5`,
  `offset-1`, `ms-auto`) lays them side by side, each from the row's top, the row as tall as its
  tallest column; plain `col`s share the twelfths left. A row holding a loop or a call is still
  read top to bottom, as those become bands or subreports of their own. Columns are placed once:
  a column hidden by its condition keeps its width, and a blank line in one column closes up only
  if the columns beside it have nothing at that height (the band shrinks by strips across its
  whole width). (Hidden columns now give their width up: see "Boxes side by side" below.)
- **The contact widget** (`t-options='{"widget": "contact", "fields": [...]}'`) writes a partner
  as lines: its name (after its company's, for a contact of a company), street, second street,
  postcode and city, country, then the phone, e-mail, website or VAT number when listed. Each line
  is a column of the query; a blank one takes no room. Countries that write addresses in another
  order are on the roadmap.
- **Words under different conditions on one line** (`<span t-if>Invoice</span><span
  t-elif>Credit Note</span> <span t-field="o.name"/>`) used to become a line each. They are now
  one formula, each word under its own condition (`TRIM(IF(…, "Invoice ", "") & … & [o.name])`),
  which reads as the HTML does; the same holds in a table cell whose pieces mix the cell's
  condition and their own.

## An invoice's tax totals, calls inside tables and names nothing sets (R4.1)

Odoo prints an invoice's totals from `tax_totals`, a dictionary computed in Python (subtotals,
tax groups, the total), through a template called inside the totals table. The first trial on
real invoices printed them only by accident: the call inside the table wasn't followed, and the
block in the company's currency, whose condition wasn't converted, showed in its place.

- **Calls inside a table** are followed like rows of the table, and a call's attributes
  (`tax_totals="o.tax_totals"`) are names in the called template, as QWeb passes them. A
  template that only makes another one primary under a new name (`inherit_id`, no content) is
  that one.
- **The values** are read from what the database stores, written in a table of the converter
  (`KNOWN_VALUES`), as the lines' domain and the company's address already are: the total and
  untaxed amounts are the move's own fields, the subtotal is the untaxed amount ("Untaxed
  Amount", as Odoo prints an invoice with no subtotals of its own), and the tax groups are a
  dataset of the move's tax lines summed by tax group, signed as the invoice shows them. The
  bases come from the tax lines' `tax_base_amount`, in the company's currency; for an invoice in
  another currency that is on the roadmap. The loop over the one subtotal is read once, so the
  tax groups inside it are a nested loop of the document like any other.
- **A name nothing sets** (`print_with_payments`, `proforma`, when the template printed isn't the
  one that sets them) is `None` in Odoo, so a condition on it is false. It is read that way, with
  a warning naming it, unless it is one of the values Odoo gives every report (`docs`, `company`,
  `report_type`…). A loop over something that isn't a path of the document (a Python list) no
  longer makes its variable a record whose keys are columns.
- `.get('key')` reads as a key, `'key' in dict` as the key having a value, and names starting with
  `display_` (but `display_name` and `display_type`) are flags, read by their value.

## Boxes side by side: grid columns give their width up when hidden (R4.1)

Odoo's invoice writes its dates, origin, reference and incoterm as a Bootstrap row of up to eight
plain `col`s, each under its condition. A column whose condition fails isn't in the HTML, and the
browser shares the row among the columns left: four of them take a quarter each. The importer
placed the eight columns once, an eighth each, so a real invoice showed its four values squeezed
into the left half, the reference cut onto five lines. A banded layout of fixed boxes can't do this
by itself, so it is now part of the report model rather than a trick of the importer.

- **The model**: an element may name the box it lies in (`flow`: a row number, the box's left edge
  and width as designed, and whether it grows). The boxes of a band with the same row number are
  a row; its elements are found by their box, so a box needs no list of its own and a report
  without boxes reads and renders as before (no format version change). The row spans its boxes,
  and the room between a box and the one before it (an `offset-1`) goes with the box.
- **The layout**, for each row of data: a box shows when one of its elements does (by its
  condition). When one is hidden, the shown boxes are laid from the row's left edge, each after its
  own room; those that don't grow keep their width (`col-6` moves left, as flex places it), and
  those that grow share what is left in proportion to their widths (equal for plain `col`s),
  never narrower than designed. The elements of a box move and widen with it, keeping where they
  were in it in proportion. This is computed before text that can grow is measured, so a value
  wraps at the width it has on that row. Images, barcodes, charts and cross-tabs move and widen too;
  a subreport keeps its place (its layout is computed once for its width).
- **Hidden, not blank**: a box gives its width up only when its elements are hidden, as a column
  under a false `t-if` is absent from the HTML; a column that is there with blank values keeps its
  room, as in the browser.
- **The importer** gives boxes to the columns of a grid row of two columns or more. A row inside a
  column moves with that column (its elements belong to the outer box), and a row with a column
  pushed right (`ms-auto`) keeps its places, as the room before that column depends on the others;
  both are on the roadmap.
- This is the idea of table cells that give their width to their neighbours in some report
  designers; ReportCraft has no table element, so it is a property of free elements instead, which
  is also what an imported grid row is.

## Table columns that ask for a share of the table (R4.1)

The importer gave a table's columns equal widths. Odoo's invoice has six: the description
squeezed into a sixth wrapped a line's text onto many lines while the amounts had room to spare.
In the browser the table is laid out by its content, and the description's cell says `w-100`
(as wide as the table), so it takes everything the other columns, as wide as their content, leave.

- A cell with Bootstrap's width classes (`w-25`, `w-50`, `w-75`, `w-100`), in any row of the
  table, gives its column that share. The other columns are as wide as their header's text
  (measured like other imported text) and never narrower than 20 mm, the room an amount takes;
  their values aren't known when importing, so they don't count.
- Columns asking for less than the whole table have their share, and the others are widened to
  fill the rest; a `w-100` column takes what the others leave, never less than a quarter of the
  table (the others are narrowed when needed). A table where no cell asks keeps equal columns, so
  the templates that imported well still do.

## Yes/no and text formats (R2.1)

Two formats complete the ones on fields: `boolean` writes a true or false value as words and
`text` writes a value in a case, between a prefix and a suffix.

- **Yes/no** reads a boolean, 1 or 0 (SQLite and CSV booleans) and the text `true`/`false`; any
  other value is written as it is, like the other formats. Without texts of its own it writes the
  report language's words, which are part of each locale's row (`Oui`/`Non`, `Ja`/`Nein`,
  `Sí`/`No`…), so a report in French says `Oui` whatever the reader's interface language. The
  designer shows those words as the hints of its two boxes. A box left empty in the file means
  the default; an empty text given (`"false_text": ""`) shows nothing, for a tick only when true.
- **Text** writes the value as text (numbers as they are, not in the locale's format, since an
  identifier such as `Ref. 1042` shouldn't get a thousands separator), in upper, lower or title
  case (title case as the `PROPER` formula writes it), between a prefix and a suffix. A blank value
  stays blank, without them, so `Tel. ` doesn't show alone.
- Both write words, so XLSX gets the words as text, as the report shows them, rather than a
  typed boolean or number: Excel can't show a boolean under a format of its own, and a number
  with a prefix is a label. Typed booleans in XLSX are on the roadmap.
- `report.check` warns when a yes/no format is on a numeric column (it won't apply), and the
  format's texts are at most 200 characters.

## Page breaks after a band, and rows per page (R2.10)

A band could start a new page (`page_break_before_if`, a group's `page_break_before`) but not end
one, and a detail band couldn't stop after a number of rows, which reports printed on forms and
title pages need.

- **`page_break_after_if`** is a formula, like the break before: when it holds for the band's
  row, the next band placed starts a new page. The request waits for a band that is shown (a
  hidden or empty band passes it on) and is dropped at the end of the report, so a break after the
  last band never adds a blank page. A page that is already new ignores it, so a group starting a
  page of its own after a band that asked for one gives one page, not two. `TRUE` breaks always
  (a report header on a page of its own); a group footer's `[branch] <> "Valence"` gives a page per
  branch but the last.
- **`rows_per_page`** on the detail band: when the page holds that many detail rows, the next one
  starts a new page. Only rows that are shown count; in labels (columns across) labels count. It is
  a whole number from 1 to 100,000, and on another band it is ignored with a warning.
- Page headers and footers never break the page: either set there is ignored with a warning, as the
  break before already was. Both are in `report.setBand` and the designer's band properties (rows
  per page for the detail band only).

## A title page without the page header and footer (R2.9)

A report header that breaks the page after it (R2.10) makes a title page, but the page header and
footer were printed on it too. Page headers and footers now have `hide_on_first_page`: on the
report's first page the page header takes no room (the title starts at the top margin) and the
page footer keeps its room, as a hidden page footer does, so the body ends at the same place on
every page. Page numbers still count the title page ("2 / 3" on the first page after it), as in
a printed document. It applies to the report's first page, not to each group's first page when a
group numbers its own pages. Margins of their own for page one are on the roadmap.

## Running totals (R2.2)

A report often shows, on each line, the total so far: a balance after each payment, a line
number, the amount carried from one line to the next. Crystal Reports has running total fields
and SSRS a `RunningValue` function; ReportCraft's formulas are Excel-style, so two functions do it:
`RUNNINGSUM([field])` and `RUNNINGCOUNT([field])`.

- They run over the rows the band summarises, in their order, up to the current row: in the
  detail band, the rows of its innermost group (all rows without groups); in a group header, the
  group's first row; in a group or report footer, all of them, which is the total. A page header
  or footer summarises no rows: blank. Running over an outer group, the whole report or the page
  (page totals) is on the roadmap.
- `RUNNINGSUM` adds as `SUM` does (numbers and numeric text, blanks and other text skipped) and an
  error stays from its row on; `RUNNINGCOUNT` counts the values that aren't blank, as `COUNTA`
  does, so `RUNNINGCOUNT([item])` numbers the lines.
- Speed: each group's values are computed once, in one pass, and kept with where each row is in
  the group (`formula::Memo`), so a group of a million rows costs a million steps, not a million
  squared. A running total depends on the current row, so an aggregate around it isn't
  remembered as one that doesn't.
- A named formula can't hold one: named formulas are computed per row before grouping, and are
  already refused when they summarise rows.

