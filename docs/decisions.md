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
