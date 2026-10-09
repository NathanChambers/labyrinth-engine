struct Camera {
    view_projection: mat4x4<f32>,
};

struct SdfClipmapData {
    minimum: vec4<f32>,
    maximum: vec4<f32>,
    resolution: vec4<u32>,
    settings: vec4<u32>,
    grid_offset: vec4<u32>,
    sky_radiance: vec4<f32>,
};

struct SelectedRadianceCell {
    coordinate_and_level: vec4<u32>,
};

struct RadianceFieldData {
    minimum: vec4<f32>,
    maximum: vec4<f32>,
    resolution: vec4<u32>,
    grid_offset: vec4<u32>,
    scroll_shift: vec4<i32>,
    settings: vec4<u32>,
};

struct RadianceFieldValues {
    values: array<vec4<f32>>,
};

struct RadianceFieldTraceDebug {
    probe_position: vec4<f32>,
    hit_position: vec4<f32>,
    hit_normal_distance: vec4<f32>,
    direction_hit: vec4<f32>,
};

struct RadianceFieldTraceDebugValues {
    values: array<RadianceFieldTraceDebug>,
};

struct SurfaceProbe {
    position_confidence: vec4<f32>,
    normal_age: vec4<f32>,
    irradiance: vec4<f32>,
    lock: u32,
    next: u32,
    sample_reason: u32,
    padding: u32,
};

struct SurfaceProbeValues {
    values: array<SurfaceProbe>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(0) @binding(8)
var sdf_texture: texture_3d<f32>;

@group(0) @binding(9)
var<uniform> sdf_clipmap: SdfClipmapData;

@group(0) @binding(11)
var<uniform> near_radiance_field: RadianceFieldData;

@group(0) @binding(12)
var<storage, read> near_radiance_values: RadianceFieldValues;

@group(0) @binding(13)
var<uniform> far_radiance_field: RadianceFieldData;

@group(0) @binding(14)
var<storage, read> far_radiance_values: RadianceFieldValues;

@group(0) @binding(16)
var<storage, read> surface_probes: SurfaceProbeValues;

@group(0) @binding(20)
var<storage, read> radiance_field_trace_debug: RadianceFieldTraceDebugValues;

@group(0) @binding(21)
var<uniform> selected_radiance_cell: SelectedRadianceCell;

const SURFACE_PROBE_CAPACITY: u32 = 131072u;
const RADIANCE_FIELD_DIRECTIONS: u32 = 6u;
const LIGHTING_KIND_UNLIT: u32 = 0u;
const LIGHTING_KIND_DIRECT: u32 = 1u;
const LIGHTING_KIND_SHADOWED: u32 = 2u;
const LIGHTING_KIND_EMISSIVE: u32 = 3u;
const LIGHTING_KIND_SKY: u32 = 4u;
const TRACE_TERMINATION_HIT: u32 = 1u;
const TRACE_TERMINATION_MAX_STEPS: u32 = 2u;
const TRACE_TERMINATION_MAX_DISTANCE: u32 = 3u;
const TRACE_TERMINATION_GRID_EXIT: u32 = 4u;

fn field_cell_index(coordinate: vec3<u32>, resolution: vec3<u32>) -> u32 {
    return coordinate.x + coordinate.y * resolution.x + coordinate.z * resolution.x * resolution.y;
}

fn display_radiance(radiance: vec3<f32>) -> vec3<f32> {
    return clamp(radiance / (radiance + vec3<f32>(0.3)), vec3<f32>(0.06), vec3<f32>(1.0));
}

fn lighting_kind_color(kind: u32) -> vec3<f32> {
    if kind == LIGHTING_KIND_DIRECT {
        return vec3<f32>(1.0, 0.86, 0.12);
    }
    if kind == LIGHTING_KIND_SHADOWED {
        return vec3<f32>(0.12, 0.45, 1.0);
    }
    if kind == LIGHTING_KIND_EMISSIVE {
        return vec3<f32>(1.0, 0.12, 0.85);
    }
    if kind == LIGHTING_KIND_SKY {
        return vec3<f32>(0.1, 0.9, 1.0);
    }
    return vec3<f32>(0.68, 0.72, 0.78);
}

fn trace_termination_color(reason: u32) -> vec3<f32> {
    if reason == TRACE_TERMINATION_HIT {
        return vec3<f32>(0.1, 1.0, 0.2);
    }
    if reason == TRACE_TERMINATION_MAX_STEPS {
        return vec3<f32>(1.0, 0.12, 0.85);
    }
    if reason == TRACE_TERMINATION_MAX_DISTANCE {
        return vec3<f32>(1.0, 0.78, 0.08);
    }
    if reason == TRACE_TERMINATION_GRID_EXIT {
        return vec3<f32>(0.25, 0.5, 1.0);
    }
    return vec3<f32>(1.0, 0.08, 0.05);
}

fn sampled_sdf_distance_color(distance: f32) -> vec3<f32> {
    let normalized_distance = clamp(distance / 2.0, 0.0, 1.0);
    return mix(vec3<f32>(0.08, 0.2, 1.0), vec3<f32>(1.0, 0.12, 0.04), normalized_distance);
}

fn probe_reason_color(reason: u32) -> vec3<f32> {
    if reason == 1u {
        return vec3<f32>(0.15, 0.65, 1.0);
    }
    if reason == 2u {
        return vec3<f32>(0.9, 0.25, 0.95);
    }
    if reason == 3u {
        return vec3<f32>(1.0, 0.5, 0.12);
    }
    if reason >= 4u {
        return vec3<f32>(1.0, 0.9, 0.15);
    }
    return vec3<f32>(0.45, 0.55, 0.68);
}

fn field_radiance(index: u32, field: RadianceFieldData, is_near: bool) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    let resolution = field.resolution.xyz;
    let logical = vec3<u32>(index % resolution.x, (index / resolution.x) % resolution.y, index / (resolution.x * resolution.y));
    let physical = (logical + field.grid_offset.xyz) % resolution;
    let physical_index = field_cell_index(physical, resolution);
    for (var lobe = 0u; lobe < RADIANCE_FIELD_DIRECTIONS; lobe++) {
        var value = far_radiance_values.values[physical_index * RADIANCE_FIELD_DIRECTIONS + lobe];
        if is_near {
            value = near_radiance_values.values[physical_index * RADIANCE_FIELD_DIRECTIONS + lobe];
        }
        result += value.rgb * value.a;
    }
    return result / f32(RADIANCE_FIELD_DIRECTIONS);
}

