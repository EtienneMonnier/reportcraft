# Functional Requirements Specification

## ReportCraft — Rust and Model Context Protocol (MCP)

**Version:** 1.1 (2026-11-01) · `docs/specification.md`  
**Status:** Initial functional baseline, aligned with the ArtCraft Crafting Apps conventions  
**Audience:** Software engineering agents operating in an isolated development environment, architects, and testers

**Precedence:** this document says *what* ReportCraft does. *How* it is built is governed by the
repository's `AGENTS.md` (never-crash rules, layering, commands, clean room, git workflow) and the
conventions of the sibling Crafting Apps (`storytold/gridcraft`, `deckcraft`, `craft-libs`…). Where
this document and `AGENTS.md` disagree, `AGENTS.md` wins and this document is corrected.
Work is planned and tracked in `docs/ROADMAP.md` (items `R<milestone>.<n>`); §14 maps the
requirements below to it.

## 1. Purpose and Vision

Build a universal reporting application that enables users and authorized agents to design, edit, execute, view, print, export, and distribute reports using heterogeneous data sources.

The platform shall support two complementary reporting modes on a shared foundation:

- **Paginated reports:** precisely laid-out documents for printing, archiving, and formal delivery, including invoices, quotations, delivery notes, financial statements, labels, and forms.
- **Interactive reports:** exploratory dashboards featuring visualizations, filtering, navigation, drill-down, and data refresh.

Both modes shall reuse compatible data definitions, parameters, calculations, access controls, and execution services without requiring the same presentation model.

### 1.1 Product shape and first-release scope

ReportCraft is a Crafting App: a native desktop application (macOS, Windows, Linux, BSD) with a
headless CLI (`reportcraft-cli`) and an MCP server driven by the same engine, and a web build
(wasm) for the lower layers. The first releases are **single-user, local and offline**: the user
works with their own files and their own database connections. Server, scheduled, multi-user and
multi-tenant deployment (§9 scheduling, §12 roles, §18.2, §18.7) are later profiles and must not
shape P0–P2 beyond keeping the core headless.

First reference use cases: business documents and statistics over Odoo (PostgreSQL) and Rent+
(Advantage Database Server, through ODBC or exported data) — invoices, delivery notes, quotations,
rental and rotation statistics.

### 1.2 Clean room and licensing

- Existing products (Crystal Reports, SSRS, Power BI, JasperReports…) may only be **observed
  black-box** — run with synthetic data, studied through their public documentation. Nothing
  inside them is read, disassembled or decompiled; no file they produced (`.rpt`, `.pbix`,
  exported PDFs) and no customer report is committed; no template, icon or wording is copied
  beyond feature names. Observations live in `plan/observed/` (not committed).
- No GPL/AGPL/LGPL code is copied. Odoo (QWeb) and o-spreadsheet are LGPL: their formats and
  behaviour may be implemented from documentation, their code may not be reused.
- ReportCraft is dual-licensed MIT OR Apache-2.0; every asset is original or openly licensed and
  listed in `ATTRIBUTION.md`.

## 2. Data Sources and Data Modeling

### FR-DATA-001 — Connections

Support relational databases, structured files, HTTP services, and extensible data providers. Connector targets, in order: CSV (done), SQLite (R1.1), PostgreSQL (R1.2), ODBC — covering SQL Server and legacy databases such as Advantage (R1.4) — then MySQL/MariaDB, JSON, XML and REST APIs. Every connector is an implementation of the `DataSource` trait; adding one does not change the core engine (§10).

Connections are **read-only by default**: ReportCraft reads data, it never writes to a data source. Credentials are kept outside report definitions (§12).

### FR-DATA-002 — Discovery and Query Design

Users shall be able to inspect schemas, tables, views, columns, data types, keys, and relationships; preview data; create queries visually; and write SQL directly where supported.

### FR-DATA-003 — Semantic Modeling

Support inner and outer joins, relationships between datasets, aliases, calculated fields, type conversion, and filters. The interface shall make potentially duplicating joins and ambiguous relationships identifiable.

### FR-DATA-004 — Execution

Support securely bound query parameters, result pagination or streaming where available, execution timeouts, cancellation, large datasets, and optional caching. Do not construct parameterized SQL through uncontrolled string concatenation.

## 3. Visual Report Designer

Provide a graphical, low-code designer with:

