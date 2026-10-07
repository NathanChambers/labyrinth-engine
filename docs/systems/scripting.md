# Scripting

**Status:** Implemented for debug capture scripts  
**Owner:** crates/scripting  
**Source:** [Capture script compiler](../../crates/scripting/src/lib.rs)

## Purpose and boundary

The scripting package compiles a restricted Rhai script into a data-only capture plan. It does not receive scene references, renderer handles, filesystem handles, or mutable engine state. The engine executes the resulting plan.

The current host API is intentionally small:

~~~rhai
capture.configure(1280, 720, "lighting", false);
capture.set_render_effects(false, true, false);
capture.set_camera([5.0, 3.0, 6.0], [0.0, 1.0, 0.0]);
capture.move_camera([7.0, 3.5, 3.0], [0.0, 1.0, 0.0], 1.5);
capture.wait(0.25);
capture.screenshot("view");
capture.sequence("orbit", 16, 0.25);
capture.exit();
~~~

Scripts are operation-limited. Camera, capture, and exit operations are capability-controlled when the compiler is given a restricted ScriptCapabilities value. The debug runner currently grants all three capabilities only when the host explicitly supplies a debug script.
`capture.set_render_effects(shadows, irradiance, gizmos)` applies independent renderer and debug-overlay switches before subsequent waits, captures, and reports.
`capture.set_render_mode("lit" | "unlit" | "wireframe")` selects the scene material/debug pass and remains active for the rest of the capture plan.
`capture.set_ui_enabled(enabled)` controls whether the runtime UI is generated and composited during the remaining capture actions, which is useful for isolating scene rendering costs.

## Lifecycle

The host reads and compiles a script after application setup. The compiler produces a CapturePlan; it does not execute actions during compilation. The engine advances that plan once per redraw and reports script, capture, or output errors through the normal application shutdown path.

## Connections

| Producer | Consumer | Data or command | When |
| --- | --- | --- | --- |
| Host settings | Scripting | Rhai source text and capabilities | After application setup |
| Scripting | Windowing | CapturePlan actions | Before redraw execution |
| Scripting | Capture | Camera, wait, screenshot, and exit commands | During plan compilation |

## Verification and open questions

cargo test -p scripting --offline verifies plan compilation, camera actions, waits, screenshots, and exit. General gameplay scripting, hot reload, and mod package loading remain future work.
