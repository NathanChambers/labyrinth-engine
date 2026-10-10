struct SdfGeometry {
    minimum: vec4<f32>,
    maximum: vec4<f32>,
    albedo: vec4<f32>,
    emission: vec4<f32>,
};

struct SdfUpdateSettings {
    minimum: vec4<f32>,
    voxel_size: vec4<f32>,
    counts: vec4<u32>,
    update: vec4<u32>,
};

@group(0) @binding(0)
var<uniform> settings: SdfUpdateSettings;

@group(0) @binding(1)
var<storage, read> geometries: array<SdfGeometry>;

@group(0) @binding(2)
var<storage, read> dirty_indices: array<u32>;

@group(0) @binding(3)
var sdf_output: texture_storage_3d<rgba32float, write>;

@group(0) @binding(4)
var emission_output: texture_storage_3d<rgba32float, write>;

@group(0) @binding(5)
var sdf_input: texture_3d<f32>;

@group(0) @binding(6)
var clearance_output: texture_storage_3d<r32float, write>;

fn signed_distance_to_bounds(position: vec3<f32>, minimum: vec3<f32>, maximum: vec3<f32>) -> f32 {
    let center = (minimum + maximum) * 0.5;
    let half_extent = (maximum - minimum) * 0.5;
    let offset = abs(position - center) - half_extent;
    return length(max(offset, vec3<f32>(0.0))) + min(max(offset.x, max(offset.y, offset.z)), 0.0);
}

@compute @workgroup_size(64)
fn update_voxels(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let dirty_index = invocation.x;
    if dirty_index >= settings.update.x {
        return;
    }

    let linear_index = dirty_indices[dirty_index];
    let resolution = settings.counts.xyz;
    let x = linear_index % resolution.x;
    let y = (linear_index / resolution.x) % resolution.y;
    let z = linear_index / (resolution.x * resolution.y);
    let coordinate = vec3<u32>(x, y, z);
    let voxel_size = settings.voxel_size.xyz;
    let voxel_padding = voxel_size * 0.5;
    let position = settings.minimum.xyz + (vec3<f32>(coordinate) + vec3<f32>(0.5)) * voxel_size;

    var nearest_distance = 16.0;
    var nearest_albedo = vec3<f32>(0.0);
    var nearest_emission = vec3<f32>(0.0);
    for (var geometry_index = 0u; geometry_index < settings.counts.w; geometry_index += 1u) {
        let geometry = geometries[geometry_index];
        let distance = signed_distance_to_bounds(position, geometry.minimum.xyz - voxel_padding, geometry.maximum.xyz + voxel_padding);
        if distance < nearest_distance {
            nearest_distance = distance;
            nearest_albedo = geometry.albedo.xyz;
            nearest_emission = geometry.emission.xyz;
        }
    }

    textureStore(sdf_output, vec3<i32>(coordinate), vec4<f32>(clamp(nearest_distance, -16.0, 16.0), nearest_albedo));
    textureStore(emission_output, vec3<i32>(coordinate), vec4<f32>(nearest_emission, 0.0));
}

@compute @workgroup_size(64)
fn update_clearance(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let dirty_index = invocation.x;
    if dirty_index >= settings.update.y {
        return;
    }
    let linear_index = dirty_indices[settings.update.z + dirty_index];
    let resolution = settings.counts.xyz;
    let x = linear_index % resolution.x;
    let y = (linear_index / resolution.x) % resolution.y;
    let z = linear_index / (resolution.x * resolution.y);
    var distance = 4u;
    for (var offset_z = -4; offset_z <= 4; offset_z += 1) {
        for (var offset_y = -4; offset_y <= 4; offset_y += 1) {
            for (var offset_x = -4; offset_x <= 4; offset_x += 1) {
                let neighbor = vec3<i32>(i32(x) + offset_x, i32(y) + offset_y, i32(z) + offset_z);
                if any(neighbor < vec3<i32>(0)) || any(neighbor >= vec3<i32>(resolution)) {
                    continue;
                }
                if textureLoad(sdf_input, neighbor, 0).r <= 0.0 {
                    distance = min(distance, u32(max(abs(offset_x), max(abs(offset_y), abs(offset_z)))));
                }
            }
        }
    }
    let half_voxel_diagonal = length(settings.voxel_size.xyz) * 0.5;
    let safe_distance = max(f32(distance) * min(settings.voxel_size.x, min(settings.voxel_size.y, settings.voxel_size.z)) - half_voxel_diagonal, 0.0);
    textureStore(clearance_output, vec3<i32>(i32(x), i32(y), i32(z)), vec4<f32>(safe_distance));
}
