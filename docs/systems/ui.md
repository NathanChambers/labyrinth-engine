# UI

**Status:** Implemented  
**Owner:** `crates/engine` and `crates/renderer`  
**Source:** [UI canvas](../../crates/engine/src/ui.rs), [UI pass](../../crates/renderer/src/lib.rs), [UI shader](../../crates/renderer/src/ui_shader.wgsl)

## Purpose and boundary

The UI system provides a screen-space canvas, anchored layout tree, auto-layout containers, and interactive controls. The engine owns element state, parent-relative layout resolution, pointer hit-testing, and input interaction. The renderer owns a second alpha-blended pass that draws the generated UI vertices after the 3D pass. Applications create the elements and consume their values.

## Public contract

`UiCanvas` owns a debug window layout, controls, labels, and typed containers. `UiLayout` resolves a size from a parent rectangle using an anchor, pivot, and offset; the default anchor and pivot are centered. `UiAutoLayout` provides row and column stacks. A zero size on the stack's main axis means the child expands into the remaining space after fixed children, padding, and spacing. The application owns each `FontAsset`, loads it from a project path, and assigns a clone to the UI; other systems can retain the same asset. Labels rasterize the selected font into screen-space quads; the renderer continues to draw them in the same alpha-blended overlay pass.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Application | Producer → UI | Layout tree, control definitions, and value reads | Setup and update |
| Input | Input → UI | Cursor position and left-button state | Each frame |
| UI | UI → rendering | Screen-space `UiVertex` values, including rasterized label quads | Each redraw |
| Rendering | UI → rendering | Alpha-blended overlay pass | After the scene pass |

The current demo uses one slider for light orbit angle, one for sphere movement speed, and a toggle button beside the light slider to pause or resume automatic rotation. The engine does not know what those values mean.
