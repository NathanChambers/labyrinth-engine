struct TriangleBvhNode {
    minimum: vec4<f32>,
    maximum: vec4<f32>,
    metadata: vec4<u32>,
};

struct TriangleBvhTriangle {
    vertices: array<vec4<f32>, 3>,
    source: vec4<u32>,
    albedo: vec4<f32>,
    emission: vec4<f32>,
};

struct SdfUpdateSettings {
    minimum: vec4<f32>,
    voxel_size: vec4<f32>,
    counts: vec4<u32>,
    update: vec4<u32>,
    region_counts: vec4<u32>,
};

struct SdfDirtyRegion {
    origin: vec4<u32>,
    size: vec4<u32>,
};

struct SdfSurfaceSample {
    distance: f32,
    albedo: vec3<f32>,
    emission: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> settings: SdfUpdateSettings;

@group(0) @binding(1)
var<storage, read> bvh_nodes: array<TriangleBvhNode>;

@group(0) @binding(2)
var<storage, read> bvh_triangles: array<TriangleBvhTriangle>;

@group(0) @binding(3)
var<storage, read> dirty_regions: array<SdfDirtyRegion>;

@group(0) @binding(4)
var sdf_output: texture_storage_3d<rgba32float, write>;

@group(0) @binding(5)
var emission_output: texture_storage_3d<rgba32float, write>;

@group(0) @binding(6)
var sdf_input: texture_3d<f32>;

@group(0) @binding(7)
var clearance_output: texture_storage_3d<r32float, write>;

fn squared_distance_to_bounds(position: vec3<f32>, minimum: vec3<f32>, maximum: vec3<f32>) -> f32 {
    let offset = max(max(minimum - position, position - maximum), vec3<f32>(0.0));
    return dot(offset, offset);
}

fn closest_point_on_segment(position: vec3<f32>, start: vec3<f32>, end: vec3<f32>) -> vec3<f32> {
    let segment = end - start;
    let segment_length_squared = dot(segment, segment);
    var amount = 0.0;
    if segment_length_squared > 0.00000001 {
        amount = clamp(dot(position - start, segment) / segment_length_squared, 0.0, 1.0);
    }
    return start + segment * amount;
}

fn closest_point_on_triangle(position: vec3<f32>, first: vec3<f32>, second: vec3<f32>, third: vec3<f32>) -> vec3<f32> {
    let first_edge = second - first;
    let second_edge = third - first;
    if dot(cross(first_edge, second_edge), cross(first_edge, second_edge)) <= 0.00000001 {
        let first_candidate = closest_point_on_segment(position, first, second);
        let second_candidate = closest_point_on_segment(position, second, third);
        let third_candidate = closest_point_on_segment(position, third, first);
        let first_distance = distance(position, first_candidate);
        let second_distance = distance(position, second_candidate);
        let third_distance = distance(position, third_candidate);
        if first_distance <= second_distance && first_distance <= third_distance {
            return first_candidate;
        }
        if second_distance <= third_distance {
            return second_candidate;
        }
        return third_candidate;
    }
    let from_first = position - first;
    let first_dot = dot(first_edge, from_first);
    let second_dot = dot(second_edge, from_first);
    if first_dot <= 0.0 && second_dot <= 0.0 {
        return first;
    }
    let from_second = position - second;
    let third_dot = dot(first_edge, from_second);
    let fourth_dot = dot(second_edge, from_second);
    if third_dot >= 0.0 && fourth_dot <= third_dot {
        return second;
    }
    let first_edge_region = first_dot * fourth_dot - third_dot * second_dot;
    if first_edge_region <= 0.0 && first_dot >= 0.0 && third_dot <= 0.0 {
        return first + first_edge * (first_dot / (first_dot - third_dot));
    }
    let from_third = position - third;
    let fifth_dot = dot(first_edge, from_third);
    let sixth_dot = dot(second_edge, from_third);
    if sixth_dot >= 0.0 && fifth_dot <= sixth_dot {
        return third;
    }
    let second_edge_region = fifth_dot * second_dot - first_dot * sixth_dot;
    if second_edge_region <= 0.0 && second_dot >= 0.0 && sixth_dot <= 0.0 {
        return first + second_edge * (second_dot / (second_dot - sixth_dot));
    }
    let third_edge_region = third_dot * sixth_dot - fifth_dot * fourth_dot;
    if third_edge_region <= 0.0 && fourth_dot - third_dot >= 0.0 && fifth_dot - sixth_dot >= 0.0 {
        return second + (third - second) * ((fourth_dot - third_dot) / ((fourth_dot - third_dot) + (fifth_dot - sixth_dot)));
    }
    let inverse_area = 1.0 / (first_edge_region + second_edge_region + third_edge_region);
    let second_weight = second_edge_region * inverse_area;
    let third_weight = first_edge_region * inverse_area;
    return first + first_edge * second_weight + second_edge * third_weight;
}

fn nearest_surface(position: vec3<f32>, maximum_distance: f32) -> SdfSurfaceSample {
    var nearest_distance = maximum_distance;
    var nearest_albedo = vec3<f32>(0.0);
    var nearest_emission = vec3<f32>(0.0);
    if settings.counts.w == 0u {
        return SdfSurfaceSample(nearest_distance, nearest_albedo, nearest_emission);
    }
    var node_index = 0u;
    for (var visit = 0u; visit < settings.counts.w; visit++) {
        if node_index >= settings.counts.w {
            break;
        }
        let node = bvh_nodes[node_index];
        if squared_distance_to_bounds(position, node.minimum.xyz, node.maximum.xyz) > nearest_distance * nearest_distance {
            node_index = node.metadata.w;
            continue;
        }
        if node.metadata.z > 0u {
            for (var triangle_offset = 0u; triangle_offset < node.metadata.z; triangle_offset++) {
                let triangle_index = node.metadata.y + triangle_offset;
                if triangle_index >= settings.update.w {
                    continue;
                }
                let triangle = bvh_triangles[triangle_index];
                let closest = closest_point_on_triangle(position, triangle.vertices[0].xyz, triangle.vertices[1].xyz, triangle.vertices[2].xyz);
                let distance = length(position - closest);
                if distance < nearest_distance {
                    nearest_distance = distance;
                    nearest_albedo = triangle.albedo.rgb;
                    nearest_emission = triangle.emission.rgb;
                }
            }
        }
        node_index += 1u;
    }
    return SdfSurfaceSample(nearest_distance, nearest_albedo, nearest_emission);
}

fn dirty_voxel_coordinate(dirty_index: u32, first_region: u32, region_count: u32) -> vec3<u32> {
    var local_index = dirty_index;
    for (var region_offset = 0u; region_offset < region_count; region_offset++) {
        let region = dirty_regions[first_region + region_offset];
        let voxel_count = region.size.x * region.size.y * region.size.z;
        if local_index < voxel_count {
            let x = local_index % region.size.x;
            let y = (local_index / region.size.x) % region.size.y;
            let z = local_index / (region.size.x * region.size.y);
            return region.origin.xyz + vec3<u32>(x, y, z);
        }
        local_index -= voxel_count;
    }
    return vec3<u32>(0u);
}

@compute @workgroup_size(64)
fn update_voxels(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let dirty_index = invocation.x;
    if dirty_index >= settings.update.x {
        return;
    }

    let coordinate = dirty_voxel_coordinate(dirty_index, 0u, settings.update.z);
    let voxel_size = settings.voxel_size.xyz;
    let position = settings.minimum.xyz + (vec3<f32>(coordinate) + vec3<f32>(0.5)) * voxel_size;
    let half_voxel_diagonal = length(voxel_size) * 0.5;
    let surface = nearest_surface(position, 16.0 + half_voxel_diagonal);
    let surface_distance = surface.distance - half_voxel_diagonal;
    textureStore(sdf_output, vec3<i32>(coordinate), vec4<f32>(clamp(surface_distance, -16.0, 16.0), surface.albedo));
    textureStore(emission_output, vec3<i32>(coordinate), vec4<f32>(surface.emission, 0.0));
}

@compute @workgroup_size(64)
fn update_clearance(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let dirty_index = invocation.x;
    if dirty_index >= settings.update.y {
        return;
    }
    let coordinate = dirty_voxel_coordinate(dirty_index, settings.update.z, settings.region_counts.x);
    let resolution = settings.counts.xyz;
    var distance = 4u;
    for (var offset_z = -4; offset_z <= 4; offset_z += 1) {
        for (var offset_y = -4; offset_y <= 4; offset_y += 1) {
            for (var offset_x = -4; offset_x <= 4; offset_x += 1) {
                let neighbor = vec3<i32>(coordinate) + vec3<i32>(offset_x, offset_y, offset_z);
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
    textureStore(clearance_output, vec3<i32>(coordinate), vec4<f32>(safe_distance));
}
