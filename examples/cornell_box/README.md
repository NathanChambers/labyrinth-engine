# Cornell Box

A self-contained real-time Cornell Box inspired by the red/green wall scene in NVIDIA's VXGI presentation. It uses engine-generated meshes, so it does not need downloaded assets.

Run from the repository root:

```text
cargo run -p cornell-box -- --windowed
```

The scene contains the colored room, two boxes, two spheres, an emissive ceiling panel, and a small directional fill light. It starts with GI, triangle probe occlusion, and shadows disabled; AABB-SDF GI is configured for when you enable it. Ambient fill is zero. Use the shared Render Debug panel to toggle rendering effects, switch display modes, and inspect performance. Enable `Show gizmos` and `GI global probes` in the GIZMOS drawer to inspect the scene-wide radiance grid and its samples. AABB-SDF GI currently uses a single diffuse bounce; it is not a path tracer and has no tone-mapping pass. NVIDIA's reference image uses a more complete GI and specular-reflection pipeline, so this scene is a geometry and lighting setup to compare against rather than a pixel-identical reproduction.

Reference: [NVIDIA VXGI: Dynamic Global Illumination](https://developer.download.nvidia.com/assets/events/GDC15/GEFORCE/VXGI_Dynamic_Global_Illumination_GDC15.pdf), Cornell Box example, slides 11–16.
