# ReportCraft

**Reports and dashboards from your data: an open-source, clean-room reporting tool in pure Rust,
in the spirit of Crystal Reports and Power BI.** Built to sit beside the ArtCraft Crafting Apps
(GridCraft, DeckCraft, WordCraft…), with the same conventions, a CLI and an MCP server.

> Status: **pre-alpha**. Paginated reports from CSV, SQLite, PostgreSQL or SQL Server data, with Excel-style
> formulas, to HTML, PDF, XLSX and DOCX work today; the designer shows and previews reports (editing
> is next on the [roadmap](docs/ROADMAP.md)).

## Try it

```sh
cargo run -p reportcraft                                                # the designer, on the sample
CRAFT_I18N_DIR=$PWD/../craft-i18n cargo run -p reportcraft -- --lang fr   # in French (craft-i18n checkout)
cargo run -p reportcraft-cli -- render --sample rentals --param period="October 2026" --out rentals.html
cargo run -p reportcraft-cli -- render --sample rentals --format pdf --out rentals.pdf
cargo run -p reportcraft-cli -- render --sample rentals --format xlsx --out rentals.xlsx  # or docx
cargo run -p reportcraft-cli -- sample rentals > rentals.rcr.json      # the report definition
cargo run -p reportcraft-cli -- render rentals.rcr.json --data mydata.csv --out out.html
cargo run -p reportcraft-cli -- render rentals.rcr.json --sqlite shop.db --query "SELECT * FROM rental" --out out.html
PGPASSWORD=… cargo run -p reportcraft-cli -- render rentals.rcr.json --postgres "host=db user=report dbname=erp" --query "SELECT …"
SQLCMDPASSWORD=… cargo run -p reportcraft-cli -- render rentals.rcr.json --sqlserver "Server=tcp:db,1433;User Id=report;Database=erp" --query "SELECT …"
cargo run -p reportcraft-cli -- schema --postgres "host=db user=report dbname=erp"  # tables, columns, keys
cargo run -p reportcraft-cli -- render sales.rcr.json --out sales.pdf --format pdf  # a report that names its saved connection loads its own data
cargo run -p reportcraft-cli -- render catalogue.rcr.json --connection odoo --query "SELECT …, a.store_fname AS photo …" --pictures ~/.local/share/Odoo/filestore/mydb --format pdf --out catalogue.pdf  # Odoo pictures
cargo run -p reportcraft-cli -- import-qweb report_invoice.xml --out invoice.rcr.json --sample-data invoice.csv  # an Odoo QWeb template
cargo run -p reportcraft-cli -- render invoice.rcr.json --data invoice.csv --format pdf --out invoice.pdf          # previewed on sample rows
cargo run -p reportcraft-cli -- validate rentals.rcr.json --data mydata.csv          # errors and warnings
cargo run -p reportcraft-cli -- commands                               # every command
cargo run -p reportcraft-cli -- mcp                                    # MCP server on stdio
```

## Docs

[Specification](docs/specification.md) · [Architecture](docs/architecture.md) · [Status](docs/STATUS.md) · [Roadmap](docs/ROADMAP.md) ·
[Decisions](docs/decisions.md) · [Contributing](CONTRIBUTING.md) · [Rules for agents](AGENTS.md)

## Project status and maintenance

ReportCraft is a personal, non-commercial project, offered to the community as a reporting tool in
the continuity of the ArtCraft suite. Its author may not maintain it for years: contributions,
forks and new maintainers are welcome, and the docs ([AGENTS.md](AGENTS.md),
[decisions](docs/decisions.md), [roadmap](docs/ROADMAP.md)) are written so someone else can take it
over.

## Licence

MIT or Apache-2.0, at your option. Crystal Reports, SSRS and Power BI are trademarks of their
owners; ReportCraft is not affiliated with them and contains none of their code or assets.
