# Math

**Status:** Implemented  
**Owner:** `crates/math`  
**Source:** [math types and `Transform`](../../crates/math/src/lib.rs)

## Purpose and boundary

The math package provides small, copyable value types used across engine systems. It reexports `glam` vectors, matrices, and quaternions and owns the engine-level `Transform` bundle.

## Public contract

`Color` is a compact RGBA value with white as its default and common color constants. `Camera` bundles a `Transform`, clear color, projection settings, and a `look_at` helper. `DirectionalLight` stores direction, color, and intensity. `Vec2`, `Vec3`, `Vec4`, `Mat2`, `Mat3`, `Mat4`, and `Quat` are the shared numerical types. `Transform` stores position, rotation, and scale directly as values with no trait objects or heap allocation. It provides identity and constructor helpers, matrix conversion, point/vector transformation, and in-place translation, rotation, and scale operations. `Easing`, `ease`, `lerp`, `lerp_vec3`, `slerp_quat`, and `ping_pong` provide bounded interpolation building blocks for animation and tools.

`Transform` represents semantic object state; `Mat4` is the derived matrix representation used when a consumer needs to transform or upload geometry. They are related but not interchangeable: the transform is authoritative state, while the matrix is a projection of that state.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Engine and game code | Consumer → math | Transform, vector, and time-scale values | Object or tool operations |
| Rendering | Consumer → math | Camera matrices and vectors | Initialization and resize |
| Mesh and future assets | Consumer → math | Positions and normals | Geometry creation |

The package intentionally contains no world ownership, GPU resources, or virtual dispatch. Future hierarchy composition can derive world matrices from local `Transform` values without changing this value type.
