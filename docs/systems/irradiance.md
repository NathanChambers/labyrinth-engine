# Irradiance volumes

**Status:** Implemented budgeted CPU probe updates, static/dynamic surface sample caching, temporally blended probe refreshes, trilinear sampling, compact directional response, a coarse grid distance field, debug visualization, and runtime performance stats
**Owner:** `crates/engine` configuration and `crates/renderer` GPU drawing
**Source:** [gi.rs](../../crates/engine/src/gi.rs), [gizmo.rs](../../crates/engine/src/gizmo.rs), [renderer lib.rs](../../crates/renderer/src/lib.rs)

## Purpose and boundary

`IrradianceVolumes` owns app-created world-space volume descriptions. The application chooses volume bounds, probe resolution, and enabled state. The engine retains those descriptions and generates debug geometry for their bounds, rings, and probe locations. The renderer owns a persistent probe cache, bounded CPU updates, cached static and per-frame dynamic surface samples, a coarse cached grid distance field, compact directional probe data, trilinear shader sampling, and the GPU gizmo pass.

## State and invariants

Each volume has minimum and maximum world-space bounds and a three-dimensional probe resolution. Resolution components are clamped to at least one when debug probes are generated. Volumes should begin at or just above large occluders such as floors, never below them; a layer of probes below the floor can leak into floor interpolation and cause temporal popping. Disabled volumes produce no debug geometry. Each refreshed probe stores sky visibility in its alpha channel and a weighted dominant incoming direction in a second compact buffer. The renderer caches samples from static meshes until their transforms or materials change, rebuilds dynamic samples for moving meshes, voxelizes world-space mesh bounds, computes a low-resolution distance transform and connected free-space region labels, relocates probe sample positions and visibility-ray origins away from occupied cells, and sphere-traces five axis-aligned rays per refreshed probe. Directly lit and emissive surface samples use distance-weighted contributions, per-instance sample normalization, and coarse probe-to-surface visibility tests; samples from a different connected region are excluded so bounce does not pass through walls. Emissive contributions also add their source direction to the compact directional response. Distance-field rebuilds are throttled while geometry moves, and probe lighting continues on its bounded per-frame schedule. Refreshed values are blended into the cache to hide individual-probe update boundaries. Stale probes near the active camera are prioritized.

## Public contract

Applications call `context.irradiance_volumes.add(IrradianceVolumeDesc { ... })` during setup and retain the returned `IrradianceVolumeId` when they need to mutate the description. `RuntimeContext::irradiance_update_budget` controls the maximum number of probes refreshed per frame; the renderer defaults to eight updates per frame so CPU work stays bounded while applications can lower or raise the budget. `RuntimeContext::sky_lighting` controls the sky color and intensity used by refreshed probes. `RuntimeContext::performance_stats` exposes probe update time, visibility rebuild time, update counts, budget, and cached sample counts for diagnostic UI. `context.gizmos` exposes line, wire-box, ring, dot, and sampled mesh-normal helpers for additional diagnostics.

## Lifecycle

The engine creates the volume set before application setup. Applications add or mutate volumes during setup and update. Each redraw rebuilds the current debug geometry and uploads it to the renderer. The renderer preserves probe results between redraws, refreshes a configurable probe budget, and uploads the cached uniform. The volume descriptions are dropped with the runtime context.

## Connections

| Other system | Direction | Data or command | When | Failure behavior |
| --- | --- | --- | --- | --- |
| Application | Application → Irradiance volumes | Volume descriptions | Setup or update | Invalid handles return `None` |
| Irradiance volumes | Engine → Gizmos | Volume bounds and probe positions | Each redraw | Disabled volumes are skipped |
| Windowing | Engine → Rendering | Gizmo line vertices | Each redraw | Empty geometry skips the pass |
| Renderer | Engine → Application | `RendererPerformanceStats` | Each redraw | Values describe the previous completed probe update when read during application update |

## Verification and open questions

`cargo test --workspace --offline` and `cargo clippy --workspace --all-targets --offline -- -D warnings` verify the current implementation. The directional response is a compact dominant-direction approximation, and the distance field is derived from coarse axis-aligned bounds rather than triangle-accurate signed distances. Temporal accumulation, higher-quality SDF generation, directional sky sampling, and volume blending remain open implementation work.