- Drag-and-drop fields and components.
- Grids, rulers, guides, snapping, alignment, and distribution.
- Precise positioning with configurable units.
- Property inspector and component hierarchy.
- Undo, redo, duplication, and copy/paste.
- Preview using live or sample data.
- Autosave, version history, and reusable templates/components.

Design mode and execution mode shall be clearly separated.

The designer is an egui interface built on craft-libs (`craft-ui`, `craft-settings`, `craft-keymap`…) like the other Crafting Apps, and stays thin: **every user-visible action is an engine command** (`file.new`, `data.loadCsv`, `report.render`…) that the CLI, the MCP server and a local control channel can also run. A command called programmatically never opens a dialog. Interface text is translatable through craft-i18n; fonts come from craft-fonts.

## 4. Paginated and Printable Reports

### FR-PRINT-001 — Document Sections

Support report headers/footers, page headers/footers, group headers/footers, and repeating detail sections. Each section shall independently support visibility, sizing, repetition, and conditional behavior.

### FR-PRINT-002 — Page Layout

Support A4, A5, Letter, and custom paper sizes; portrait and landscape orientation; margins; multiple columns; conditional page breaks; page numbering; optional blank pages; and sections whose content grows dynamically.

### FR-PRINT-003 — Elements

Support plain/rich text, bound fields, tables, images, lines, shapes, barcodes, QR codes, charts, text boxes, and embedded subreports.

### FR-PRINT-004 — Advanced Pagination Behavior

Support conditional section suppression, keep-together rules, repeated group headers, controlled splitting, and independent or linked subreports. Expressions may govern visibility, color, formatting, permitted dimensions, and page breaks.

### FR-PRINT-005 — Rendering Fidelity

On-screen preview, exported documents, and printed output shall share a consistent reference geometry. Differences arising from fonts or printer drivers shall be detectable and documented.

## 5. Interactive Reports and Dashboards

Users shall be able to compose pages containing tables, pivot tables, KPI indicators, metric cards, bar charts, line charts, pie charts, scatter plots, time series, and filters.

Required interactions include cross-filtering, hierarchical drill-down, drill-through to detailed records, global and local filters, time-range selection, sorting, and period comparisons.

Data shall refresh on demand and, when supported by the provider, on a configurable schedule. The interface shall display the last refresh timestamp and warn about stale results when appropriate.

## 6. Expression and Calculation Engine

Provide a shared, typed, deterministic expression engine supporting arithmetic, boolean, string, date/time, conditional, and aggregation operations.

Minimum functions include sum, average, minimum, maximum, count, conditional aggregation, rounding, type conversion, string manipulation, and date arithmetic.

Evaluation contexts shall be explicit: record, group, report, page, and interactive filter context. Pagination-dependent calculations must execute at the correct rendering stage.

Specify behavior for null values, division by zero, invalid conversions, and circular dependencies. Local, report-scoped, and cross-component variables shall have well-defined scope and evaluation order. Avoid calculations that silently depend on implicit execution order.

## 7. Parameters, Filters, and Interactions

Reports shall declare typed parameters including string, number, boolean, date, date range, single-choice, and multi-choice.

Parameters may have default values, allowed-value lists, validation rules, and dependencies on other parameters.

Filters may operate at source, dataset, group, component, or visualization level. Interactive report viewing state shall be shareable and exportable without modifying the underlying report definition.

## 8. Subreports and Document Composition

Reports shall embed other reports as components. Subreports shall accept parent-supplied parameters, use independent data sources and calculations, and expose results to the parent when explicitly configured.

Define behavior when a subreport returns no rows, fails, or exceeds available page space.

Document composition shall support combining templates such as cover pages, main content, appendices, contractual terms, and supplementary documents.

## 9. Export, Printing, and Distribution

Target formats include PDF, HTML, CSV, XLSX, DOCX, JSON, and PNG/SVG where applicable. PDF output goes through craft-libs `craft-pdf` and printing through `craft-print`; XLSX/DOCX writers reuse or match the suite's (GridCraft, WordCraft) so that exported files open cleanly in them.

Exports shall respect report intent: visual fidelity for paginated documents and data structure for analytical exports.

Users shall configure page ranges, scaling, print parameters, and output filenames. Batch generation runs from the CLI. Scheduled execution, delivery to authorized destinations, and archiving with execution parameters and report versions belong to the server profile (§1.1).

### 9.1 Import and migration formats

To help users move existing reports, ReportCraft shall read, from public documentation only (§1.2):

