# Labyrinth Engine MCP

Codex loads this MCP server from the trusted project's `.codex/config.toml`. It exposes frame pause/resume controls, render captures, scene-grid inspection, and renderer profiling from a running app.

Enable the engine endpoint in the project process before launching it:

```powershell
$env:LABYRINTH_MCP_PORT = "43856"
cargo run -p <example-package>
```

Use the package name for the example or project being run. Available tools:

- `pause_engine` freezes the application after its current completed frame. The app and renderer stop advancing while inspection requests continue to work.
- `get_engine_status` returns the pause state and last completed frame ID.
- `resume_engine` resumes application updates and rendering.
- `list_render_buffers` and `capture_render_buffer` capture color, depth, normals, shadows, and renderer debug views.
- `inspect_grids` returns SDF and radiance grid bounds, resolutions, spacing, and recent update counters.
- `pick_viewport_pixel` maps a pixel in the current camera viewport to the nearest scene triangle and returns its world position and containing grid coordinates. Supply screenshot dimensions when they differ from the current viewport.
- `inspect_sdf_voxel` reads one voxel's signed distance, albedo, emission, and clearance by grid coordinate.
- `inspect_radiance_probe` reads one probe's relocation state, coverage, angular radiance, sample counts, and sample ages by grid coordinate.
- `get_performance_stats` returns CPU phase timings, GPU pass timings when supported, workload counters, and GI coverage for the latest frame. It reads GPU timestamps on demand and may wait for the GPU.
- `profile_frames` collects 1–120 upcoming frames of CPU timings and workload counters, then reports mean, p50, p95, p99, and maximum frame CPU time. The engine must be running. Set `include_gpu_timing` to collect GPU pass times for each frame; this synchronizes with the GPU and can perturb frame pacing.

The endpoint binds only to `127.0.0.1`. Capture PNGs are retained below the engine's configured debug capture root in `mcp/`. Responses include a completed `frame_id` and pause state. Pause before taking multiple captures and queries to keep them aligned to one scene state, then resume when analysis is complete. Grid, probe, and performance queries read renderer-owned data on demand; `profile_frames` temporarily retains a bounded set of frame samples in the engine MCP coordinator and does not change scene or renderer settings.

If Codex was already open when `.codex/config.toml` was added, reload or restart the project session so it reads the project-scoped MCP configuration. After changing `server.py` or its tool definitions, start a new Codex session or restart Codex: an existing session can retain the previous tool catalog and adapter process. Verify that `get_performance_stats` and `profile_frames` appear in the server tools, then call `get_engine_status` while the scene is running. If the current session reports a response parsing error after the engine endpoint has changed, restart the Codex session to start the checked-in adapter version. Codex loads project-scoped configuration only for trusted projects.
