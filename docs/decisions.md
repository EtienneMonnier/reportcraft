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

