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
light-change bounds ──> per-field radiance refresh flags ──24-ray all-cell initialization, then six rays per cell per frame; distance-tiered in-frustum snapshots add four samples per channel; scene-change snapshots are coalesced to one per 60 frames──> radiance ray batches
world-cell-seeded hemisphere samples ──> 4 equal-weight initial rays per cardinal channel ──> 6 directional radiance channels ──24-ray all-cell initialization plus six-ray-per-cell-per-frame running-average refinement and additive camera-priority snapshots──> scene radiance field
selected field cell + ray/mode/overview options ──> selected-ray gizmo pass ──raw radiance + stored lobe + direction/count/blend/update diagnostics──> async readback ──> GI radiance popout
clicked viewport pixel ──> depth-prepass depth + world normal + surface ID ──> shared GPU candidate evaluator ──async weights and RGB contributions──> async readback ──> bounded CPU SDF and unexpanded scene-AABB segment checks ──> side-by-side visibility status and blocker bounds ──> GI Surface Inspector and diagnostic gizmos
panel header input ──click/drag──> shared UI window state ──collapse/position/size──> shared panels and Genos inspector
depth + normals + surface IDs ──fixed 32px screen lattice──> retained optional screen-space probe cache (updates disabled by default)
uniform radiance field + relocation validity ──trilinear + soft receiver-plane weighting ──SDF voxel + direct scene-AABB segment filters──> cosine-weighted six-lobe lighting ──> full-resolution raster GI
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
| [Application](../examples/lighting/src/main.rs) | [Rendering](systems/rendering.md) | Render settings, Lit/Unlit/Wire/GI Support display modes, selected radiance-field cell and ray, Geometry/Radiance mode, optional overview, cache freeze, temporal accumulation, and gizmo visibility | Each update |
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
| [Debug Capture](systems/capture.md) | [UI](systems/ui.md) | Scripted UI composition setting | Render-effects action |
| [Rendering](systems/rendering.md) | [Profiling](systems/profiling.md) | CPU phase timings and GPU debug markers | Each redraw |
| [Debug Capture](systems/capture.md) | [Profiling](systems/profiling.md) | Scripted performance report | Report action |
| [Application](../examples/lighting/src/main.rs) | [Irradiance volumes](systems/irradiance.md) | Volume bounds and probe resolution | Setup |
| [Application](../examples/genos/src/main.rs) | [Rendering](systems/rendering.md) | Setup-time scene radiance-field resolution | Before renderer initialization |
| [Windowing](systems/windowing.md) | [Irradiance volumes](systems/irradiance.md) | Active camera position used to snap authored irradiance clipmap bounds | Each redraw |
| [Irradiance volumes](systems/irradiance.md) | [Rendering](systems/rendering.md) | Debug gizmo line vertices | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Visibility-grid occupancy, region labels, and evaluated probe locations | Grid rebuild and probe refresh |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Active camera transform, scene instances, lights, and sky for the experimental conservative-voxel AABB GI and world-space radiance field; changed scene bounds re-anchor the static fields | Each redraw or scene/light update |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | All-cell 24-ray initialization and six-ray-per-cell-per-frame refinement; distance-tiered in-frustum snapshots add samples to the existing running average; explicit queued refresh work is capped at 16 cells per frame when refinement is disabled; radiance ray origins relocate to 0.35 m SDF clearance within 1.0 m, with original center, relocated origin, and offset diagnostics; includes occupancy-confirmed hit/material data, persistent fixed-direction lobes, and trace diagnostics | Scene/light change, static field update, camera motion with fixed bounds, raster shading, and optional diagnostics |
| [Irradiance volumes](systems/irradiance.md) | [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | Latest asynchronously read back raw and stored radiance, signed difference, update age, sampled directions, lobe resolved and attempted sample counts, blend factors, geometry diagnostics, and original/relocated probe positions for all 24 rays of the selected field cell; the inspector presents the selected ray by default | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Single field refresh queue, per-cell probe relocation and invalid-origin rejection, field-only trilinear interpolation with six cosine-weighted radiance lobes; no receiver-to-probe visibility filter currently; retained optional fixed-lattice surface-probe cache with deterministic slots, world-anchored history, movement-triggered refresh, and 3,000-probe budget | GI-enabled compute update, raster shading, and debug views |

The windowing system is the lifecycle coordinator. The renderer owns GPU resources and does not decide window mode. The example supplies an update callback that changes transforms before each redraw; there is no world state or scripting interaction yet. Any producer that can provide the mesh contract can use the same render path.
