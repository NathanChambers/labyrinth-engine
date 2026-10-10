# UI

**Status:** Implemented  
**Owner:** `crates/engine`, `crates/renderer`, and `crates/debug_ui`<br>
**Source:** [UI canvas](../../crates/engine/src/ui.rs), [shared debug panels](../../crates/debug_ui/src/lib.rs), [radiance inspector](../../crates/debug_ui/src/radiance_inspector.rs), [surface inspector](../../crates/debug_ui/src/surface_inspector.rs), [UI pass](../../crates/renderer/src/lib.rs), [UI shader](../../crates/renderer/src/ui_shader.wgsl)

## Purpose and boundary

The UI system provides a screen-space canvas, anchored layout tree, auto-layout containers, and interactive controls. The engine owns element state, parent-relative layout resolution, pointer hit-testing, and input interaction. The renderer owns a second alpha-blended pass that draws the generated UI vertices after the 3D pass. Applications create elements and consume their values; `crates/debug_ui` provides reusable application-level panels.

## Public contract

`UiCanvas` owns debug window layouts, independent panels, controls, labels, graphs, and typed containers. `UiLayout` resolves a size from a parent rectangle using an anchor, pivot, and offset; the default anchor and pivot are centered. `UiAutoLayout` provides row and column stacks. A zero size on the stack's main axis means the child expands into the remaining space after fixed children, padding, and spacing. Buttons center their text horizontally and vertically inside their hit rectangle by default; `UiCanvas::set_button_text_centered(button, false)` left-aligns text with an 8 px inset for header bars. The application owns each `FontAsset`, loads it from a project path, and assigns a clone to the UI. Labels and button text rasterize the selected font into screen-space quads, while graph elements generate a background quad and thick line-segment quads from application-provided samples.

Containers start visible. Applications can change visibility with `UiCanvas::set_container_visible`; hidden containers and their descendants are omitted from auto-layout, rendering, and pointer interaction. Standalone labels and graphs can be shown or hidden with `set_label_visible` and `set_graph_visible`, and panel layouts can be changed with `set_panel_layout`. `UiCanvas` exposes the last viewport size and whether the latest pointer press landed on a UI element, panel, graph, or window so tools can distinguish UI clicks from scene clicks.

`register_panel_window` and `register_window_panel` attach a header and content root to either an independent panel or the canvas window. A press on the header starts a drag; movement beyond 4 px moves the panel, header, and content together, clamped to the viewport. Releasing without dragging collapses or expands the content and resizes the background to its registered expanded height. `set_panel_window_expanded_height` supports dynamic content, and `set_panel_window_visible` hides a complete panel window. Header clicks are consumed as UI input, so they do not capture the scene cursor. Shared render and performance panels, the radiance inspector, the surface inspector, and Genos' local stress-scene panel use this behavior.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Application | Producer → UI | Layout tree, control definitions, container visibility, button style, and value reads | Setup and update |
| Input | Input → UI | Cursor position and left-button state | Each frame |
| Shared debug panels | UI → scene and radiance field | Pick request, selected cell, grid-step movement, and one-shot GPU timing sample | On click or while Move cell is active |
| Rendering | Rendering → UI | Current scene radiance-field bounds and resolution | Each redraw |
| Rendering | UI → Rendering | One-shot clicked-pixel request for the GI Surface Inspector | On scene pick |
| Rendering | Rendering → UI | GPU candidate values, SDF voxel checks, and CPU source-triangle visibility hits | After asynchronous readback |
| UI | UI → rendering | Screen-space `UiVertex` values, including text and graph quads | Each redraw |
| Rendering | UI → rendering | Alpha-blended overlay pass | After the scene pass |

All three example projects use the shared display mode, render options, gizmo controls, performance panel, GI radiance inspector, and GI Surface Inspector from `crates/debug_ui`. The radiance inspector exposes 8, 32, and 64 equal-area sphere regions, selected-cell navigation, angular coverage, and an opaque sphere freshness view where resolved regions turn white and fade to black over time; missed or unsampled regions remain black. Its ray panel shows both latest samples for the selected cell, with Geometry/Radiance modes and per-trace diagnostics. The panel reports sampled and fresh region counts; changing angular resolution clears the cache and starts a new field refresh. In Radiance mode, unrelated gizmos are hidden unless Show other gizmos is enabled.

GI Surface Inspector picking captures the exact viewport pixel, then reads the same depth, world normal, and surface ID from the depth prepass. A renderer compute entry point reconstructs the world position and evaluates the same trilinear candidate and cosine-weighted angular-region helpers used by final field shading. The asynchronous result contains all eight candidates' grid coordinates, relocated probe positions and validity, spatial weights, directional RGB and support, normalized final shares, RGB contributions, and the first occupied SDF voxel along each segment. After readback, the renderer checks each valid biased-surface-sample-to-probe segment against cached transformed source triangles and reports the closest hit's instance, triangle, distance, position, normal, barycentric coordinates, and vertices. The inspector compares mesh hits with SDF occupancy and shows ray status, hit triangle, hit normal, and SDF voxel in the viewport gizmos; the window reports per-probe visibility and highlights the highest-contribution candidate. Invalid probes remain excluded. The renderer owns the diagnostic result; the UI presents it without estimating visibility. The Render Options `Triangle probe occlusion` checkbox controls the GPU triangle-BVH filter used by radiance-field sampling; it is enabled by default. The current scene has one uniform scene-wide radiance field, so the inspector reports that field and does not expose a near/far selector.

Pick surface arms a one-click scene picker. It builds a screen ray from the active camera and viewport, intersects transformed scene mesh triangles, and chooses the closest cell center in the renderer-provided scene field bounds. UI clicks are excluded. Move cell enables grid navigation: W/S step along Z, A/D along X, and R/F along Y. While this mode is active, the shared examples suppress those keys from camera translation. Picking and movement mode are mutually exclusive.

Each project keeps its panel placement and project-specific controls local; Genos retains its stress-scene controls and its inspector uses the same draggable, collapsible window behavior. The engine does not interpret radiance diagnostics. The renderer owns field bounds and GPU-evaluated surface contributions; the shared inspectors present those values.

## Verification and open questions

`cargo check -p renderer -p labyrinth_debug_ui -p labyrinth --offline` passed for the new inspector data and presentation contract. The panel layout and gizmos were not visually exercised.
