# Debug MCP

**Status:** Implemented for frame pause/resume, render capture, scene-grid inspection, performance snapshots, and bounded frame profiling  
**Owner:** `crates/engine` coordinates live requests and owns bounded profile capture state; `crates/renderer` owns performance counters and GPU timestamp readback; `tools/labyrinth_mcp/server.py` adapts MCP to the local engine endpoint  
**Source:** [Engine endpoint](../../crates/engine/src/mcp_debug.rs), [renderer capture](../../crates/renderer/src/lib.rs), [MCP server](../../tools/labyrinth_mcp/server.py), [Codex project config](../../.codex/config.toml)

## Purpose and boundary

The Debug MCP lets Codex inspect and control the inspection lifecycle of a running Labyrinth Engine project. The MCP server exposes tools over stdio; the engine opens a loopback TCP endpoint only when `LABYRINTH_MCP_PORT` is set in the project process. Scene and render state remain owned by the engine and renderer. MCP can pause the application after a completed frame, inspect that frozen state, and resume application updates.

Each successful capture writes a PNG below `WindowSettings::debug_capture_root/mcp/` and returns it as an MCP image. The MCP server does not choose or move the camera. Omitting dimensions captures at the current window resolution; callers may request a custom resolution up to 4096 by 4096. The color and debug-view images contain the renderer scene and gizmos; the engine UI canvas is not included. Grid metadata and individual cell queries return structured JSON. Pixel picking casts through the supplied viewport dimensions against the renderer's current transformed scene triangles and returns a world hit plus containing grid coordinates. Every response identifies its last completed `frame_id` and whether the application is paused. Pause before capturing and querying to keep separate results on one frame.

## State and invariants

The MCP process owns protocol handling and does not retain scene state. The engine endpoint accepts only loopback connections and fixed commands with bounded coordinates, capture dimensions, and profile frame counts. While running, requests are serviced after a completed frame. `pause_engine` takes effect at that boundary: the `Runner` stops calling application updates and stops requesting redraws, retaining the last presented frame. The event loop remains responsive and services MCP inspection/capture requests while paused. `resume_engine` resumes redraws and resets the frame timer so paused wall-clock time is not applied as one large simulation delta. A 20 ms wait interval bounds pause request latency. GPU access and camera selection remain in the engine and renderer. MCP captures reuse the current GI cache without advancing its update scheduler; inspection queries do not change the active camera, render settings, or GI cache.

The engine MCP coordinator retains at most one in-progress profile with a maximum of 120 frame samples. It records renderer-owned `RendererPerformanceStats` after each completed frame and returns the frame IDs with the samples. CPU-only profiling does not request GPU readback. Optional per-frame GPU timestamp collection blocks for GPU results and can affect frame pacing; use it when GPU pass timings are needed, and compare CPU-only runs for less intrusive frame measurements. `get_performance_stats` is a one-shot sample that requests GPU timings and opt-in triangle-BVH counters for the latest completed frame.

The supported buffers are `color`, `depth`, `normals`, directional shadow cascades 0 through 2, `spot_shadow`, point shadow faces 0 through 11, `unlit`, `wireframe`, `shadow_visibility`, `gi_only`, and `gi_support`. `normals` is the depth-prepass world normal target mapped from signed normal components to RGB. Depth buffers are grayscale PNG previews; color and renderer debug views are PNG render output. Shader views are rendered through `RenderDebugMode` without changing the active application setting.

## Public contract

Codex loads the project-scoped server from `.codex/config.toml` when this project is trusted. MCP tool definitions are discovered when a Codex session starts; an existing session can retain its earlier tool catalog and adapter process after files change. Start a new session or restart Codex after changing the adapter or its tool definitions. The configured per-tool timeout is 200 seconds, above the adapter's 185-second profiling timeout. The MCP server provides:

