# Mesh

**Status:** Implemented  
**Owner:** `crates/mesh`  
**Source:** [mesh contract](../../crates/mesh/src/lib.rs), [primitive generators](../../crates/engine/src/primitives.rs)

## Purpose and boundary

The mesh system defines the CPU geometry and texture contracts shared by geometry producers, asset importers, and the renderer. It contains vertex positions, normals, UV coordinates, colors, triangle indices, PBR material values, and optional base-color, tangent-space normal, metallic, and roughness texture pixels. It does not load files, create GPU resources, or own scene entities.

## State and invariants

`Mesh::from_data` rejects indices outside the vertex array and incomplete triangles. Builder methods maintain valid indices as vertices and triangles are added. Geometry storage is private, so all supported geometry changes pass through those methods. Each mesh carries a geometry version. Clones share the same geometry and version; the first builder mutation on a shared clone copies the geometry, assigns it a new identity, and advances its revision. The renderer can detect replacement or mutation from this small token without scanning vertex and index contents each frame.

## Public contract

`MeshVertex` is a buffer-compatible value containing position, normal, UV coordinates, and RGBA color. `Mesh` exposes validated construction through `from_data`, incremental construction through `add_vertex`, `add_triangle`, and `add_quad`, read-only slices through `vertices` and `indices`, and a read-only `(identity, revision)` token through `geometry_version`. `Texture::from_rgba8` validates dimensions and stores shared RGBA8 pixels under a stable identity. `Material` carries scalar PBR values and optional base-color, normal, metallic, and roughness textures.

## Lifecycle

Mesh values are created by a producer and retained inside `MeshInstance` values owned by the application. Cloning a mesh shares its geometry until either clone is mutated through a builder, at which point copy-on-write gives the mutated value independent geometry identity. Cloning a texture shares CPU pixels and preserves its texture identity. The renderer uploads geometry and material textures, batches instances that share geometry and all four texture identities, and keeps instance transforms and PBR values in a separate GPU instance buffer. Transform changes update only affected instance records; geometry or material texture changes rebuild draw-batch metadata. CPU consumers such as bounds and GI can continue to evaluate instance transforms against local mesh geometry.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Engine primitives | Producer → mesh | Plane, cube, and sphere geometry | Scene construction |
| [Asset import](asset-import.md) | Producer → mesh | Static triangle geometry, UVs, RGBA8 PBR texture pixels, and scalar PBR values | Asset loading |
| Rendering | Mesh → renderer | Shared local-space vertex/index/UV data, geometry and four-channel texture identities, and per-instance transforms/materials | Initialization and redraw |

## Verification and open questions

Mesh validation tests cover invalid indices and incomplete triangles. Primitive tests verify outward triangle winding. `cargo check --offline -p mesh -p asset_import -p renderer -p sponza` compiles the four-channel material path. Normal maps use a tangent frame derived from world-position and UV derivatives in the fragment shader; alpha blending/cutouts, authored vertex tangents, skinning, and topology streaming remain outside this contract.
