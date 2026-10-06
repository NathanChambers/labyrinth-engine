# Repository guidance

When implementing or materially changing an engine system's state ownership, lifecycle, public contract, or interaction with another system, use the repository's `document-engine-system` skill and update the affected pages in `docs/systems/` and `docs/system-map.md`.

When asked to review documentation accuracy or when a cross-system change leaves the documented flow uncertain, use `audit-engine-docs`.

Keep proposed designs distinct from implemented behavior. Small edits that do not change documented behavior need no documentation update.

Rust style: keep function declarations, calls, and constructor arguments on one physical line whenever they fit within the repository's 200-column `rustfmt.toml` limit. Keep struct literals and long data tables multiline when that improves readability.
