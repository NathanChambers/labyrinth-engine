# Debug Capture

**Status:** Implemented  
**Owner:** crates/engine with GPU readback in crates/renderer  
**Source:** [Capture coordinator](../../crates/engine/src/capture.rs), [renderer capture](../../crates/renderer/src/lib.rs)

## Purpose and boundary

Debug capture executes a compiled Rhai capture plan against the active camera. It owns action sequencing, timed camera movement, output path validation, PNG encoding, and completion state. The renderer owns GPU resources and returns RGBA pixels for a requested resolution.

Capture output is written below WindowSettings::debug_capture_root and the script's relative output directory. Parent traversal, absolute paths, and path separators in image names are rejected. Each screenshot writes the final RGBA image plus scene depth and only the shadow resources used by the scene's light types: directional cascades, the spot atlas, or point-shadow faces. All diagnostic output is PNG; point-light cube faces are emitted as six separate images. UI capture is represented in the script settings but is not implemented yet; scene-only output is the supported mode.

## Lifecycle

The host loads the script after application setup. On each redraw, the coordinator applies camera actions after the application update, so the debug plan has precedence while active. The renderer then presents the normal frame. Pending screenshots are read back at their requested resolution and written as PNG files. capture.exit() requests event-loop shutdown after pending capture work completes.

## Connections

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| [Scripting](scripting.md) | Capture | CapturePlan | After setup |
| [Windowing](windowing.md) | Capture | Active camera, scene, and frame delta | Each redraw |
| Capture | [Math](math.md) | Camera poses and interpolation | Camera actions |
| Capture | [Rendering](rendering.md) | Scripted render-setting overrides, including triangle-BVH visibility filtering | Each redraw after application update |
| Capture | [Rendering](rendering.md) | Custom-resolution capture request | After normal render |
| Capture | Filesystem | Validated PNG path, RGBA pixels, and depth buffer exports | Screenshot action |
| Capture | [Windowing](windowing.md) | Exit request | Plan completion |

## Verification and open questions

The lighting example's scripts/capture.rhai produced multiple 1280×720 PNG images and exits cleanly. Capture includes the scene, shadows, and irradiance probe lighting; UI output remains a planned extension. Scripts can call `capture.set_render_effects(...)`, `capture.set_shadow_quality(...)`, `capture.set_render_mode(...)`, and `capture.set_ui_enabled(...)` before waits or captures to isolate renderer paths, and `capture.report("performance")` writes the current renderer CPU phase timings beside the images.
Capture scripts can independently toggle ambient occlusion with capture.set_ambient_occlusion(bool), separate from direct shadows and irradiance.
CaptureRenderMode supports shadow_visibility for direct-light visibility diagnostics.
Capture scripts can toggle direct-light shadow multiplication with set_direct_light_visibility(bool).
Capture scripts can toggle the triangle-BVH visibility filter for AABB-SDF radiance-field sampling with set_triangle_probe_occlusion(bool); this override remains active for the rest of the plan and is applied after the application update on each redraw.
Capture scripts can enable opt-in triangle-BVH traversal counters with set_triangle_probe_diagnostics(bool). The next performance report reads segment, bounds-test, triangle-test, and hit counts from the rendered frame; counter collection is disabled by default and adds atomic operations when enabled.
Capture scripts can switch the triangle-BVH builder with set_triangle_probe_bvh_sah(bool). The Genos builder A/B script records five median-builder reports, five SAH-builder reports, then five median repeat reports in one run; the repeat helps show timing drift across the capture.

The renderer owns GPU readback staging and returns `CapturedFrame` with scene depth plus directional, spot, and point shadow depth layers. It leaves unused light-family layers empty, so a point-only scene does not produce directional or spot images. The engine capture coordinator owns filenames and encodes each depth layer as an occluder-presence grayscale PNG: cleared depth is black and nearer geometry is brighter, with a mild contrast curve. Scene-depth previews linearize using the active camera near/far clip range, improving contrast without changing the camera or renderer depth buffer. This keeps GPU resource ownership in rendering while making the relevant diagnostic depth surfaces available to offline analysis.
CaptureRenderMode supports gi_only for viewing probe irradiance without direct lighting, shadows, material albedo, or emission. This isolates whether shadowed receivers still receive indirect lighting.
