# Primitives

**Status:** Implemented  
**Owner:** `crates/engine`  
**Source:** [primitive generators](../../crates/engine/src/primitives.rs)

## Purpose and boundary

The primitives module provides reusable procedural meshes for common modeling shapes. It is part of the public engine facade, so games, tools, tests, and examples can create geometry without duplicating topology code.

## Public contract

The module currently provides `plane`, `quad`, `cube`, `sphere`, `cylinder`, `cone`, and `torus`. Each function requires a `Color`, making the temporary per-vertex color explicit; callers use `Color::WHITE` when no tint is needed. Each function returns a `mesh::Mesh` with positions, normals, vertex colors, and triangle indices. Resolution parameters control radial and surface tessellation where applicable. A future material system can replace vertex color storage without changing primitive topology.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Mesh | Primitives → mesh | Generated vertices and triangles | Primitive request |
| Example or game | Consumer → primitives | Dimensions, resolution, and color | Scene construction |
| Rendering | Mesh → renderer | Resulting mesh values | Renderer initialization |

Primitive generation does not allocate GPU resources or choose materials. Imported assets should produce the same `Mesh` contract, so they can share the renderer path.
