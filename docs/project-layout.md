# Project layout

The workspace grows by working vertical slices. The graphics milestone currently has four engine packages and two example consumers:

```text
labyrinth-engine/
├── Cargo.toml
├── crates/
│   ├── engine/              # Public game-facing package: labyrinth, primitives, windowing
│   ├── math/                # Shared math value types and transforms
│   ├── mesh/                # Shared CPU mesh contract
│   ├── renderer/            # GPU rendering and readback
│   └── scripting/           # Rhai capture-plan compiler and capabilities
├── examples/
│   ├── primitives/          # Basic mesh, camera, lighting, and UI smoke test
│   ├── lighting/            # Outdoor/interior lighting experiment scene
│   └── genos/               # Genos-inspired building and animated-light stress scene
└── docs/
```

```text
math ──> renderer ──> engine ──> examples
math ──> engine
mesh ──> renderer ──> engine ──> examples
  └────────────────────────────> engine primitives
```

Arrows point from a dependency to its consumer. The public package owns window lifecycle, mode selection, and reusable primitive generation. The mesh package defines the data contract and the renderer owns GPU resources, graphics API selection, and the draw path. The example only composes primitives into a scene. See [windowing](systems/windowing.md), [mesh](systems/mesh.md), and [rendering](systems/rendering.md).

The directory names describe responsibilities without repeating the project name in every crate. An internal crate should receive a registry-specific name only if it later needs independent publication.

## Planned packages

The following boundaries remain proposals from the runtime, Rhai, modding, and AI tooling design:

| Package or contract | Intended responsibility |
| --- | --- |
| `world` | Authoritative entity and component state; validated commands and scoped queries |
| `runtime` | Fixed update sequencing and integration of world, scripts, and renderer |
| `interfaces/behavior` | Versioned Rhai host API exposed to mods |
| `interfaces/tooling` | Scoped editor and runtime context for agent tools |
| `tools/cli` | Project validation, targeted tests, and diagnostics |

The implemented scripting package currently compiles restricted debug capture plans. Broader gameplay scripting remains separate from the future world and runtime packages. Scripts and tools do not receive mutable world internals. The editor will own selection and viewport state and combine it with a read-only runtime projection. Stale revision checks should prevent a tool request from changing the wrong state.

Add asset, audio, physics, and editor packages when a working interaction needs them. Keep application-specific assets beside their application; `FontAsset` owns loading and sharing parsed font data without owning project files. Keep backend-specific implementations behind the rendering boundary. Avoid a general `common` or `types` crate that collects unrelated helpers.

## Workspace rules

- Use one Cargo workspace for engine packages and examples, with shared toolchain and formatting settings.
- Keep crate features additive. A backend choice must not change the meaning of game state.
- Give each implemented system a page in the [system catalog](systems/README.md) and record its interactions in the [system map](system-map.md).
- Keep game-specific assets and behavior out of engine crates. Examples should consume the public API.
- Version serialized formats and the mod interface explicitly when they are introduced.
