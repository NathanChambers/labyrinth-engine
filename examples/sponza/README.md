# Sponza Atrium

Loads `assets/meshes/Sponza.fbx` with the engine's FBX importer and keeps imported mesh transforms and materials. Base-color, normal, metallic, and roughness images from `assets/meshes/` are applied to the PBR materials. GI, triangle probe occlusion, and shadows start disabled. AABB-SDF GI is configured for when you enable it and currently provides one diffuse bounce. Use the shared Render Debug panel to toggle rendering effects, switch display modes, and inspect performance.

One public source with an FBX file is [TheRealMJP/DeferredTexturing](https://github.com/TheRealMJP/DeferredTexturing/blob/master/Content/Models/Sponza/Sponza.fbx) (8.63 MB). Its repository code is MIT-licensed, but check the model and texture terms in the repository before redistributing those assets. The official [Crytek Sponza Sample Scene](https://www.cryengine.com/asset-db/product/crytek/sponza-sample-scene) is free to add to a CRYENGINE account and is covered by the CRYENGINE Limited License Agreement. Khronos also documents a [Sponza glTF sample](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Sponza); this example currently accepts FBX only.

Click the scene to capture the mouse. Mouse movement looks around; W/A/S/D move, Space/C move vertically, and Shift moves faster. Press Escape to release the mouse for UI interaction.

The included FBX refers to external texture files. The corresponding maps from TheRealMJP repository are included under `assets/meshes/`; the importer also looks for conventional roughness and metallic sibling maps when the FBX has no explicit map slot. Missing files are skipped so the scene can still load with FBX material colors. Run from the repository root:

```text
cargo run -p sponza -- --windowed
```

To use a different FBX, pass `--model="C:/path/to/Sponza.fbx"`. The importer searches the model folder recursively for external textures. Camera framing is set for this Sponza export; adjust it in `src/main.rs` if you substitute a file with different units or axes.
