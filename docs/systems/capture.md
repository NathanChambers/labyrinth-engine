# Debug Capture

**Status:** Implemented  
**Owner:** crates/engine with GPU readback in crates/renderer  
**Source:** [Capture coordinator](../../crates/engine/src/capture.rs), [renderer capture](../../crates/renderer/src/lib.rs)

## Purpose and boundary

Debug capture executes a compiled Rhai capture plan against the active camera. It owns action sequencing, timed camera movement, output path validation, PNG encoding, and completion state. The renderer owns GPU resources and returns RGBA pixels for a requested resolution.

Capture output is written below WindowSettings::debug_capture_root and the script's relative output directory. Parent traversal, absolute paths, and path separators in image names are rejected. UI capture is represented in the script settings but is not implemented yet; scene-only output is the supported mode.

## Lifecycle

The host loads the script after application setup. On each redraw, the coordinator applies camera actions after the application update, so the debug plan has precedence while active. The renderer then presents the normal frame. Pending screenshots are read back at their requested resolution and written as PNG files. capture.exit() requests event-loop shutdown after pending capture work completes.

## Connections

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| [Scripting](scripting.md) | Capture | CapturePlan | After setup |
| [Windowing](windowing.md) | Capture | Active camera, scene, and frame delta | Each redraw |
| Capture | [Math](math.md) | Camera poses and interpolation | Camera actions |
| Capture | [Rendering](rendering.md) | Custom-resolution capture request | After normal render |
| Capture | Filesystem | Validated PNG path and RGBA pixels | Screenshot action |
| Capture | [Windowing](windowing.md) | Exit request | Plan completion |

## Verification and open questions

The lighting example's scripts/capture.rhai produced multiple 1280×720 PNG images and exits cleanly. Capture includes the scene, shadows, and irradiance probe lighting; UI output remains a planned extension. Scripts can call `capture.set_render_effects(...)`, `capture.set_render_mode(...)`, and `capture.set_ui_enabled(...)` before waits or captures to isolate renderer paths, and `capture.report("performance")` writes the current renderer CPU phase timings beside the images.
