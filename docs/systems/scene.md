# Scene

**Status:** Implemented  
**Owner:** `crates/engine`  
**Source:** [scene registry](../../crates/engine/src/scene.rs), [runtime assignments](../../crates/engine/src/lib.rs)

## Purpose and boundary

The scene registry owns renderable mesh, camera, and light values and returns typed handles for them. `RuntimeContext` owns which camera and lights are assigned to the active render pass. The scene is not yet the full gameplay entity/component world.

## Public contract

`spawn_mesh`, `spawn_camera`, `spawn_light`, `spawn_point_light`, and `spawn_spot_light` create typed handles. Applications retain those handles and mutate the associated values through typed accessors, including point- and directional-light accessors. `active_camera` selects one camera; `active_lights` selects the directional, point, and spot lights used for rendering.

## Connections

| Other system | Direction | Data or command | When |
| --- | --- | --- | --- |
| Application | Producer → scene | Meshes, cameras, and lights | Setup |
| Application | Producer → runtime | Active camera and light handles | Setup or update |
| Rendering | Scene → renderer | Selected camera, lights, and mesh instances | Resume and redraw |

The current handles are stable for the lifetime of their scene entries. Destruction, generational reuse, parent hierarchies, and gameplay components belong to the future world system.
