use std::collections::HashMap;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use math::{Color, Mat4, Transform, Vec3};
use mesh::{Material, Mesh, MeshVertex, Texture};

#[derive(Clone, Debug)]
pub struct ImportedMesh {
    pub name: String,
    pub mesh: Mesh,
    pub transform: Transform,
    pub material: Material,
}

pub fn load_fbx(path: impl AsRef<Path>) -> Result<Vec<ImportedMesh>, String> {
    let path = path.as_ref();
    let texture_roots = path.parent().map(Path::to_path_buf).into_iter().collect::<Vec<_>>();
    load_fbx_with_texture_roots(path, &texture_roots)
}

pub fn load_fbx_allow_missing_textures(path: impl AsRef<Path>) -> Result<Vec<ImportedMesh>, String> {
    let path = path.as_ref();
    let texture_roots = path.parent().map(Path::to_path_buf).into_iter().collect::<Vec<_>>();
    load_fbx_internal(path, &texture_roots, true)
}

pub fn load_fbx_with_texture_roots(path: impl AsRef<Path>, texture_roots: &[PathBuf]) -> Result<Vec<ImportedMesh>, String> {
    load_fbx_internal(path.as_ref(), texture_roots, false)
}

fn load_fbx_internal(path: &Path, texture_roots: &[PathBuf], allow_missing_textures: bool) -> Result<Vec<ImportedMesh>, String> {
    let path_string = path.to_str().ok_or_else(|| format!("model path is not valid UTF-8: {}", path.display()))?;
    let scene = ufbx::load_file(
        path_string,
        ufbx::LoadOpts {
            target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
            target_unit_meters: 1.0,
            generate_missing_normals: true,
            load_external_files: false,
            ignore_missing_external_files: true,
            ..Default::default()
        },
    )
    .map_err(|error| format!("failed to import {}: {}", path.display(), error.description))?;

    let mut texture_cache = HashMap::new();
    let mut file_texture_cache = HashMap::new();
    let mut imported_meshes = Vec::new();
    for node in &scene.nodes {
        let Some(source_mesh) = node.mesh.as_ref().map(AsRef::as_ref) else {
            continue;
        };
        let transform = convert_transform(node.geometry_to_world);
        for part in &source_mesh.material_parts {
            if part.num_triangles == 0 {
                continue;
            }
            let mut vertices = Vec::with_capacity(part.num_triangles * 3);
            let mut indices = Vec::with_capacity(part.num_triangles * 3);
            let mut triangulated_face = vec![0; source_mesh.max_face_triangles * 3];
            for &face_index in &part.face_indices {
                let face = source_mesh.faces[face_index as usize];
                let triangle_count = source_mesh.triangulate_face(&mut triangulated_face, face) as usize;
                for &corner_index in &triangulated_face[..triangle_count * 3] {
                    let position = source_mesh.vertex_position[corner_index as usize];
                    let normal = if source_mesh.vertex_normal.exists { source_mesh.vertex_normal[corner_index as usize] } else { ufbx::Vec3 { x: 0.0, y: 1.0, z: 0.0 } };
                    let tex_coord = if source_mesh.vertex_uv.exists { source_mesh.vertex_uv[corner_index as usize] } else { ufbx::Vec2 { x: 0.0, y: 0.0 } };
                    let color = if source_mesh.vertex_color.exists { source_mesh.vertex_color[corner_index as usize] } else { ufbx::Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 } };
                    let vertex_index = vertices.len() as u32;
                    vertices.push(MeshVertex {
                        position: [position.x as f32, position.y as f32, position.z as f32],
                        normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                        tex_coord: [tex_coord.x as f32, 1.0 - tex_coord.y as f32],
                        color: [color.x as f32, color.y as f32, color.z as f32, color.w as f32],
                    });
                    indices.push(vertex_index);
                }
            }
            if indices.is_empty() {
                continue;
            }

            let material = node.materials.get(part.index as usize).map(AsRef::as_ref).or_else(|| source_mesh.materials.get(part.index as usize).map(AsRef::as_ref));
            let material = import_material(material, path, texture_roots, &mut texture_cache, &mut file_texture_cache, allow_missing_textures)?;
            let mesh = Mesh::from_data(vertices, indices).map_err(|error| format!("{}: {error}", node.element.name))?;
            imported_meshes.push(ImportedMesh { name: node.element.name.as_ref().to_owned(), mesh, transform, material });
        }
    }

    if imported_meshes.is_empty() {
        return Err(format!("{} did not contain any triangle meshes", path.display()));
    }
    Ok(imported_meshes)
}