fn field_cell_position(index: u32, field: RadianceFieldData) -> vec3<f32> {
    let resolution = field.resolution.xyz;
    let x = index % resolution.x;
    let y = (index / resolution.x) % resolution.y;
    let z = index / (resolution.x * resolution.y);
    let logical_center = vec3<f32>(f32(x), f32(y), f32(z)) + vec3<f32>(0.5);
    let cell_size = (field.maximum.xyz - field.minimum.xyz) / vec3<f32>(resolution);
    return field.minimum.xyz + logical_center * cell_size;
}

fn sdf_physical_coordinate(logical: vec3<u32>) -> vec3<i32> {
    return vec3<i32>((logical + sdf_clipmap.grid_offset.xyz) % sdf_clipmap.resolution.xyz);
}

fn sdf_distance_at(logical: vec3<i32>) -> f32 {
    let size = vec3<i32>(sdf_clipmap.resolution.xyz);
    if any(logical < vec3<i32>(0)) || any(logical >= size) {
        return 1.0;
    }
    return textureLoad(sdf_texture, sdf_physical_coordinate(vec3<u32>(logical)), 0).x;
}

fn sdf_occupancy_surface_cell(logical: vec3<i32>) -> bool {
    if sdf_distance_at(logical) > 0.0 {
        return false;
    }
    return sdf_distance_at(logical + vec3<i32>(1, 0, 0)) > 0.0 || sdf_distance_at(logical - vec3<i32>(1, 0, 0)) > 0.0 || sdf_distance_at(logical + vec3<i32>(0, 1, 0)) > 0.0 || sdf_distance_at(logical - vec3<i32>(0, 1, 0)) > 0.0 || sdf_distance_at(logical + vec3<i32>(0, 0, 1)) > 0.0 || sdf_distance_at(logical - vec3<i32>(0, 0, 1)) > 0.0;
}

