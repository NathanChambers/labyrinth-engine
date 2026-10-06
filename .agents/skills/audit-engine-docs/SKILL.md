---
name: audit-engine-docs
description: Check engine system pages and their connection map against current code and contracts when reviewing documentation drift or cross-system changes.
---

# Audit engine documentation

Use this skill for a requested documentation review or a material integration change. Do not run a repository-wide audit for an isolated local edit.

1. Identify the affected systems from the change, request, and [docs/systems/README.md](../../../docs/systems/README.md).
2. Trace each affected flow from entry point through owner to consumer. Inspect relevant state writers, lifecycle paths, public contracts, and serialized forms.
3. Compare those findings with the system pages and [docs/system-map.md](../../../docs/system-map.md). Check ownership, direction of each interaction, timing, failure behavior, and source links.
4. Correct documentation that the implementation establishes. Record an unresolved discrepancy when code and intended design conflict; do not silently choose one.
5. Check catalog entries, relative links, and status labels for affected pages. Keep proposed systems distinct from implemented ones.
6. Report which pages and flows were checked, what changed, and any remaining uncertainty.

Use focused searches and checks around the affected systems. A documentation audit does not authorize unrelated code changes.
