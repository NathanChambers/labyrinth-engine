# Mesh

**Status:** Implemented  
**Owner:** `crates/mesh`  
**Source:** [mesh contract](../../crates/mesh/src/lib.rs), [primitive generators](../../crates/engine/src/primitives.rs)

## Purpose and boundary

The mesh system defines the CPU geometry contract shared by all geometry producers and the renderer. It contains vertex positions, normals, colors, and triangle indices. It does not load files, create GPU resources, or own scene entities.

## State and invariants

`Mesh::from_data` rejects indices outside the vertex array and incomplete triangles. Builder methods maintain valid indices as vertices and triangles are added. Geometry storage is private, so all supported geometry changes pass through those methods. Each mesh carries a geometry version; a clone receives a new identity, and adding vertices or triangles advances its revision. The renderer can detect replacement or mutation from this small token without scanning vertex and index contents each frame.

## Public contract

`MeshVertex` is a buffer-compatible value containing position, normal, and RGBA color. `Mesh` exposes validated construction through `from_data`, incremental construction through `add_vertex`, `add_triangle`, and `add_quad`, read-only slices through `vertices` and `indices`, and a read-only `(identity, revision)` token through `geometry_version`. Primitive generators and future FBX/OBJ importers should return this contract before handing data to rendering.

## Lifecycle

Mesh values are created by a producer and retained inside `MeshInstance` values owned by the application. Cloning a mesh creates an independent identity while copying its geometry and revision. Geometry builder calls advance that mesh's revision. The renderer borrows instances during initialization and can update transformed vertex data while the index topology remains unchanged. Asset ownership and topology replacement remain application responsibilities.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Engine primitives | Producer → mesh | Plane, cube, and sphere geometry | Scene construction |
| Future asset importers | Producer → mesh | Imported triangle geometry | Asset loading |
| Rendering | Mesh → renderer | Mesh geometry version, vertex/index data, and transforms | Initialization and redraw |

## Verification and open questions

Mesh validation tests cover invalid indices and incomplete triangles. Primitive tests verify outward triangle winding. Materials, tangents, skinning, submeshes, and topology streaming remain future work; transforms are carried by `MeshInstance`.
