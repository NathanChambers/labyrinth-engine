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
active camera + GI mode ──> engine clipmap bounds ──> rendering
renderer visibility grid ──occupancy/regions──> probe placement and bounce filtering
active camera + instance AABBs ──> renderer toroidal conservative-occupancy + clearance clipmap ──voxel-confirmed visibility/material──> world-space radiance field compute pass
direct lights + sky + proxy emission ──hit radiance──> 6 directional lobes in near/far radiance fields
depth + normals + surface IDs ──fixed 32px screen lattice──> budgeted deterministic tile-owned probes ──4 stable SDF rays──> direct/emissive + coarse-field radiance
fixed screen-lattice neighborhood + near/far radiance fields ──compatibility/visibility weighted lookup──> full-resolution raster GI
SDF build/upload timings + radiance-field GPU timing/availability ──> renderer performance stats ──> lighting diagnostics
```

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| [Primitives example](../examples/primitives/src/main.rs) | [Windowing](systems/windowing.md) | Application implementation, entities, assignments, and window settings | Startup |
| [Engine primitives](systems/primitives.md) | [Mesh](systems/mesh.md) | Generated primitive geometry | On request |
| [Math](systems/math.md) | [Rendering](systems/rendering.md) | Camera matrix and vector operations | Initialization and resize |
| [Windowing](systems/windowing.md) | [Math](systems/math.md) | Frame delta and registered time scales | Each update |
| [Windowing](systems/windowing.md) | [Input](systems/input.md) | Keyboard, mouse, focus, and cursor events | Event loop |
| [Input](systems/input.md) | [Primitives example](../examples/primitives/src/main.rs) | Key state and mouse delta | Each update |
| [Input](systems/input.md) | [UI](systems/ui.md) | Cursor position and left-button state | Each frame |
| [Lighting example](../examples/lighting/src/main.rs) | [UI](systems/ui.md) | Inspector controls, collapsible drawer state, and container visibility | Setup and each update |
| [UI](systems/ui.md) | [Rendering](systems/rendering.md) | Screen-space overlay vertices | Each redraw |
| [Application](../examples/lighting/src/main.rs) | [Rendering](systems/rendering.md) | Render debug mode, shadow/GI settings, master gizmo visibility, independent global/screen GI gizmos, selected radiance-field cell, and SDF occupancy diagnostics | Each update |
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
| [Windowing](systems/windowing.md) | [Irradiance volumes](systems/irradiance.md) | Active camera position used to snap clipmap bounds | Each redraw |
| [Irradiance volumes](systems/irradiance.md) | [Rendering](systems/rendering.md) | Debug gizmo line vertices | Each redraw |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Visibility-grid occupancy, region labels, and evaluated probe locations | Grid rebuild and probe refresh |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Active camera transform, scene instances, lights, and sky for the experimental conservative-voxel AABB GI and world-space radiance field | Each redraw or scene/light update |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Occupancy-confirmed hit/material data, safe clearance values, persistent directional radiance-field samples, explicit trace termination diagnostics, and SDF occupancy boundary voxels | Clipmap rebuild, field update, raster shading, and optional diagnostics |
| [Rendering](systems/rendering.md) | [Irradiance volumes](systems/irradiance.md) | Fixed 32px screen lattice with deterministic tile-owned slots, world-anchored radiance history, movement-triggered refresh, bounded global-field refresh, and 3,000-probe budget | GI-enabled compute update and raster shading |

The windowing system is the lifecycle coordinator. The renderer owns GPU resources and does not decide window mode. The example supplies an update callback that changes transforms before each redraw; there is no world state or scripting interaction yet. Any producer that can provide the mesh contract can use the same render path.
