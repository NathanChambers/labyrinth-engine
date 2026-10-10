# System map

This map records relationships established in code. Broader gameplay scripting and editor tooling remain proposals in [project-layout.md](project-layout.md).

```text
primitives example ──entities/assignments──> windowing ──window/scene/backend──> rendering
genos stress example ──building + animated boxes and lights──> scene ──selected camera/lights/meshes──> rendering
       │                                  ^                                      │
       └──uses──> engine primitives ──> mesh <────────────────────────────────────┘
                                          │
                                          └──future asset importers
math ──> engine, rendering
scripting ──> engine ──> renderer
irradiance volumes ──> gizmos ──> rendering
active scene geometry bounds ──> renderer scene-sized static SDF + uniform radiance field ──> rendering; scene-bound changes re-anchor and refresh caches
renderer visibility grid ──occupancy/regions──> probe placement and bounce filtering
instance AABBs ──> renderer fixed conservative-occupancy + clearance grid ──voxel-confirmed visibility/material──> world-space radiance field compute pass
light-change bounds ──> per-field radiance refresh flags ──two counter-sweeping rays per scheduled cell update, round-robin region coverage──> radiance region samples
world-cell-seeded stratified sphere directions ──> equal-area 8/32/64 angular regions, each with its own running estimate ──round-robin samples──> scene radiance field
transformed source triangles ──> renderer-built triangle BVH ──Render Options toggle──> GPU probe-segment visibility filter in radiance-field sampling
selected field cell + Geometry/Radiance mode ──> latest-sample gizmo pass ──two trace records + 64 region radiance/sample-age records──> async readback ──> GI radiance inspector and freshness heatmap
clicked viewport pixel ──> depth-prepass depth + world normal + surface ID ──> shared GPU candidate evaluator + SDF voxel segment checks ──async weights and RGB contributions──> readback ──> renderer CPU exact transformed-triangle segment checks ──> per-probe mesh/SDF visibility comparison, hit triangle and hit geometry ──> GI Surface Inspector and diagnostic gizmos
panel header input ──click/drag──> shared UI window state ──collapse/position/size──> shared panels and Genos inspector
depth + normals + surface IDs ──fixed 32px screen lattice──> retained optional screen-space probe cache (updates disabled by default)
uniform angular-region radiance field + relocation validity ──trilinear + soft receiver-plane weighting ──SDF voxel + direct scene-AABB segment filters + optional source-triangle BVH segment filter──> cosine-weighted angular integration──> full-resolution raster GI
SDF build/upload timings + radiance-field GPU timing/availability ──> renderer performance stats ──> lighting diagnostics
```

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| [Primitives example](../examples/primitives/src/main.rs) | [Windowing](systems/windowing.md) | Application implementation, entities, assignments, and window settings | Startup |
| [Engine primitives](systems/primitives.md) | [Mesh](systems/mesh.md) | Generated primitive geometry | On request |
| [Math](systems/math.md) | [Rendering](systems/rendering.md) | Camera matrix and vector operations | Initialization and resize |
| [Windowing](systems/windowing.md) | [Math](systems/math.md) | Frame delta and registered time scales | Each update |
| [Windowing](systems/windowing.md) | [Input](systems/input.md) | Keyboard, mouse, focus, and cursor events; capture is cleared on suspension and shutdown | Event loop and teardown |
| [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | [Windowing](systems/windowing.md) | Suppresses cursor capture on scene clicks while surface picking is armed | Pick mode |
| [Input](systems/input.md) | [Primitives example](../examples/primitives/src/main.rs) | Key state and mouse delta | Each update |
| [Input](systems/input.md) | [UI](systems/ui.md) | Cursor position and left-button state | Each frame |
| [Shared debug panels](../crates/debug_ui/src/lib.rs) | [UI](systems/ui.md) | Reusable display mode, render options, gizmos, performance panel, and GI radiance inspector used by Lighting, Genos, and Primitives | Setup and each update |
| [Shared debug panels](../crates/debug_ui/src/lib.rs) | [Rendering](systems/rendering.md) | `Triangle probe occlusion` render setting toggled from Render Options | Each UI update |
| [Rendering](systems/rendering.md) | [Mesh](systems/mesh.md) | Transformed indexed triangles flattened into a world-space BVH; transform-only instance changes refit leaf triangles and node bounds, while topology changes rebuild the tree | Renderer initialization, instance update, or GI sampling when enabled |
| [Primitives example](../examples/primitives/src/main.rs) | [Shared debug panels](../crates/debug_ui/src/lib.rs) | Render debug panels alongside the local lighting controls | Setup and each update |
| [Lighting example](../examples/lighting/src/main.rs) | [Shared debug panels](../crates/debug_ui/src/lib.rs) | Render debug panels with the GI radiance inspector launch button | Setup and each update |
| [Genos stress example](../examples/genos/src/main.rs) | [Shared debug panels](../crates/debug_ui/src/lib.rs) | Render debugging panels alongside the local stress-scene inspector | Setup and each update |
| [Lighting example](../examples/lighting/src/main.rs) | [UI](systems/ui.md) | Lighting controls and shared debug panel placement | Setup and each update |
| [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | [Scene](systems/scene.md) | Viewport ray from active camera and cursor position, intersected with transformed mesh triangles for surface picking | Pick click |
| [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | [Irradiance volumes](systems/irradiance.md) | Picked surface mapped to nearest cell center in the scene field; optional W/S, A/D, R/F cell stepping; one-shot GPU timing sample | Each update or explicit timing request |
| [Shared surface inspector](../crates/debug_ui/src/surface_inspector.rs) | [Rendering](systems/rendering.md) | Clicked framebuffer pixel requested for depth-prepass reconstruction and exact GPU radiance-candidate evaluation | Pick click |
| [Rendering](systems/rendering.md) | [Shared surface inspector](../crates/debug_ui/src/surface_inspector.rs) | Asynchronous surface point, normal, and eight shader-evaluated probe records: grid/relocated positions, validity, spatial weight, directional RGB/support, final share, and RGB contribution | After one-shot pick compute/readback |
| [Genos stress example](../examples/genos/src/main.rs) | [UI](systems/ui.md) | Local scene inspector registered as a movable, collapsible window while its controls remain project-specific | Setup and panel input |
| [Rendering](systems/rendering.md) | [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | Whole scene-pass GPU time through `RendererPerformanceStats::gpu_scene_ms` | After one-shot inspector sample or scripted report | Timestamp queries may be unavailable on the selected adapter |
| [Rendering](systems/rendering.md) | [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | Current scene field bounds and resolution through `RuntimeContext` | Each redraw |
| [Lighting example](../examples/lighting/src/main.rs) | [Scene](systems/scene.md) | Room-test point-light movement controls | Each application update |
| [UI](systems/ui.md) | [Rendering](systems/rendering.md) | Screen-space overlay vertices | Each redraw |
| [Application](../examples/lighting/src/main.rs) | [Rendering](systems/rendering.md) | Render settings, Lit/Unlit/Wire/GI Support display modes, selected radiance-field cell, Geometry/Radiance mode, cache freeze, temporal accumulation, and gizmo visibility | Each update |
| [Genos stress example](../examples/genos/src/main.rs) | [Rendering](systems/rendering.md) | AABB-SDF GI mode, global-grid-only diagnostic option, time-of-day directional sun controls, and independent screen-probe GI and gizmo visibility toggles | Setup and each update |
| [Genos capture script](../examples/genos/scripts/global_grid_diagnostic.rhai) | [Debug Capture](systems/capture.md) | Screen-probe-free global-grid GI-only and lit scene screenshots | Capture run |
| [Genos stress example](../examples/genos/src/main.rs) | [Scene](systems/scene.md) | Building meshes, animated point-light and box transforms, and active light handles | Setup and each update |
| [Application](../examples/lighting/src/main.rs) | [Windowing](systems/windowing.md) | Master gizmo visibility plus normals, lights, and global-probe toggles | Each redraw |
| [Mesh](systems/mesh.md) | [Rendering](systems/rendering.md) | Validated vertices and triangle indices | Renderer initialization and scene topology updates |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Window surface, assigned camera, assigned lights, mesh instances | Resume or startup |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Updated mesh instances, including topology changes | Each redraw; renderer skips unchanged instance data, updates changed vertex ranges, and rebuilds flattened buffers when topology changes |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Resize dimensions and draw requests | Window events |
| [Rendering](systems/rendering.md) | [Windowing](systems/windowing.md) | Initialization or draw failure | Startup or redraw |
| [Scripting](systems/scripting.md) | [Windowing](systems/windowing.md) | Compiled debug capture plan | After application setup |
| [Windowing](systems/windowing.md) | [Debug Capture](systems/capture.md) | Active camera, scene, and frame delta | Each redraw |
| [Debug Capture](systems/capture.md) | [Rendering](systems/rendering.md) | Custom-resolution scene readback | Screenshot action |
| [Debug Capture](systems/capture.md) | [Rendering](systems/rendering.md) | Scripted shadow, irradiance, and gizmo settings | Render-effects action |
| [Debug Capture](systems/capture.md) | [Rendering](systems/rendering.md) | Scripted triangle-BVH visibility-filter override and optional traversal diagnostic counters | Override each redraw; counters read back on performance report |
| [Debug Capture](systems/capture.md) | [Rendering](systems/rendering.md) | Scripted median or binned SAH triangle-BVH builder selection for same-run profiling | On strategy change; rebuilds the cached world-space triangle tree |
| [Debug Capture](systems/capture.md) | [UI](systems/ui.md) | Scripted UI composition setting | Render-effects action |
| [Rendering](systems/rendering.md) | [Profiling](systems/profiling.md) | CPU phase timings, GPU dirty-region SDF compute timing, and GPU debug markers | Each redraw |
| [Debug Capture](systems/capture.md) | [Profiling](systems/profiling.md) | Scripted performance report | Report action |
| [Application](../examples/lighting/src/main.rs) | [Irradiance volumes](systems/irradiance.md) | Volume bounds and probe resolution | Setup |
| [Application](../examples/genos/src/main.rs) | [Rendering](systems/rendering.md) | Setup-time scene radiance-field resolution | Before renderer initialization |
| [Windowing](systems/windowing.md) | [Irradiance volumes](systems/irradiance.md) | Active camera position used to snap authored irradiance clipmap bounds | Each redraw |
| [Irradiance volumes](systems/irradiance.md) | [Rendering](systems/rendering.md) | Debug gizmo line vertices | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Visibility-grid occupancy, region labels, and evaluated probe locations | Grid rebuild and probe refresh |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Active camera transform, scene instances, lights, and sky for the experimental conservative-voxel AABB GI and world-space radiance field; changed scene bounds re-anchor the static fields | Each redraw or scene/light update |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Equal-area 8/32/64 angular regions per cell, two counter-sweeping rays per scheduled cell update with round-robin coverage, relocation state, and latest trace data | Scene/light change, field update, raster shading, and diagnostics |
| [Irradiance volumes](systems/irradiance.md) | [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | Latest asynchronously read back trace for the selected cell plus 64 segment radiance, sample count, attempt count, and age records; sphere gizmo maps successful sample age from white to black; unresolved regions remain black | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Single field refresh queue, per-cell probe relocation, trilinear interpolation, cosine-weighted angular-region integration, receiver-plane heuristic, and optional triangle-BVH probe visibility; retained optional screen-probe cache | GI-enabled compute update, raster shading, and debug views |

The windowing system is the lifecycle coordinator. The renderer owns GPU resources and does not decide window mode. The example supplies an update callback that changes transforms before each redraw; there is no world state or scripting interaction yet. Any producer that can provide the mesh contract can use the same render path.
