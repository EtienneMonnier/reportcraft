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
