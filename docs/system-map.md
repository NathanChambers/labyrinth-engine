# System map

This map records relationships established in code. Proposed runtime, scripting, and tooling boundaries are in [project-layout.md](project-layout.md).

```text
primitives example ──entities/assignments──> windowing ──window/scene/backend──> rendering
       │                                  ^                                      │
       └──uses──> engine primitives ──> mesh <────────────────────────────────────┘
                                          │
                                          └──future asset importers
math ──> engine, rendering
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
| [UI](systems/ui.md) | [Rendering](systems/rendering.md) | Screen-space overlay vertices | Each redraw |
| [Mesh](systems/mesh.md) | [Rendering](systems/rendering.md) | Validated vertices and triangle indices | Renderer initialization |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Window surface, assigned camera, assigned lights, mesh instances | Resume or startup |
| [Windowing](systems/windowing.md) | [Rendering](systems/rendering.md) | Resize dimensions and draw requests | Window events |
| [Rendering](systems/rendering.md) | [Windowing](systems/windowing.md) | Initialization or draw failure | Startup or redraw |

The windowing system is the lifecycle coordinator. The renderer owns GPU resources and does not decide window mode. The example supplies an update callback that changes transforms before each redraw; there is no world state or scripting interaction yet. Any producer that can provide the mesh contract can use the same render path.