- `pause_engine`: stops application updates and rendering after the current completed frame; returns that frame ID.
- `resume_engine`: resumes updates and rendering.
- `get_engine_status`: returns pause state and last completed frame ID.
- `list_render_buffers`: names the supported buffers.
- `capture_render_buffer`: accepts a buffer name and optional paired width and height, and returns a PNG image from the current active camera.
- `inspect_grids`: returns SDF and radiance grid bounds, resolution, voxel/cell spacing, and current GI update counters.
- `pick_viewport_pixel`: accepts screenshot pixel coordinates and optional viewport dimensions; returns hit position, normal, instance/triangle indices, and containing SDF/radiance coordinates.
- `inspect_sdf_voxel`: reads signed distance, albedo, emission, and conservative clearance for one SDF coordinate.
- `inspect_radiance_probe`: reads relocation state, coverage, angular radiance, sample/attempt counts, and ages for one radiance cell coordinate.
- `get_performance_stats`: returns the latest CPU phase timings, GPU pass timings when supported, work counters, and GI coverage. It performs an on-demand GPU timestamp readback.
- `profile_frames`: samples 1–120 upcoming frames and returns per-frame CPU timings and renderer work counters plus summary percentiles. The engine must be running. `include_gpu_timing` enables per-frame GPU timestamp readback and may perturb frame pacing.

Grid, pick, voxel, probe, performance, and status results include `{frame_id, paused, data}`. Image tool responses include frame ID and pause state in their text content. `profile_frames` returns the ending frame ID and per-sample frame IDs; it runs while the engine advances, so do not pause before starting it. To compare captures and single-frame queries, call `pause_engine`, collect data while its returned frame remains paused, then call `resume_engine`. To enable the engine endpoint, set `LABYRINTH_MCP_PORT=43856` in the environment of the project process before launching it. The MCP server uses that same port and connects to `127.0.0.1`. The feature is disabled when the engine process does not set the variable. Capture PNGs are also retained under the configured debug capture root for later inspection.

## Lifecycle

The endpoint starts during `run()` when the port environment variable is present. A listener thread accepts local requests and queues typed requests. While running, the engine services them after scene updates and the normal render. While paused, it services ordinary inspection requests from the event-loop wait path without advancing the scene or rendering. `profile_frames` holds its MCP request open while the engine records the requested number of future frames; a pause ends the trace with the samples collected so far. The renderer performs requested GPU timestamp readback and exposes its CPU counters; the engine tags results with completed frame IDs. The MCP adapter returns image or text content. Resume resets the elapsed-time baseline. The listener and its pending requests end with the engine process.

## Connections

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| Codex MCP client | [MCP adapter](../../tools/labyrinth_mcp/server.py) | MCP initialize, tool listing, render captures, and grid inspection calls over stdio | MCP session |
| MCP adapter | Engine debug endpoint | Whitelisted local pause, resume, status, capture, grid, pick, SDF-voxel, or radiance-probe command | Tool invocation |
| MCP adapter | Engine MCP profiler | One-shot performance query or bounded profile request with optional GPU timestamp sampling | Tool invocation |
| Renderer | Engine MCP profiler | CPU phase counters, GPU pass timings, GI workload, and triangle-BVH diagnostics tagged to completed frames | One-shot query or each sampled frame |
| Engine debug endpoint | [Windowing](windowing.md) | Queued request; pause state, completed frame ID, active camera, and output root | After the current frame or during paused event-loop waits |
| Engine | [Rendering](rendering.md) | Camera, selected `RenderDebugMode`, pixel coordinates, or grid coordinates | Capture or inspection request |
| Rendering | Engine | Render maps, grid metadata, ray-hit details, and read-back SDF/radiance values | GPU readback or scene query |
| Engine | MCP adapter | PNG/JSON payload or a bounded error response | Request completion |

## Verification and open questions

Profiling captures at most 120 frame samples in memory and returns them in the MCP response; it does not write a report file or change renderer settings. GPU timings are unavailable on adapters without timestamp-query support. Live profiling paths require rebuilding and restarting the running project before use. When MCP code or tool schemas change, restart the Codex session too; existing sessions keep their earlier discovered tools. Whole-grid region queries and bulk export remain future tools. Deterministic camera movement and waits remain available through [Debug Capture](capture.md).
