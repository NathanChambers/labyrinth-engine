# Input

**Status:** Implemented  
**Owner:** `crates/engine`  
**Source:** [input state](../../crates/engine/src/input.rs), [window event handling](../../crates/engine/src/lib.rs)

## Purpose and boundary

The input system translates window and device events into a frame-readable `InputState`. The engine owns event collection and cursor capture; applications read state and decide how it affects gameplay or tools.

## Public contract

`RuntimeContext::input` exposes key state through `is_key_down`, relative mouse movement through `mouse_delta`, and cursor capture through `cursor_captured` and `set_cursor_captured`. Mouse delta is cleared after each successful update and render. Escape releases capture and shows the cursor. Holding either Alt key releases the cursor for UI interaction; releasing Alt recaptures it. A left click recaptures the cursor when Alt is not held. Window focus loss also releases capture.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Windowing | Window events → input | Keyboard, mouse, focus, and cursor events | Event loop |
| Application | Input → application | Key state and mouse delta | Each update |
| Application | Application → input | Cursor capture request | Setup or update |

The current public key API uses winit `KeyCode` values. Game actions and rebinding should be added above this device layer later.
