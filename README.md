# Labyrinth Engine

Labyrinth Engine is an early Rust game engine designed for games built by people and AI agents together. The first runnable slice opens a desktop window and renders a perspective scene with a white floor, red cube, green sphere, and diffuse lighting.

## Run the example

Install a current Rust toolchain and a graphics driver with Vulkan support, then run:

```text
cargo run -p primitives
```

The example starts windowed at 1280 × 720. Use `--borderless` or `--fullscreen` for borderless or exclusive fullscreen. `--windowed` explicitly selects windowed mode. Press **F11** to cycle modes and **Escape** to close the window.

Vulkan is the default graphics API. The renderer also accepts `--dx12`, `--opengl`, or `--metal` on platforms where those backends are available. macOS normally needs `--metal` unless a Vulkan portability implementation is installed. Exclusive fullscreen falls back to borderless when no usable mode is available, including on Wayland.

## Current boundaries

- `crates/engine` is the public `labyrinth` package. It owns the window event loop, lifecycle, and display mode.
- `crates/mesh` owns the stable CPU mesh contract shared by primitive generators, future asset importers, and the renderer.
- `crates/math` owns shared vector, matrix, quaternion, and transform value types.
- `crates/renderer` owns GPU setup, camera, depth buffer, shader, resize, presentation, and transformed mesh-instance uploads. It consumes geometry without knowing whether it came from primitives or imported assets.
- `crates/engine` exposes reusable plane, quad, cube, sphere, cylinder, cone, and torus generators.
- `examples/primitives` composes engine primitives into the demo scene and passes the resulting meshes through the public package.

The [project layout](docs/project-layout.md) separates current packages from proposed future ones. Implemented behavior is recorded in the [system catalog](docs/systems/README.md) and [system map](docs/system-map.md).

## Next directions

Rhai is the planned language for game behavior and untrusted mods. Mods will receive explicit host capabilities and execution limits. Tooling should let an editor share a scoped, versioned view of the selected scene area with an AI agent, then run a targeted test and return structured diagnostics. These systems are design intentions; they are not part of the graphics example yet.
