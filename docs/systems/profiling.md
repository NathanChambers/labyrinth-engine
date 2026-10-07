# Profiling

The renderer exposes CPU phase timings through `RendererPerformanceStats`. The debug panel displays light updates, instance and probe sample updates, probe evaluation, irradiance upload, probe budget, sample counts, and connected voxel regions. `frame_cpu_ms` measures the engine-side redraw path through submission; it does not include asynchronous GPU completion or display presentation time. When the adapter supports timestamp queries, reports also include GPU timings for shadow, scene, gizmo, and UI passes plus their total.

Rhai capture scripts can write a report beside screenshots:

```rhai
capture.wait(1.0);
capture.report("performance");
```

Reports are text files containing the measured CPU phases for the frame after the action is processed. The open-world capture script uses this path automatically. `examples/lighting/scripts/performance_matrix.rhai` applies four render-effect combinations and writes one report for each, making it possible to compare an unlit baseline with shadows and GI independently.

GPU work is grouped with wgpu debug markers for RenderDoc captures. The primary groups are directional and spot shadow passes, scene pass, gizmo pass, and UI pass. Timestamp results are resolved every frame but read back only when a scripted performance report is requested, avoiding a synchronization stall during ordinary frames. Adapters without timestamp support continue to report zero GPU timings.

The current irradiance implementation evaluates probe refreshes on the CPU. During rendering, the indirect irradiance lookup is evaluated per mesh vertex and interpolated across triangles instead of repeating the eight-corner lookup for every fragment. Its bounded update budget keeps CPU work predictable, while future clipmap and compute-shader work should move probe refresh and visibility sampling to the GPU.