fn make_vertex(position: vec3<f32>, color: vec4<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(position, 1.0);
    output.color = color;
    return output;
}

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    if instance_index < SURFACE_PROBE_CAPACITY {
        if surface_probes.values[instance_index].position_confidence.w <= 0.0 || surface_probes.values[instance_index].normal_age.w > 1.0 {
            return make_vertex(vec3<f32>(0.0), vec4<f32>(0.0));
        }
        let center = surface_probes.values[instance_index].position_confidence.xyz;
        let reason_color = probe_reason_color(surface_probes.values[instance_index].sample_reason);
        let axis_index = vertex_index / 2u;
        let endpoint = vertex_index % 2u;
        if axis_index < 3u {
            var axis = vec3<f32>(1.0, 0.0, 0.0);
            if axis_index == 1u {
                axis = vec3<f32>(0.0, 1.0, 0.0);
            } else if axis_index == 2u {
                axis = vec3<f32>(0.0, 0.0, 1.0);
            }
            let direction = select(-axis, axis, endpoint == 1u);
            let position = center + direction * 0.025;
            return make_vertex(position, vec4<f32>(reason_color, 0.9));
        }
        let normal = normalize(surface_probes.values[instance_index].normal_age.xyz);
        let position = center + select(vec3<f32>(0.0), normal * 0.2, endpoint == 1u);
        let color = display_radiance(surface_probes.values[instance_index].irradiance.rgb);
        return make_vertex(position, vec4<f32>(color, 0.95));
    }

    let field_cell_count = near_radiance_field.resolution.x * near_radiance_field.resolution.y * near_radiance_field.resolution.z;
    let field_instance = instance_index - SURFACE_PROBE_CAPACITY;
    let is_near = field_instance < field_cell_count;
    if field_instance >= field_cell_count * 2u {
        let trace_instance = field_instance - field_cell_count * 2u;
        var trace_field = near_radiance_field;
        if selected_radiance_cell.coordinate_and_level.w == 1u {
            trace_field = far_radiance_field;
        }
        let cell = min(selected_radiance_cell.coordinate_and_level.xyz, trace_field.resolution.xyz - vec3<u32>(1u));
        let physical_cell = (cell + trace_field.grid_offset.xyz) % trace_field.resolution.xyz;
        let physical_cell_index = field_cell_index(physical_cell, trace_field.resolution.xyz);
        let trace_offset = select(0u, field_cell_count * RADIANCE_FIELD_DIRECTIONS, selected_radiance_cell.coordinate_and_level.w == 1u);
        if trace_instance < RADIANCE_FIELD_DIRECTIONS {
            let trace_data_index = trace_offset + physical_cell_index * RADIANCE_FIELD_DIRECTIONS + trace_instance;
            let trace = radiance_field_trace_debug.values[trace_data_index];
            let origin = trace.probe_position.xyz;
            let raw_direction = trace.direction_hit.xyz;
            let direction_is_valid = all(abs(raw_direction) < vec3<f32>(1e20)) && dot(raw_direction, raw_direction) > 0.0001;
            let direction = normalize(select(vec3<f32>(0.0, 1.0, 0.0), raw_direction, direction_is_valid));
            let hit_position = trace.hit_position.xyz;
            let termination_reason = u32(max(trace.direction_hit.w, 0.0) + 0.5);
            let hit = termination_reason == TRACE_TERMINATION_HIT;
            let valid = trace.probe_position.w > 0.5 && direction_is_valid && all(abs(hit_position) < vec3<f32>(1e20));
            let invalid_color = vec3<f32>(1.0, 0.08, 0.05);
            let geometry_color = select(vec3<f32>(1.0, 0.5, 0.08), vec3<f32>(0.1, 1.0, 0.2), hit);
            let ray_color = select(geometry_color, invalid_color, !valid);
            let lighting_color = select(lighting_kind_color(u32(max(trace.hit_position.w, 0.0) + 0.5)), invalid_color, !valid);
            let termination_color = select(trace_termination_color(termination_reason), invalid_color, !valid);
            let sampled_distance = select(trace.hit_normal_distance.x, 0.0, hit);
            let distance_color = select(sampled_sdf_distance_color(sampled_distance), invalid_color, !valid);
            let marker_position = select(origin + direction * 0.2, hit_position, valid);
            if vertex_index < 2u {
                return make_vertex(select(origin, marker_position, vertex_index == 1u), vec4<f32>(ray_color, 0.95));
            }
            if vertex_index < 4u {
                if !valid || !hit {
                    return make_vertex(vec3<f32>(0.0), vec4<f32>(0.0));
                }
                let normal_start = marker_position;
                let normal_end = marker_position + normalize(trace.hit_normal_distance.xyz) * 0.22;
                return make_vertex(select(normal_start, normal_end, vertex_index == 3u), vec4<f32>(ray_color, 0.95));
            }
            let sphere_vertex = vertex_index - 4u;
            let ring = sphere_vertex / 8u;
            let segment = (sphere_vertex % 8u) / 2u;
            let angle = f32(segment) * 1.5707963 + select(0.0, 1.5707963, sphere_vertex % 2u == 1u);
            var axis_u = vec3<f32>(1.0, 0.0, 0.0);
            var axis_v = vec3<f32>(0.0, 1.0, 0.0);
            if ring == 1u {
                axis_v = vec3<f32>(0.0, 0.0, 1.0);
            } else if ring == 2u {
                axis_u = vec3<f32>(0.0, 1.0, 0.0);
                axis_v = vec3<f32>(0.0, 0.0, 1.0);
            }
            let sphere_offset = (axis_u * cos(angle) + axis_v * sin(angle)) * 0.045;
            var marker_color = lighting_color;
            if ring == 1u {
                marker_color = termination_color;
            } else if ring == 2u {
                marker_color = distance_color;
            }
            return make_vertex(marker_position + sphere_offset, vec4<f32>(marker_color, 0.95));
        }
        if trace_instance < RADIANCE_FIELD_DIRECTIONS + 12u {
            let line_index = trace_instance - RADIANCE_FIELD_DIRECTIONS;
            let edge = array<vec2<u32>, 12>(vec2<u32>(0, 1), vec2<u32>(0, 2), vec2<u32>(0, 4), vec2<u32>(1, 3), vec2<u32>(1, 5), vec2<u32>(2, 3), vec2<u32>(2, 6), vec2<u32>(3, 7), vec2<u32>(4, 5), vec2<u32>(4, 6), vec2<u32>(5, 7), vec2<u32>(6, 7));
            let corners = array<vec3<f32>, 8>(vec3<f32>(0, 0, 0), vec3<f32>(1, 0, 0), vec3<f32>(0, 1, 0), vec3<f32>(1, 1, 0), vec3<f32>(0, 0, 1), vec3<f32>(1, 0, 1), vec3<f32>(0, 1, 1), vec3<f32>(1, 1, 1));
            let cell_minimum = trace_field.minimum.xyz + vec3<f32>(cell) * (trace_field.maximum.xyz - trace_field.minimum.xyz) / vec3<f32>(trace_field.resolution.xyz);
            let cell_size = (trace_field.maximum.xyz - trace_field.minimum.xyz) / vec3<f32>(trace_field.resolution.xyz);
            let edge_corners = edge[line_index];
            let position = cell_minimum + corners[select(edge_corners.x, edge_corners.y, vertex_index == 1u)] * cell_size;
            let valid = radiance_field_trace_debug.values[trace_offset + physical_cell_index * RADIANCE_FIELD_DIRECTIONS].probe_position.w > 0.5;
            return make_vertex(position, vec4<f32>(select(vec3<f32>(1.0, 0.08, 0.05), vec3<f32>(0.2, 0.85, 1.0), valid), 0.95));
        }
        return make_vertex(vec3<f32>(0.0), vec4<f32>(0.0));
    }
    var field = far_radiance_field;
    if is_near {
        field = near_radiance_field;
    }
    let cell_index = select(field_instance - field_cell_count, field_instance, is_near);
    let center = field_cell_position(cell_index, field);
    let cell_size = (field.maximum.xyz - field.minimum.xyz) / vec3<f32>(field.resolution.xyz);
    let radius = max(min(min(cell_size.x, cell_size.y), cell_size.z) * 0.12, 0.025);
    let axis_index = vertex_index / 2u;
    let endpoint = vertex_index % 2u;
    var axis = vec3<f32>(1.0, 0.0, 0.0);
    if axis_index == 1u {
        axis = vec3<f32>(0.0, 1.0, 0.0);
    } else if axis_index == 2u {
        axis = vec3<f32>(0.0, 0.0, 1.0);
    }
    let direction = select(-axis, axis, endpoint == 1u);
    let position = center + select(vec3<f32>(0.0), direction * radius, axis_index < 3u);
    let radiance = field_radiance(cell_index, field, is_near);
    let alpha = select(0.35, 0.68, is_near);
    return make_vertex(position, vec4<f32>(display_radiance(radiance), alpha));
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}

