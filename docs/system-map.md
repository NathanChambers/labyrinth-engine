# System map

This map records relationships established in code. Broader gameplay scripting and editor tooling remain proposals in [project-layout.md](project-layout.md).

```text
primitives example ──entities/assignments──> windowing ──window/scene/backend──> rendering
control example ──empty scene + frame sampling──> windowing ──fallback camera/clear/present──> rendering
genos stress example ──building + animated boxes and lights──> scene ──selected camera/lights/meshes──> rendering
FX static-world example ──copied FBX assets──> asset importer ──meshes/materials/textures──> scene ──instances──> rendering
Sponza example ──FBX + sibling PBR maps──> asset importer ──meshes/materials/textures──> scene ──instances──> rendering
       │                                  ^                                      │
       └──uses──> engine primitives ──> mesh <────────────────────────────────────┘
                                          │
                                          └──future asset importers
math ──> engine, rendering
scripting ──> engine ──> renderer
irradiance volumes ──> gizmos ──> rendering
active scene geometry bounds + target cell spacing + padding ──> rounded per-axis SDF and radiance grids ──> rendering; out-of-bounds motion re-anchors and refreshes caches
renderer visibility grid ──occupancy/regions──> probe placement and bounce filtering
mesh triangles ──> renderer BVH nearest-surface queries ──conservative surface shell + clearance grid──> world-space radiance field compute pass
hard field invalidation ──> per-cell history reset queue ──> radiance region samples
active camera position ──> distance-ordered initial reset queue; camera-cell changes queue a nearby focus neighborhood
geometry changes ──> one coalesced whole-field dirty sweep; directional-light changes ──> one coalesced whole-field sweep; moving point/spot lights ──> bounded affected-cell priority queue
urgent local-light and camera-focus cells preempt pending resets; each consumes its reset once ──> remaining reset, dirty, and round-robin refinement queues ──> up to 1,024 rays and 32 cell invocations per frame──> radiance region samples
freeze control ──> pause radiance-field compute without consuming queued refresh work; refinement resumes round-robin when work budget remains──> radiance field
world-cell-seeded stratified sphere directions ──> equal-area 8/32/64 angular regions, each with its own running estimate ──bootstrap covers every region, then refinement schedules up to 64 rays per cell──> scene radiance field
transformed source triangles ──> renderer-built triangle BVH ──Render Options toggle──> GPU probe-segment visibility filter in radiance-field sampling
selected field cell + Geometry/Radiance mode ──> latest-sample gizmo pass ──up to eight trace records + 64 region radiance/sample-age records──> async readback ──> GI radiance inspector and freshness heatmap
clicked viewport pixel ──> depth-prepass depth + world normal + surface ID ──> shared GPU candidate evaluator + SDF voxel segment checks ──async weights and RGB contributions──> readback ──> renderer CPU exact transformed-triangle segment checks ──> per-probe mesh/SDF visibility comparison, hit triangle and hit geometry ──> GI Surface Inspector and diagnostic gizmos
panel header input ──click/drag──> shared UI window state ──collapse/position/size──> shared panels and Genos inspector
Cornell Box and Sponza examples ──DebugPanels──> shared display-mode, render-options, gizmo, and performance UI
Primitives, Lighting, Genos, and Sponza ──camera id + input + inspector key-capture──> shared FreeCamera ──transform──> active scene camera
F1 key press ──> UI visibility mode ──skip normal UI vertices and input; preserve marked FPS label──> rendering
windowing run ──initialize once──> logging ──UTC timestamp + level + target + key-values──> stderr or application-provided log backend
uniform angular-region radiance field + relocation validity ──trilinear + soft receiver-plane weighting ──SDF voxel + direct scene-AABB segment filters + optional source-triangle BVH segment filter──> cosine-weighted angular integration──> full-resolution raster GI
SDF build/upload timings + GPU per-cell sampled-region summaries ──30-frame async readback and CPU aggregation──> field coverage and scheduler performance stats ──> lighting diagnostics and capture reports
```

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| [Primitives example](../examples/primitives/src/main.rs) | [Windowing](systems/windowing.md) | Application implementation, entities, assignments, and window settings | Startup |
| [Primitives example](../examples/primitives/src/main.rs) | [Profiling](systems/profiling.md) | Timed debug/release frame interval and engine CPU samples for the animated primitives scene | Profile duration after two-second warmup |
| [Control example](../examples/control/src/main.rs) | [Windowing](systems/windowing.md) | Empty scene and optional timed debug/release frame sampling | Startup and each redraw |
| [Engine primitives](systems/primitives.md) | [Mesh](systems/mesh.md) | Generated primitive geometry | On request |
| [FX example](../examples/fx/src/main.rs) | [Asset import](systems/asset-import.md) | Copied terrain, building, and prop FBX files plus texture roots | Scene setup |
| [Sponza example](../examples/sponza/src/main.rs) | [Asset import](systems/asset-import.md) | Sponza FBX and adjacent downloaded material maps | Scene setup |
| [Asset import](systems/asset-import.md) | [Mesh](systems/mesh.md) | Static triangulated geometry, UVs, scalar PBR values, and decoded base-color, normal, metallic, and roughness pixels | Asset loading |
| [FX example](../examples/fx/src/main.rs) | [Mesh](systems/mesh.md) | Imported meshes and materials retained as static scene instances; repeated assets share geometry | Scene setup |
| [Math](systems/math.md) | [Rendering](systems/rendering.md) | Camera matrix and vector operations | Initialization and resize |
| [Windowing](systems/windowing.md) | [Math](systems/math.md) | Frame delta and registered time scales | Each update |
| [Windowing](systems/windowing.md) | [Input](systems/input.md) | Keyboard, mouse, focus, and cursor events; capture is cleared on suspension and shutdown | Event loop and teardown |
| [Windowing](systems/windowing.md) | [Logging](systems/logging.md) | Initializes the default structured logger before application setup | Each `run` call; an earlier application logger is preserved |
| [Rendering](systems/rendering.md) | [Logging](systems/logging.md) | Renderer startup and first-present records through the shared facade | Renderer creation and first presented frame |
| [Debug Capture](systems/capture.md) | [Logging](systems/logging.md) | Capture and performance-report output paths | After writing a capture or report |
| Applications | [Logging](systems/logging.md) | Level macros with messages and structured key-values | Whenever an application emits a record |
| [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | [Windowing](systems/windowing.md) | Suppresses cursor capture on scene clicks while surface picking is armed | Pick mode |
| [Input](systems/input.md) | [Primitives example](../examples/primitives/src/main.rs) | Key state and mouse delta | Each update |
| [Input](systems/input.md) | [UI](systems/ui.md) | Cursor position and left-button state | Each frame |
| [Windowing](systems/windowing.md) | [UI](systems/ui.md) | Apps start with reduced overlay mode; F1 toggles normal UI visibility and marked FPS overlay labels remain visible | Startup and key press; repeats ignored |
| [Shared debug panels](../crates/debug_ui/src/lib.rs) | [UI](systems/ui.md) | Reusable display mode, render options, gizmos, performance panel, and GI radiance inspector used by Lighting, Genos, and Primitives | Setup; control and inspector updates while normal UI is visible; FPS-only updates in F1 reduced mode; skipped when UI is disabled |
| [Primitives example](../examples/primitives/src/main.rs), [Lighting example](../examples/lighting/src/main.rs), [Genos stress example](../examples/genos/src/main.rs), [Sponza example](../examples/sponza/src/main.rs) | [Shared free camera](../crates/debug_ui/src/free_camera.rs) | Per-example orientation, captured mouse input, WASD/Space/C movement, Shift boost, and debug-inspector movement-key suppression | Setup and each application update |
| [Shared free camera](../crates/debug_ui/src/free_camera.rs) | [Scene](systems/scene.md) | Rotation and normalized flight translation applied to the selected camera | Each application update |
| [Shared debug panels](../crates/debug_ui/src/lib.rs) | [Rendering](systems/rendering.md) | `Triangle probe occlusion` render setting toggled from Render Options | Each UI update |
| [Shared debug panels](../crates/debug_ui/src/lib.rs) | [Irradiance volumes](systems/irradiance.md) | `Relocate GI probes` toggle updates `RuntimeContext::gi_radiance_probe_relocation_enabled`; defaults on | Each UI update while AABB-SDF is available |
| [RuntimeContext](../crates/engine/src/lib.rs) | [Rendering](systems/rendering.md) | Probe-relocation setting forwarded to the renderer; changing it invalidates the cached radiance field for a full refresh | Each redraw; refresh on setting change |
| [Mesh](systems/mesh.md) | [Rendering](systems/rendering.md) | Shared indexed triangles, per-triangle base color/emission, four PBR texture channels, per-instance transforms, normal transforms, materials, and surface IDs drive scene draws and triangle-surface SDF generation | Renderer initialization and redraw |
| [Rendering](systems/rendering.md) | [Mesh](systems/mesh.md) | Transformed indexed triangles flattened into a world-space BVH; transform-only instance changes refit leaf triangles and node bounds, while topology changes rebuild the tree | Renderer initialization, instance update, or GI sampling when enabled |
| [Primitives example](../examples/primitives/src/main.rs) | [Shared debug panels](../crates/debug_ui/src/lib.rs) | Render debug panels alongside the local lighting controls | Setup and each update |
| [Cornell Box example](../examples/cornell_box/src/main.rs) | [Shared debug panels](../crates/debug_ui/src/lib.rs) | Default display-mode, render-options, gizmo, and performance panels | Setup and each update |
| [Sponza example](../examples/sponza/src/main.rs) | [Shared debug panels](../crates/debug_ui/src/lib.rs) | Default display-mode, render-options, gizmo, and performance panels | Setup and each update |
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
| [Application](../examples/lighting/src/main.rs) | [Rendering](systems/rendering.md) | Render settings, Lit/Unlit/Wire/SDF Support display modes, selected radiance-field cell, Geometry/Radiance mode, cache freeze, temporal accumulation, and gizmo visibility | Each update; SDF Support requires enabled AABB-SDF GI |
| [Primitives](../examples/primitives/src/main.rs), [Lighting](../examples/lighting/src/main.rs), [Genos](../examples/genos/src/main.rs), [Cornell Box](../examples/cornell_box/src/main.rs), [Sponza](../examples/sponza/src/main.rs) | [Rendering](systems/rendering.md) | Select AABB-SDF as their GI mode; examples do not select authored volumes or clipmaps | Setup |
| [Genos stress example](../examples/genos/src/main.rs) | [Rendering](systems/rendering.md) | AABB-SDF GI mode and time-of-day directional sun controls | Setup and each update |
| [Genos capture script](../examples/genos/scripts/global_grid_diagnostic.rhai) | [Debug Capture](systems/capture.md) | Global-grid GI-only and lit scene screenshots | Capture run |
| [Genos stress example](../examples/genos/src/main.rs) | [Scene](systems/scene.md) | Building meshes, animated point-light and box transforms, and active light handles | Setup and each update |
| [Genos stress example](../examples/genos/src/main.rs) | [Mesh](systems/mesh.md) | One shared white cube geometry cloned across static and moving boxes; per-instance materials preserve their colors | Scene construction |
| [Application](../examples/lighting/src/main.rs) | [Windowing](systems/windowing.md) | Master gizmo visibility plus normals, lights, and global-probe toggles | Each redraw |
| [Mesh](systems/mesh.md) | [Rendering](systems/rendering.md) | Geometry version plus validated vertices and triangle indices | Initialization; version changes trigger renderer cache updates |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Window surface, assigned camera, assigned lights, mesh instances | Resume or startup |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Updated mesh instances, including topology changes | Each redraw; renderer skips unchanged instance data, uploads changed instance records, and rebuilds unique local-space geometry batches when topology changes |
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
| Application | [Rendering](systems/rendering.md) | Target radiance-field cell spacing in meters | Before renderer initialization |
| [Windowing](systems/windowing.md) | [Irradiance volumes](systems/irradiance.md) | Active camera position used to snap authored irradiance clipmap bounds | Each redraw |
| [Irradiance volumes](systems/irradiance.md) | [Rendering](systems/rendering.md) | Debug gizmo line vertices | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Visibility-grid occupancy, region labels, and evaluated probe locations | Grid rebuild and probe refresh |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Active camera transform prioritizes nearby queued SDF radiance cells when crossing a field-cell boundary; scene instances, lights, and sky update the experimental triangle-surface SDF GI; changed scene bounds re-anchor the static fields | Each redraw or scene/light update |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Equal-area 8/32/64 angular regions per cell, hard-reset and history-preserving queues, 16-cell/1,024-ray frame ceiling, variable ray batches, refinement cursor, relocation state, and latest trace data | Scene/light/camera change, field update, raster shading, and diagnostics |
| [Irradiance volumes](systems/irradiance.md) | [Shared radiance inspector](../crates/debug_ui/src/radiance_inspector.rs) | Up to eight asynchronously read-back traces for the selected cell plus 64 segment radiance, sample count, attempt count, and age records; field-wide coverage percentages refresh every 30 rendered frames | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Single field refresh queue, per-cell probe relocation, trilinear interpolation, cosine-weighted angular-region integration, receiver-plane heuristic, and optional triangle-BVH probe visibility | GI-enabled compute update, raster shading, and debug views |

The windowing system is the lifecycle coordinator. The renderer owns GPU resources and does not decide window mode. The example supplies an update callback that changes transforms before each redraw; there is no world state or scripting interaction yet. Any producer that can provide the mesh contract can use the same render path.