fn import_material(
    source: Option<&ufbx::Material>,
    model_path: &Path,
    texture_roots: &[PathBuf],
    texture_cache: &mut HashMap<u32, Texture>,
    file_texture_cache: &mut HashMap<PathBuf, Texture>,
    allow_missing_textures: bool,
) -> Result<Material, String> {
    let Some(source) = source else {
        return Ok(Material::default());
    };
    let color_map = if source.pbr.base_color.has_value { &source.pbr.base_color } else { &source.fbx.diffuse_color };
    let color = if color_map.has_value { color_map.value_vec4 } else { ufbx::Vec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 } };
    let texture_ref = source.pbr.base_color.texture.as_ref().or(source.fbx.diffuse_color.texture.as_ref()).map(|texture| &**texture);
    let base_color_texture = texture_ref.map(|texture| load_texture(texture, model_path, texture_roots, texture_cache, allow_missing_textures)).transpose()?.flatten();
    let normal_map = source.pbr.normal_map.texture.as_ref().or(source.fbx.normal_map.texture.as_ref()).map(|texture| &**texture);
    let roughness_map = source.pbr.roughness.texture.as_ref().map(|texture| &**texture);
    let metallic_map = source.pbr.metalness.texture.as_ref().map(|texture| &**texture);
    let normal_texture = load_material_map_texture(normal_map, texture_ref, "normal", model_path, texture_roots, texture_cache, file_texture_cache, allow_missing_textures)?;
    let roughness_texture = load_material_map_texture(roughness_map, texture_ref, "roughness", model_path, texture_roots, texture_cache, file_texture_cache, allow_missing_textures)?;
    let metallic_texture = load_material_map_texture(metallic_map, texture_ref, "metallic", model_path, texture_roots, texture_cache, file_texture_cache, allow_missing_textures)?;
    let roughness = if source.pbr.roughness.has_value {
        source.pbr.roughness.value_vec4.x as f32
    } else if roughness_texture.is_some() {
        1.0
    } else {
        0.7
    };
    let metallic = if source.pbr.metalness.has_value {
        source.pbr.metalness.value_vec4.x as f32
    } else if metallic_texture.is_some() {
        1.0
    } else {
        0.0
    };

    Ok(Material {
        base_color: Color::rgba(color.x as f32, color.y as f32, color.z as f32, color.w as f32),
        metallic,
        roughness,
        base_color_texture,
        normal_texture,
        metallic_texture,
        roughness_texture,
        ..Material::default()
    })
}

fn load_material_map_texture(
    map: Option<&ufbx::Texture>,
    base_color_map: Option<&ufbx::Texture>,
    map_kind: &str,
    model_path: &Path,
    texture_roots: &[PathBuf],
    texture_cache: &mut HashMap<u32, Texture>,
    file_texture_cache: &mut HashMap<PathBuf, Texture>,
    allow_missing_textures: bool,
) -> Result<Option<Texture>, String> {
    if let Some(texture) = map {
        if let Some(texture) = load_texture(texture, model_path, texture_roots, texture_cache, allow_missing_textures)? {
            return Ok(Some(texture));
        }
    }
    let Some(base_color_map) = base_color_map else {
        return Ok(None);
    };
    let Some(path) = resolve_conventional_texture(base_color_map, map_kind, texture_roots) else {
        return Ok(None);
    };
    let key = path.canonicalize().unwrap_or(path.clone());
    if let Some(texture) = file_texture_cache.get(&key) {
        return Ok(Some(texture.clone()));
    }
    let bytes = fs::read(&path).map_err(|error| format!("failed to read texture {}: {error}", path.display()))?;
    let filename = path.to_string_lossy();
    let (width, height, rgba8) = decode_texture(&bytes, &filename)?;
    let texture = Texture::from_rgba8(width, height, rgba8)?;
    file_texture_cache.insert(key, texture.clone());
    Ok(Some(texture))
}