@vertex
fn occupancy_vertex_main(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let resolution = sdf_clipmap.resolution.xyz;
    let cell_count = resolution.x * resolution.y * resolution.z;
    if instance_index >= cell_count {
        return make_vertex(vec3<f32>(0.0), vec4<f32>(0.0));
    }
    let logical = vec3<i32>(i32(instance_index % resolution.x), i32((instance_index / resolution.x) % resolution.y), i32(instance_index / (resolution.x * resolution.y)));
    if !sdf_occupancy_surface_cell(logical) {
        return make_vertex(vec3<f32>(0.0), vec4<f32>(0.0));
    }
    let corners = array<vec3<f32>, 8>(vec3<f32>(0, 0, 0), vec3<f32>(1, 0, 0), vec3<f32>(0, 1, 0), vec3<f32>(1, 1, 0), vec3<f32>(0, 0, 1), vec3<f32>(1, 0, 1), vec3<f32>(0, 1, 1), vec3<f32>(1, 1, 1));
    let triangles = array<u32, 36>(0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5);
    let voxel_size = (sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz) / vec3<f32>(resolution);
    let center = sdf_clipmap.minimum.xyz + (vec3<f32>(logical) + vec3<f32>(0.5)) * voxel_size;
    let position = center + (corners[triangles[vertex_index]] - vec3<f32>(0.5)) * voxel_size;
    return make_vertex(position, vec4<f32>(0.95, 0.32, 0.08, 0.24));
}
