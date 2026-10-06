# Mesh

**Status:** Implemented  
**Owner:** `crates/mesh`  
**Source:** [mesh contract](../../crates/mesh/src/lib.rs), [primitive generators](../../crates/engine/src/primitives.rs)

## Purpose and boundary

The mesh system defines the CPU geometry contract shared by all geometry producers and the renderer. It contains vertex positions, normals, colors, and triangle indices. It does not load files, create GPU resources, or own scene entities.

## State and invariants

`Mesh::from_data` rejects indices outside the vertex array and incomplete triangles. Builder methods maintain valid indices as vertices and triangles are added. A mesh can be cloned or passed by shared reference without exposing its storage for mutation.

## Public contract

`MeshVertex` is a buffer-compatible value containing position, normal, and RGBA color. `Mesh` exposes validated construction through `from_data`, incremental construction through `add_vertex`, `add_triangle`, and `add_quad`, and read-only slices through `vertices` and `indices`. Primitive generators and future FBX/OBJ importers should return this contract before handing data to rendering.

## Lifecycle

Mesh values are created by a producer and retained inside `MeshInstance` values owned by the application. The renderer borrows instances during initialization and can update transformed vertex data while the index topology remains unchanged. Asset ownership and topology replacement remain application responsibilities.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Engine primitives | Producer → mesh | Plane, cube, and sphere geometry | Scene construction |
| Future asset importers | Producer → mesh | Imported triangle geometry | Asset loading |
| Rendering | Mesh → renderer | MeshInstance geometry and transforms | Initialization and redraw |

## Verification and open questions

Mesh validation tests cover invalid indices and incomplete triangles. Primitive tests verify outward triangle winding. Materials, tangents, skinning, submeshes, and topology streaming remain future work; transforms are carried by `MeshInstance`.