- Odoo QWeb report templates: visual editing and preview on sample data (R4.1).
- Power BI text projects (PBIP/TMDL) (R4.2) and Odoo spreadsheet dashboards (o-spreadsheet JSON) (R5.2).
- Crystal Reports `.rpt` files, read-only, **only if** a clean-room route exists (R4.3, decision pending).

Unsupported parts of an imported file are reported to the user, never silently dropped.

## 10. Functional Architecture and Rust Core

The application shall be built around a Rust core independent of the user interface, as an engine-first Cargo workspace whose layers are enforced by `cargo xtask layers` (a crate depends only on lower layers; nothing below L6 depends on a UI toolkit):

| Layer | Crate | Responsibility |
|---|---|---|
| L0 | `core` | Values, types, errors, input limits |
| L1 | `model` | Canonical, versioned report definition |
| L2 | `data` | Connectors (`DataSource`), queries, semantic model |
| L3 | `layout` (+ future `expr`, `chart`) | Expressions/aggregations, pagination, writers |
| L5 | `engine` | Session, command registry, execution orchestration, history |
| L6 | `mcp`, `ui-egui` | MCP server, designer and viewer |
| L7 | `apps/*` | Desktop app, CLI, web build |

L0–L3 shall build for wasm. A future server profile adds services above the engine; it does not move logic out of it.

Report definitions shall use a structured, versioned, UI-independent format with stable object identifiers.

