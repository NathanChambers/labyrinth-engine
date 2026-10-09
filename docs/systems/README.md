# System catalog

Each implemented engine system gets one page named for its responsibility. Use [template.md](template.md) for the page structure. [The system map](../system-map.md) records implemented interactions; [the project layout](../project-layout.md) separates current packages from proposals.

| System | Owner | Status |
| --- | --- | --- |
| [Windowing](windowing.md) | `crates/engine` | Implemented |
| [Input](input.md) | `crates/engine` | Implemented |
| [UI](ui.md) | `crates/engine` | Implemented |
| [Scene](scene.md) | `crates/engine` | Implemented |
| [Math](math.md) | `crates/math` | Implemented |
| [Mesh](mesh.md) | `crates/mesh` | Implemented |
| [Primitives](primitives.md) | `crates/engine` | Implemented |
| [Rendering](rendering.md) | `crates/renderer` | Implemented |
| [Irradiance volumes](irradiance.md) | `crates/engine`, `crates/renderer` | Authored volumes, variable-density clipmap, and experimental AABB-SDF hybrid with a coarse world field and deterministic fixed-lattice surface probes |
| [Scripting](scripting.md) | `crates/scripting` | Implemented for debug capture |
| [Debug Capture](capture.md) | `crates/engine`, `crates/renderer` | Implemented |

System pages describe observed behavior and link to source paths. When a change alters ownership, a public contract, lifecycle, or cross-system flow, update the affected page and map together. World, broader gameplay scripting, runtime coordination, editor tooling, and project tools remain proposals in [project-layout.md](../project-layout.md).
