---
name: document-engine-system
description: Create or update an engine system page when implementing or materially changing its state ownership, behavior, lifecycle, public contract, or integration.
---

# Document an engine system

Use this skill for a concrete subsystem, not for a small edit that leaves its documented behavior unchanged.

1. Trace the behavior through its owner, callers, consumers, and relevant serialized data before writing.
2. Read [docs/systems/README.md](../../../docs/systems/README.md) for the documentation layout and [docs/systems/template.md](../../../docs/systems/template.md) for the page shape.
3. Create or update one page under `docs/systems/` for the system that owns the behavior. State what it owns, how it changes, how it starts and stops, and what callers can rely on.
4. Record each cross-system interaction in a table with its producer, consumer, data or command, and timing. Link to the other system page when it exists.
5. Update the catalog in `docs/systems/README.md` and the implemented relationships in `docs/system-map.md` when the change adds, removes, or changes an interaction.
6. Link claims to relevant source paths and checks. Mark proposals and unknowns explicitly; do not describe planned behavior as implemented.
7. Compare the page and map with the affected code and contracts. Report any unresolved gap.

Keep the page useful to game authors and engine contributors. Explain observable behavior and ownership before implementation details.
