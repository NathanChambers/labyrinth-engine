# Mesh

**Status:** Implemented  
**Owner:** `crates/mesh`  
**Source:** [mesh contract](../../crates/mesh/src/lib.rs), [primitive generators](../../crates/engine/src/primitives.rs)

## Purpose and boundary

The mesh system defines the CPU geometry and texture contracts shared by geometry producers, asset importers, and the renderer. It contains vertex positions, normals, UV coordinates, colors, triangle indices, PBR material values, and optional base-color texture pixels. It does not load files, create GPU resources, or own scene entities.

## State and invariants

`Mesh::from_data` rejects indices outside the vertex array and incomplete triangles. Builder methods maintain valid indices as vertices and triangles are added. Geometry storage is private, so all supported geometry changes pass through those methods. Each mesh carries a geometry version. Clones share the same geometry and version; the first builder mutation on a shared clone copies the geometry, assigns it a new identity, and advances its revision. The renderer can detect replacement or mutation from this small token without scanning vertex and index contents each frame.

## Public contract

`MeshVertex` is a buffer-compatible value containing position, normal, UV coordinates, and RGBA color. `Mesh` exposes validated construction through `from_data`, incremental construction through `add_vertex`, `add_triangle`, and `add_quad`, read-only slices through `vertices` and `indices`, and a read-only `(identity, revision)` token through `geometry_version`. `Texture::from_rgba8` validates dimensions and stores shared RGBA8 pixels under a stable identity. `Material` carries scalar PBR values and an optional base-color texture.

## Lifecycle

Mesh values are created by a producer and retained inside `MeshInstance` values owned by the application. Cloning a mesh shares its geometry until either clone is mutated through a builder, at which point copy-on-write gives the mutated value independent geometry identity. Cloning a texture shares CPU pixels and preserves its texture identity. The renderer uploads geometry and base-color texture resources, batches instances that share both geometry and texture, and keeps instance transforms and PBR values in a separate GPU instance buffer. Transform changes update only affected instance records; geometry or texture changes rebuild draw-batch metadata. CPU consumers such as bounds and GI can continue to evaluate instance transforms against local mesh geometry.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Engine primitives | Producer → mesh | Plane, cube, and sphere geometry | Scene construction |
| [Asset import](asset-import.md) | Producer → mesh | Static triangle geometry, UVs, RGBA8 base-color pixels, and scalar PBR values | Asset loading |
| Rendering | Mesh → renderer | Shared local-space vertex/index/UV data, geometry and texture identities, and per-instance transforms/materials | Initialization and redraw |

## Verification and open questions

Mesh validation tests cover invalid indices and incomplete triangles. Primitive tests verify outward triangle winding. `cargo check --offline --workspace` compiles the mesh, importer, renderer, and FX example together. Automated regression coverage for texture sharing and imported-asset runtime loading remains open. Normal maps, metallic/roughness texture maps, alpha blending/cutouts, tangents, skinning, and topology streaming are not part of this contract yet.
