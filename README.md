# ReportCraft

**Reports and dashboards from your data: an open-source, clean-room reporting tool in pure Rust,
in the spirit of Crystal Reports and Power BI.** Built to sit beside the ArtCraft Crafting Apps
(GridCraft, DeckCraft, WordCraft…), with the same conventions, a CLI and an MCP server.

> Status: **pre-alpha**. Paginated reports from CSV, SQLite or PostgreSQL data to HTML and PDF
> work today; formulas, Excel/Word export and the designer are on the [roadmap](docs/ROADMAP.md).

## Try it

```sh
cargo run -p reportcraft-cli -- render --sample rentals --param period="October 2026" --out rentals.html
cargo run -p reportcraft-cli -- render --sample rentals --format pdf --out rentals.pdf
cargo run -p reportcraft-cli -- sample rentals > rentals.rcr.json      # the report definition
cargo run -p reportcraft-cli -- render rentals.rcr.json --data mydata.csv --out out.html
cargo run -p reportcraft-cli -- render rentals.rcr.json --sqlite shop.db --query "SELECT * FROM rental" --out out.html
PGPASSWORD=… cargo run -p reportcraft-cli -- render rentals.rcr.json --postgres "host=db user=report dbname=erp" --query "SELECT …"
cargo run -p reportcraft-cli -- schema --postgres "host=db user=report dbname=erp"  # tables, columns, keys
cargo run -p reportcraft-cli -- validate rentals.rcr.json --data mydata.csv          # errors and warnings
cargo run -p reportcraft-cli -- commands                               # every command
cargo run -p reportcraft-cli -- mcp                                    # MCP server on stdio
```

## Docs

[Specification](docs/specification.md) · [Architecture](docs/architecture.md) · [Status](docs/STATUS.md) · [Roadmap](docs/ROADMAP.md) ·
[Decisions](docs/decisions.md) · [Contributing](CONTRIBUTING.md) · [Rules for agents](AGENTS.md)

## Licence

MIT or Apache-2.0, at your option. Crystal Reports, SSRS and Power BI are trademarks of their
owners; ReportCraft is not affiliated with them and contains none of their code or assets.
