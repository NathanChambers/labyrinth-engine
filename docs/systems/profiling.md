# Profiling

The renderer exposes CPU phase timings through `RendererPerformanceStats`. The debug panel displays overall frame CPU time, light and instance updates, probe updates, SDF field build and upload times, rebuilt SDF voxel count, radiance-field availability, and the latest reported GPU GI time. The GI radiance inspector can request a one-shot scene-pass GPU timing sample to inspect whole-scene GPU time. `frame_cpu_ms` measures the engine-side redraw path through submission; it does not include asynchronous GPU completion or display presentation time. When the adapter supports timestamp queries, reports include separate GPU timings for shadow, scene, world-space radiance-field update, gizmo, and UI passes plus their total.

Rhai capture scripts can write a report beside screenshots:

```rhai
capture.wait(1.0);
capture.report("performance");
```

Reports are text files containing the measured CPU phases for the frame after the action is processed. The open-world capture script uses this path automatically. `examples/lighting/scripts/performance_matrix.rhai` applies four render-effect combinations and writes one report for each, making it possible to compare an unlit baseline with shadows and GI independently.

`examples/genos/scripts/performance.rhai` reports the full scene, gizmos disabled, GI disabled, and GI plus shadows disabled. Run it with `cargo run --release -p genos -- --scale big --capture-script=examples/genos/scripts/performance.rhai` to compare the large stress scene; use `--dynamic 0 --boxes-still --sun-speed 0` to isolate static-scene and lighting cost. Reports are written to `captures/genos-performance/`.

GPU work is grouped with wgpu debug markers for RenderDoc captures. The primary groups are directional and spot shadow passes, world-space radiance-field compute, scene shading, gizmos, and UI. Timestamp results are resolved every frame and normally read back only for a scripted performance report. The shared GI inspector also provides an explicit one-shot timing sample; this blocks for the timestamp readback only when requested. Its `gpu_scene_ms` value covers the entire scene pass, so it reports whole-scene cost rather than the cost of a specific GI filter. The report includes CPU SDF build time (including proxy AABB preparation), CPU packing/enqueue time for texture uploads, the number of rebuilt SDF voxels, GPU radiance-field update time, and field availability (still exposed as `gi_history_valid_percent` for compatibility). A one-cell SDF scroll should report one exposed slab's voxel count; geometry changes or shifts at least as large as the SDF report a full rebuild. Adapters without timestamp support continue to report zero GPU timings.

Authored volumes and irradiance clipmaps evaluate probe refreshes on the CPU with a bounded per-frame budget. Their indirect lookup is evaluated per mesh vertex and interpolated across triangles. Experimental AABB-SDF mode instead updates one scene-sized world-space radiance field in a compute pass, then samples it per fragment. The configured cell counts were doubled on each axis, halving cell spacing for the same field extent; Receiver-to-probe visibility filtering has been removed; invalid relocation records are still excluded.
