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
