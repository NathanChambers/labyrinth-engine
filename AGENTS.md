# Repository guidance

When implementing or materially changing an engine system's state ownership, lifecycle, public contract, or interaction with another system, use the repository's `document-engine-system` skill and update the affected pages in `docs/systems/` and `docs/system-map.md`.

When asked to review documentation accuracy or when a cross-system change leaves the documented flow uncertain, use `audit-engine-docs`.

Keep proposed designs distinct from implemented behavior. Small edits that do not change documented behavior need no documentation update.

Rust style: keep function declarations, calls, and constructor arguments on one physical line whenever they fit within the repository's 200-column `rustfmt.toml` limit. Keep struct literals and long data tables multiline when that improves readability.

## Running engine scene inspection

- This trusted project configures the `labyrinth_engine` MCP server in `.codex/config.toml`. Use `pause_engine`, `get_engine_status`, `resume_engine`, `list_render_buffers`, `capture_render_buffer`, `inspect_grids`, `pick_viewport_pixel`, `inspect_sdf_voxel`, `inspect_radiance_probe`, `get_performance_stats`, and `profile_frames` when the user asks to inspect or profile a running Labyrinth Engine scene. Pause first to align multiple inspection queries to one completed frame; resume after analysis. `profile_frames` requires the engine to be running, and GPU timestamp sampling synchronizes with the GPU.
- Start a project with `LABYRINTH_MCP_PORT=43856` in its environment to enable the local loopback debug endpoint; for PowerShell, run `$env:LABYRINTH_MCP_PORT = "43856"` before launching the app. Captures and inspection queries only read renderer data; pause/resume tools control the update and render loop.
- The endpoint is local-only and disabled unless `LABYRINTH_MCP_PORT` is set in the engine process. Captures and inspection queries do not change scene state; pause/resume commands intentionally control application updates and rendering.
- After changing the MCP adapter or tool definitions, start a new Codex session or restart Codex so it launches the updated adapter and refreshes the tool catalog. Existing sessions can retain stale MCP tools.
