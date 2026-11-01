# Architecture

> **Last updated:** 2026-11-01

How ReportCraft is built today. Rules for contributors are in [`AGENTS.md`](../AGENTS.md).

## Workspace and layering

An engine-first Cargo workspace (edition 2024, resolver 3), laid out like the other Crafting Apps.
`cargo xtask layers` enforces the layers; nothing below L6 depends on a UI toolkit.

| Layer | Crate | Job |
|---|---|---|
| L0 | `core` | `Value`, `FieldType`, `Column`, `RowSet`, the shared `Error`, input caps (`limits`) |
| L0 | `barcode` | Code 128, EAN-13 and QR code encoders (module grids), written from the public standards |
| L1 | `model` | The report definition (`*.rcr.json`, `format_version`): page setup, parameters, sort, groups, bands, elements |
| L2 | `data` | `DataSource` and `SchemaSource` traits, schema types; `MemorySource`, `CsvSource` (wasm-safe) |
| L3 | `sqlite` | Read-only SQLite source and schema discovery (native only; implements `DataSource`, `SchemaSource`) |
| L3 | `postgres` | Read-only PostgreSQL source and schema discovery (native only; implements `DataSource`, `SchemaSource`) |
| L3 | `layout` | Banded layout to `Document` (pages of positioned text); HTML writer |
| L5 | `engine` | `Session`, the command registry (`cmd.rs`), built-in samples |
| L6 | `mcp` | MCP server over stdio (hand-written JSON-RPC), one tool per command |
| L6 | `ui-egui` | _(planned, R3.1)_ the designer |
| L7 | `apps/reportcraft-cli` | `commands`, `render`, `run`, `sample`, `mcp`, `version` |
| — | `xtask` | `ci`, `layers` |

## Data flow

1. Every action is a command: `Session::execute(id, params)`. A panic escaping a command is caught
   and the session restored.
2. A data source reads a `RowSet` (capped by `limits::MAX_ROWS`).
3. `layout::layout` sorts rows by the group fields then the sort keys, walks them, and emits bands:
   report header, page header, group headers, detail, group footers, report footer, with the page
   footer at the bottom of each page. A band that doesn't fit moves to the next page. Aggregates
   are computed over the rows of the enclosing group (or the whole report). Page numbers are filled
   in once the page count is known.
4. Writers turn the `Document` into HTML (today), PDF, XLSX or DOCX (roadmap R2).

## Report file

JSON, versioned by `format_version` (currently 1). `reportcraft-cli sample rentals` prints a
complete example. Units are millimetres; font sizes are points.
