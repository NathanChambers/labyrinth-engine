# Windowing

**Status:** Implemented  
**Owner:** `crates/engine`  
**Source:** [Window API and event loop](../../crates/engine/src/lib.rs), [Windows mode check](../../crates/engine/src/windows_fullscreen.rs)

## Purpose and boundary

Windowing owns the desktop event loop, window lifetime, initial size, display mode, and input shortcuts for the primitives example. It coordinates the application lifecycle, time update, scene synchronization, and rendering. It does not create GPU resources or generate geometry.

## State and invariants

`Runner` owns `WindowSettings`, the active `Window`, the active `Renderer`, the `RuntimeContext`, and any terminal error. The application object owns its object handles and behavior through the `Application` trait. Only one window and renderer are active. The public `WindowMode` values are windowed, borderless fullscreen, and exclusive fullscreen. The current mode in settings is updated when F11 cycles modes.

## Public contract

`run(settings, application)` starts the event loop. The application receives `setup`, `update`, and `cleanup` lifecycle hooks through the `Application` trait. `RuntimeContext` exposes the scene registry, active camera and light assignments, and global `Time`. Games spawn mesh, camera, and light entities, retain typed handles, mutate them through scene accessors, and choose the active render entities through `active_camera` and `active_lights`. `WindowSettings` selects title, logical size, mode, and graphics API.

F11 cycles windowed → borderless → exclusive → windowed. Escape and the close button exit. Exclusive mode chooses a reported video mode at the monitor's native size when possible, preferring larger area and higher refresh rate. It requests borderless fullscreen when no usable mode is available. On Windows, a display-mode test guards the exclusive request; on Wayland, exclusive requests use borderless because the window system does not support them.

## Lifecycle

The application creates its window and renderer on `resumed`. It drops both on `suspended`, allowing surface resources to be recreated on a later resume. Resize events update the renderer's surface and depth target. Redraw events render one frame. Initialization or rendering errors exit the event loop and return to the caller.

## Connections

| Other system | Direction | Data or command | When | Failure behavior |
| --- | --- | --- | --- | --- |
| [Rendering](rendering.md) | Windowing → rendering | Window, graphics API, mesh instances | Resume | Error exits the loop |
| [Math](math.md) | Windowing → math | Elapsed time and animation updates | Each redraw | Update changes are applied before rendering |
| [Rendering](rendering.md) | Windowing → rendering | Resize and draw | Window event | Draw error exits the loop |
| [Rendering](rendering.md) | Rendering → windowing | Initialization or draw error | Startup or redraw | Error returned from `run` |

## Verification and open questions

`cargo check --workspace --offline` passed on Windows. Windowed, borderless, and exclusive-request runs each initialized Vulkan and presented a first frame before the lifecycle refactor. The exclusive request fell back to borderless in the test environment. Pixels were not directly inspected. Native exclusive mode and Linux/macOS behavior still need checks on suitable displays.