fn resolve_conventional_texture(base_color_map: &ufbx::Texture, map_kind: &str, texture_roots: &[PathBuf]) -> Option<PathBuf> {
    let filename = Path::new(&*base_color_map.filename).file_name()?;
    let stem = Path::new(filename).file_stem()?.to_string_lossy();
    let extension = Path::new(filename).extension()?.to_string_lossy();
    let base_stem =
        ["_diffuse", "_albedo", "_basecolor", "_base_color"].into_iter().find_map(|suffix| stem.to_ascii_lowercase().strip_suffix(suffix).map(|_| stem[..stem.len() - suffix.len()].to_owned()))?;
    let mut base_stems = vec![base_stem.clone()];
    for color in ["_Red", "_Green", "_Blue"] {
        if let Some(shared_stem) = base_stem.strip_suffix(color) {
            base_stems.push(shared_stem.to_owned());
            break;
        }
    }
    let map_suffix = match map_kind {
        "normal" => ["normal", "Normal"],
        "roughness" => ["roughness", "Roughness"],
        "metallic" => ["metallic", "Metallic"],
        _ => return None,
    };
    for base_stem in base_stems {
        for suffix in map_suffix {
            let candidate = format!("{base_stem}_{suffix}.{extension}");
            for root in texture_roots {
                if let Some(path) = find_file_named(root, std::ffi::OsStr::new(&candidate)) {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn load_texture(texture: &ufbx::Texture, model_path: &Path, texture_roots: &[PathBuf], texture_cache: &mut HashMap<u32, Texture>, allow_missing_textures: bool) -> Result<Option<Texture>, String> {
    let texture_id = texture.element.element_id;
    if let Some(texture) = texture_cache.get(&texture_id) {
        return Ok(Some(texture.clone()));
    }

    let (bytes, filename) = if texture.content.is_empty() {
        let Some(texture_path) = resolve_texture_path(texture, model_path, texture_roots) else {
            if allow_missing_textures {
                return Ok(None);
            }
            return Err(format!("could not find texture {} referenced by {}", texture.filename, model_path.display()));
        };
        let bytes = fs::read(&texture_path).map_err(|error| format!("failed to read texture {}: {error}", texture_path.display()))?;
        (bytes, texture_path.to_string_lossy().into_owned())
    } else {
        (texture.content.as_ref().to_vec(), texture.filename.as_ref().to_owned())
    };
    let (width, height, rgba8) = decode_texture(&bytes, &filename)?;
    let texture_asset = Texture::from_rgba8(width, height, rgba8)?;
    texture_cache.insert(texture_id, texture_asset.clone());
    Ok(Some(texture_asset))
}

fn resolve_texture_path(texture: &ufbx::Texture, model_path: &Path, texture_roots: &[PathBuf]) -> Option<PathBuf> {
    let model_root = model_path.parent().unwrap_or_else(|| Path::new("."));
    let names = [&*texture.filename, &*texture.relative_filename, &*texture.absolute_filename];
    for name in names.into_iter().filter(|name| !name.is_empty()) {
        let requested_path = PathBuf::from(name);
        if requested_path.is_file() {
            return Some(requested_path);
        }
        for root in std::iter::once(model_root).chain(texture_roots.iter().map(PathBuf::as_path)) {
            let candidate = root.join(name.replace('\\', "/"));
            if candidate.is_file() {
                return Some(candidate);
            }
            let basename = requested_path.file_name()?;
            if let Some(found) = find_file_named(root, basename) {
                return Some(found);
            }
        }
    }
    None
}

fn find_file_named(root: &Path, filename: &std::ffi::OsStr) -> Option<PathBuf> {
    let requested_stem = Path::new(filename).file_stem()?;
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().is_some_and(|candidate| candidate.to_string_lossy().eq_ignore_ascii_case(&filename.to_string_lossy())) {
            return Some(path);
        }
        if path.is_file()
            && path.file_stem().is_some_and(|candidate| normalized_texture_stem(candidate) == normalized_texture_stem(requested_stem))
            && path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "png" | "psd" | "jpg" | "jpeg" | "tga"))
        {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file_named(&path, filename) {
                return Some(found);
            }
        }
    }
    None
}

fn normalized_texture_stem(stem: &std::ffi::OsStr) -> String {
    let stem = stem.to_string_lossy().to_ascii_lowercase();
    let Some(suffix_start) = stem.rfind(" (") else {
        return stem;
    };
    let suffix = &stem[suffix_start + 2..];
    if suffix.ends_with(')') && suffix[..suffix.len() - 1].chars().all(|character| character.is_ascii_digit()) { stem[..suffix_start].to_owned() } else { stem }
}

fn decode_texture(bytes: &[u8], filename: &str) -> Result<(u32, u32, Vec<u8>), String> {
    if filename.to_ascii_lowercase().ends_with(".psd") || bytes.starts_with(b"8BPS") {
        let psd = psd::Psd::from_bytes(bytes).map_err(|error| format!("failed to decode PSD texture {filename}: {error}"))?;
        return Ok((psd.width(), psd.height(), psd.rgba()));
    }

    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| format!("failed to decode texture {filename}: {error}"))?;
    let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut decoded).map_err(|error| format!("failed to decode texture {filename}: {error}"))?;
    let pixels = &decoded[..info.buffer_size()];
    let rgba8 = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|value| [*value, *value, *value, 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]]).collect(),
        png::ColorType::Indexed => return Err(format!("indexed PNG texture {filename} was not expanded")),
    };
    Ok((info.width, info.height, rgba8))
}

fn convert_transform(matrix: ufbx::Matrix) -> Transform {
    let matrix = Mat4::from_cols_array(&[
        matrix.m00 as f32,
        matrix.m10 as f32,
        matrix.m20 as f32,
        0.0,
        matrix.m01 as f32,
        matrix.m11 as f32,
        matrix.m21 as f32,
        0.0,
        matrix.m02 as f32,
        matrix.m12 as f32,
        matrix.m22 as f32,
        0.0,
        matrix.m03 as f32,
        matrix.m13 as f32,
        matrix.m23 as f32,
        1.0,
    ]);
    let (scale, rotation, position) = matrix.to_scale_rotation_translation();
    Transform::new(Vec3::new(position.x, position.y, position.z), rotation, scale)
}
