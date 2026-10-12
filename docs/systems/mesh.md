# Mesh

**Status:** Implemented  
**Owner:** `crates/mesh`  
**Source:** [mesh contract](../../crates/mesh/src/lib.rs), [primitive generators](../../crates/engine/src/primitives.rs)

## Purpose and boundary

The mesh system defines the CPU geometry contract shared by all geometry producers and the renderer. It contains vertex positions, normals, colors, and triangle indices. It does not load files, create GPU resources, or own scene entities.

## State and invariants

`Mesh::from_data` rejects indices outside the vertex array and incomplete triangles. Builder methods maintain valid indices as vertices and triangles are added. Geometry storage is private, so all supported geometry changes pass through those methods. Each mesh carries a geometry version. Clones share the same geometry and version; the first builder mutation on a shared clone copies the geometry, assigns it a new identity, and advances its revision. The renderer can detect replacement or mutation from this small token without scanning vertex and index contents each frame.

## Public contract

`MeshVertex` is a buffer-compatible value containing position, normal, and RGBA color. `Mesh` exposes validated construction through `from_data`, incremental construction through `add_vertex`, `add_triangle`, and `add_quad`, read-only slices through `vertices` and `indices`, and a read-only `(identity, revision)` token through `geometry_version`. Primitive generators and future FBX/OBJ importers should return this contract before handing data to rendering.

## Lifecycle

Mesh values are created by a producer and retained inside `MeshInstance` values owned by the application. Cloning a mesh shares its geometry until either clone is mutated through a builder, at which point copy-on-write gives the mutated value independent geometry identity. The renderer uploads each unique geometry once and keeps instance transforms and materials in a separate GPU instance buffer. Transform changes update only affected instance records; geometry mutations rebuild the relevant renderer geometry layout. CPU consumers such as bounds and GI can continue to evaluate the instance transform against local mesh geometry.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Engine primitives | Producer → mesh | Plane, cube, and sphere geometry | Scene construction |
| Future asset importers | Producer → mesh | Imported triangle geometry | Asset loading |
| Rendering | Mesh → renderer | Shared local-space vertex/index data, geometry version, and per-instance transforms/materials | Initialization and redraw |

## Verification and open questions

Mesh validation tests cover invalid indices and incomplete triangles. Primitive tests verify outward triangle winding. `cargo check --offline` and Vulkan startup of the Genos small scene exercised the shared-clone renderer path. Automated regression coverage for clone sharing and copy-on-write is still open. Materials, tangents, skinning, submeshes, and topology streaming remain future work; transforms and materials are carried by `MeshInstance`.