Extension contracts (data connectors, calculation functions, visual components, exporters) are **Rust traits registered in one place**, compiled in. The workspace forbids `unsafe`, so there is no in-process dynamic plugin loading; runtime extensions, if ever needed, run out of process (for example as a subprocess or MCP server) with explicit permissions. Errors shall be typed (`Result` with each crate's error type) and traceable. Long-running operations shall support cancellation and configurable CPU, memory, and execution-time limits.

## 11. Model Context Protocol (MCP) Integration

Expose an MCP-compatible server through which authorized agents can discover, design, edit, validate, execute, and export reports.

Minimum capabilities:

- Discover available data sources and inspect schemas.
- Create and update datasets and semantic models.
- Create and modify reports, sections, elements, parameters, and expressions.
- Validate report definitions and calculations.
- Preview and execute reports.
- Export generated artifacts and inspect execution status.
- Cancel long-running operations.

MCP tools are the engine commands (one tool per command, named after the command id with `.` replaced by `_`), so the MCP server, the CLI and the designer always behave the same; the server exists from R0 and grows with the command catalog.

All operations shall use documented structured input/output schemas. Read operations must be distinguished from destructive or privileged actions. The MCP server shall enforce the effective user's permissions rather than grant elevated agent privileges: in the desktop profile, an agent can use only the connections the user configured, with their database privileges, and actions that publish, overwrite files or reach new data sources require the user's confirmation.

Calls shall be auditable, modifications traceable, and execution results identifiable. Long-running operations shall expose status and cancellation mechanisms.

**Architectural rule:** MCP is an adapter to the application core, not a separate implementation of business logic.

## 12. Security and Governance

In the desktop profile, identity is the operating-system user and data access is governed by each connection's database role. In the server profile (§1.1), support authentication, roles, permissions by report and data source, and data-access restrictions.

Connection credentials shall be stored securely and excluded from portable report definitions. Secrets must not appear in logs or MCP responses.

Custom queries shall respect connection privileges and administrator-defined limits.

Execution audit records shall include the acting identity, report identifier/version, non-sensitive parameters, timestamps, outcome, and relevant errors.

## 13. Performance, Reliability, and Quality

**Never crash.** A crash loses the user's work, so robustness outranks features: no panics in production code (`unwrap`, `expect`, `panic!` and similar are denied by lints), no `unsafe`, errors returned as typed results, every input (report files, data, query results, CLI/MCP parameters) treated as hostile, input-sized work capped, recursion bounded. A command that fails leaves the session as it was. Every crash fix comes with a regression test.

Dates and times go through `chrono`; no hand-written calendar arithmetic.

Use incremental or streaming data processing where possible rather than loading all data into memory; until then, row counts are capped and the cap is reported to the user.

Rendering engines shall be independently testable. Given identical input data, parameters, report version, and rendering environment, results should be reproducible.

Automated tests shall cover formatting, aggregations, grouping, subreports, page breaks, null values, empty datasets, connection failures, and large data volumes.

Benchmark performance using reference datasets. Numeric targets for latency, memory, and concurrency shall be established before release acceptance.

## 14. Implementation Phases

| Phase | Scope | Expected deliverable | Roadmap items |
|---|---|---|---|
| P0 — Foundation | Report model, initial connectors, queries, validation, CLI and MCP | Standalone Rust core | R0 (done), R1 |
| P1 — Pagination | Sections, groups, layout, expressions, formats, PDF, print | Complete printable reports | R2 |
| P2 — Design | Visual editor, properties, templates, history | No-code report creation | R3 |
| P2b — Migration | QWeb, PBIP/TMDL, `.rpt` (if clean-room route) | Existing reports brought over | R4 |
| P3 — Analytics | Analytical model, charts, filters, interactions | Operational dashboards | R5 |
| P4 — Automation | Server profile, API, scheduling, distribution, roles | Automated integrations and operations | not yet planned |

The MCP server and CLI exist from P0, because every command is reachable from them as soon as it exists; P4 adds what a shared server needs (identities, scheduling, audit at scale).

**Implementation status (2026-11-01).** Implemented: versioned report model with validation; CSV source; banded layout with report/page/group headers and footers, nested groups, sorting, sum/count/avg/min/max, parameters, page breaks and page numbers; HTML output; CLI; MCP server. Specified (this document, roadmap): everything else. Status follows §23's vocabulary.

## 15. Functional Acceptance Scenarios

The initial acceptance suite shall demonstrate that the platform can:

1. Connect to a database, discover its schema, and run a parameterized query.
2. Design a multi-page document with groups, totals, repeated headers, and conditional visibility.
3. Generate a PDF matching its preview, including sections and subreports spanning multiple pages.
4. Build a dashboard whose visualizations respond to a shared filter.
5. Reuse one business calculation across multiple reports without duplicating its definition.
6. Export paginated and analytical reports into suitable formats.
7. Edit a report through the graphical interface or MCP and obtain consistent results.
8. Reject unauthorized operations and audit sensitive actions (desktop profile: an MCP agent cannot reach a connection the user did not configure, and confirmation-gated actions are refused without confirmation).
9. Handle empty datasets, connection errors, and invalid calculations without silently returning incorrect results.
10. Load an older report-definition version or explicitly report incompatibility.

## 16. Development Rules for the Coding Agent

Prioritize a reliable functional core before expanding the graphical component library. Each feature must have a data contract, validation rules, automated tests, and explicit error handling.

Maintain strict separation between data acquisition, calculations, composition, rendering, and distribution. Share appropriate services between the two reporting modes while preserving presentation engines tailored to their distinct purposes.

A successful build is not proof of feature completion. Every feature must pass a reproducible functional acceptance scenario.

Every change also follows `AGENTS.md`: one roadmap item per branch and PR, `cargo xtask ci` green before committing, `docs/ROADMAP.md` and `docs/STATUS.md` updated in the same PR, and a PR title written as the squash commit message (plain English, present tense, what changed for the user).

## 17. Required Follow-up Technical Specifications

This document is the initial functional baseline, not an exhaustive implementation specification. Before autonomous implementation, define and version:

- The canonical report-definition schema (including migrations and compatibility rules).
- Rust module interfaces and extension contracts.
- The MCP tool/resource catalog and request/response schemas.
- Exact pagination, grouping, and expression evaluation semantics.
- Permission boundaries and secret-handling rules.
- Executable acceptance fixtures, golden rendering tests, and performance benchmarks.

These artifacts shall form the authoritative implementation contract for the coding agent.

---

## 18. Long-Term Product Vision (3–5 Years)

### 18.1 Strategic destination

Evolve the application from a report designer into a **headless-capable, embeddable, extensible reporting and analytical platform**. The product shall support deterministic document production, exploratory analysis, and agent-driven workflows through one versioned semantic foundation. It must remain useful without an AI agent, an internet connection, or a specific host application.

Long-term ambitions in this section are **roadmap candidates, not commitments for the first release**. Each capability must pass architecture review, threat modeling, and explicit prioritization before implementation.

### 18.2 Deployment and product independence

- Support desktop/offline, on-premises service, containerized service, and managed multi-tenant deployment profiles.
- Make the Rust execution engine usable as a library and as a standalone service, without requiring a graphical frontend.
- Support an embedded viewer/designer with configurable branding, localization, permissions, and host-provided authentication context.
- Keep data connectors, report definitions, and rendered outputs portable across supported deployment profiles, subject to documented compatibility constraints.
- Avoid mandatory dependence on proprietary cloud services; enable reproducible builds and documented backup/restore procedures.

**Success criterion:** A report created in a supported desktop environment can be validated and executed by a headless server using the same versioned definition and compatible connectors.

### 18.3 AI-native authoring and MCP evolution

- Allow an authorized agent to discover schemas, propose a semantic model, create a report draft, refine layout, explain expressions, diagnose failures, and request previews.
- Provide granular MCP operations for reading, proposing, validating, applying, reverting, and comparing changes; prefer structured patches to wholesale replacement.
- Introduce a **propose → validate → preview → approve → publish** workflow for agent-authored changes.
- Provide dry-run execution, resource estimates, change diffs, structured diagnostics, and machine-readable acceptance results.
- Treat database content, report text, and retrieved metadata as untrusted input; never let them override tool permissions or agent policy.
- Require explicit authorization for publishing, distributing, modifying data sources, or accessing sensitive information.
- Ensure agent-generated outputs remain fully editable in the graphical designer and are not tied to an AI-specific format.

**Success criterion:** An agent can generate a draft from an authorized schema, validate it, show a preview, and request approval without publishing or exposing secrets automatically.

### 18.4 Semantic and analytical capabilities

- Introduce governed metrics, dimensions, hierarchies, calendars, fiscal periods, reusable calculations, and lineage tracking.
- Support semantic versioning of business metrics and impact analysis when a source field or formula changes.
- Provide cross-report drill-through, saved analytical views, reusable filters, and controlled sharing of filter states.
- Investigate incremental refresh, materialized aggregates, streaming updates, and hybrid query strategies where appropriate.
- Support advanced time intelligence, period-over-period comparisons, cohort analysis, and extensible statistical functions.
- Keep complex analytical processing optional: small datasets and simple documents must remain lightweight.

**Success criterion:** A governed metric can be referenced by multiple documents and dashboards, with traceability to its definition and data source.

### 18.5 Advanced document composition and fidelity

- Develop a deterministic pagination pipeline with explicit layout constraints, overflow diagnostics, font embedding/substitution policies, and reproducible rendering fixtures.
- Support reusable master pages, conditional sections, multi-document assembly, appendices, signatures, and rich text spanning multiple pages.
- Investigate accessible document output, tagged PDF, archival-oriented PDF profiles, and locale-aware typography subject to format-specific conformance testing.
- Support barcode/QR rendering, labels, batch generation, printer-specific profiles, and optional digital signing through isolated adapters.
- Provide a visual regression framework to detect layout changes between engine versions.

**Success criterion:** Complex reference documents produce stable page counts and geometry across supported execution environments, with documented tolerance rules.

### 18.6 Extension ecosystem

- Publish stable, versioned contracts for data connectors, expression functions, visual components, exporters, authentication adapters, and distribution targets.
- Define extension manifests, capability declarations, compatibility checks, and controlled installation/update procedures.
- Prefer isolated execution or sandboxing for third-party extensions; enforce time, memory, filesystem, and network permissions.
- Supply developer documentation, sample extensions, conformance tests, and a compatibility matrix.
- Treat an extension marketplace or community catalog as optional, after security and lifecycle controls are mature.

**Success criterion:** A new connector can be added as a trait implementation and validated by the conformance tests without changing core code; installing one without rebuilding is possible only out of process (§10), since in-process dynamic loading would require `unsafe`.

### 18.7 Enterprise operations and governance

- Introduce workspace/project isolation, tenant isolation, role- and attribute-based access control, and optional row-level security integration.
- Add immutable audit events, lineage, retention policies, backup/restore, disaster-recovery procedures, and operational telemetry.
- Support SSO through standards-based identity adapters, service identities, secret rotation, and policy-driven data export.
- Provide execution queues, concurrency quotas, cancellation, scheduling, retry policies, and alerting on failures.
- Separate report ownership, review, approval, publication, and consumption permissions.

**Success criterion:** A deployment can prove who changed, approved, executed, and exported a report, while enforcing tenant and data boundaries.

### 18.8 Developer experience and lifecycle

- Store report definitions in a canonical, human-reviewable, machine-validatable representation suitable for version control.
- Provide schema migrations, compatibility tests, semantic diffs, deterministic fixtures, and report-as-code workflows.
- Support development, testing, staging, and production promotion with approvals and rollback.
- Publish SDKs or client libraries only where stable contracts justify their maintenance.
- Maintain reference datasets and golden outputs for pagination, calculations, security, and performance regression testing.

**Success criterion:** A report change can pass automated validation and rendering regression tests before being promoted to production.

## 19. Architectural Guardrails for Future-Proofing

1. **One canonical definition:** The visual designer, APIs, MCP tools, and execution engine must operate on the same versioned report model; no separate agent-only document format.
2. **Two distinct rendering concerns:** Paginated layout and interactive visualization share data and expressions but retain independent presentation pipelines.
3. **Stable contracts before plugins:** Extension points must be explicitly versioned and tested before third-party support is promised.
4. **Security by default:** Treat queries, extensions, templates, and MCP inputs as untrusted; enforce least privilege and resource quotas.
5. **Offline-first capability:** Core authoring and execution must not require an external AI or hosted control plane.
6. **Reproducibility:** Pin report definition, semantic model, input snapshot, locale, fonts, and engine version where exact output reproduction is required.
7. **Observability:** Every execution must have a correlation identifier, structured status, timings, and safe diagnostics.
8. **No speculative complexity in P0:** Future requirements must inform interfaces without forcing premature implementation of distributed infrastructure.

## 20. Long-Term Roadmap and Decision Gates

| Horizon | Primary outcomes | Gate before proceeding |
|---|---|---|
| Foundation (P0–P2) | Reliable Rust core, canonical schema, paginated rendering, graphical authoring | Golden tests, schema stability, successful end-to-end document workflows |
| Product completion (P3–P4) | Interactive analytics, MCP operations, scheduling, access control | End-to-end authorization, agent approval workflow, dashboard acceptance tests |
| Expansion (12–24 months, indicative) | Embedding SDK, extension API, semantic catalog, deployment profiles | Proven customer use cases, extension security model, migration strategy |
| Platform maturity (24–36 months, indicative) | Enterprise governance, multi-tenant isolation, advanced operational controls | Threat modeling, tenant-isolation tests, reliability objectives |
| Ecosystem (36–60 months, indicative) | Broader extension ecosystem, advanced analytics, collaborative workflows | Stable public contracts, maintenance capacity, adoption evidence |

These horizons are illustrative planning bands, not delivery dates. Reprioritize them according to validated use cases, staffing, and operational constraints.

## 21. Long-Term Non-Goals and Scope Controls

The platform is **not initially** intended to be a general-purpose database, an ETL orchestration suite, a spreadsheet editor, a full data-science workbench, or an autonomous system allowed to publish reports without human-defined authorization. Any future overlap with these domains must be justified by user needs and assessed against maintenance cost.

Do not implement distributed execution, custom query languages (queries stay in each source's own language, such as SQL; the expression language of §6 is for calculated fields only), proprietary file formats, or a plugin marketplace merely because they appear in the long-term vision. Prefer established interoperable standards and minimal abstractions until concrete requirements justify greater complexity.

## 22. Roadmap Validation Metrics

Track measurable outcomes rather than feature count alone:

- **Authoring:** Time to create a basic document, complex grouped document, and interactive dashboard; number of manual corrections required.
- **Rendering:** Golden-test pass rate, layout regression count, export fidelity, and reproducibility across supported environments.
- **Execution:** Latency percentiles, peak memory, concurrency, cancellation responsiveness, and failure rates on reference datasets.
- **Integration:** MCP operation success rate, validation coverage, approval compliance, and connector compatibility.
- **Security:** Unauthorized-access test pass rate, secret leakage incidents, tenant-isolation failures, and audit completeness.
- **Maintainability:** Schema migration success, extension compatibility, automated test coverage, and time to diagnose failures.

Numerical targets shall be established from measured baselines before each release gate; this specification intentionally does not invent performance promises.

## 23. Required Clean-Room Agent Planning Output

Before implementing any long-term feature, the coding agent shall produce a short design record, added to `docs/decisions.md` in the same PR, containing: the clean-room sources used (observations in `plan/observed/`, public documentation links — never another product's internals); the user problem; in-scope and out-of-scope behavior; affected modules; canonical schema changes; Rust and MCP contract changes; security implications; migration and rollback strategy; automated acceptance tests; performance risks; and a minimal implementation sequence.

The agent must explicitly distinguish **implemented**, **specified**, **experimental**, and **future** capabilities in documentation and user interfaces. No roadmap item shall be represented as production-ready solely because a placeholder API or prototype exists.
