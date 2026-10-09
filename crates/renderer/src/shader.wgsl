struct Camera {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    view: mat4x4<f32>,
    shadow_view_projection: array<mat4x4<f32>, 3>,
    shadow_cascade_splits: vec4<f32>,
    spot_shadow_view_projections: array<mat4x4<f32>, 8>,
    spot_shadow_rects: array<vec4<f32>, 8>,
    spot_shadow_atlas_size: vec4<f32>,
    point_shadow_view_projections: array<mat4x4<f32>, 12>,
    light_directions: array<vec4<f32>, 8>,
    light_colors: array<vec4<f32>, 8>,
    light_positions: array<vec4<f32>, 8>,
    light_params: array<vec4<f32>, 8>,
    light_kinds: array<vec4<u32>, 8>,
    light_shadow_modes: array<vec4<u32>, 8>,
    light_count: vec4<u32>,
    camera_position: vec4<f32>,
    debug_mode: vec4<u32>,
    shadow_settings: vec4<f32>,
    viewport_size: vec4<f32>,
    shadow_flags: vec4<u32>,
    ambient_light: vec4<f32>,
    gi_settings: vec4<u32>,
};

struct IrradianceVolume {
    minimum: vec4<f32>,
    maximum: vec4<f32>,
    resolution: vec4<u32>,
    probe_offset: u32,
    padding: vec3<u32>,
};

struct IrradianceData {
    volumes: array<IrradianceVolume, 4>,
    probes: array<vec4<f32>, 4096>,
    directions: array<vec4<f32>, 4096>,
    probe_positions: array<vec4<f32>, 4096>,
    volume_count: vec4<u32>,
};

struct SdfClipmapData {
    minimum: vec4<f32>,
    maximum: vec4<f32>,
    resolution: vec4<u32>,
    settings: vec4<u32>,
    grid_offset: vec4<u32>,
    sky_radiance: vec4<f32>,
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

struct SurfaceProbeUniform {
    screen_resolution: vec4<u32>,
    cache_settings: vec4<u32>,
};

struct SurfaceProbe {
    position_confidence: vec4<f32>,
    normal_age: vec4<f32>,
    irradiance: vec4<f32>,
    sample_reason: u32,
    surface_id: u32,
};

struct SurfaceProbeValues {
    values: array<SurfaceProbe>,
};

struct SurfaceProbeCandidate {
    pixel: vec2<u32>,
    reason: u32,
    importance: f32,
    valid: u32,
};

struct SurfaceProbeLighting {
    radiance: vec3<f32>,
    support: f32,
};

struct RadianceTraceResult {
    radiance: vec3<f32>,
    hit_position: vec3<f32>,
    hit_normal: vec3<f32>,
    hit_distance: f32,
    termination_reason: u32,
    last_sdf_distance: f32,
    lighting_kind: u32,
};

struct SdfHitRadianceResult {
    radiance: vec3<f32>,
    lighting_kind: u32,
};

struct SdfTraceResult {
    position: vec3<f32>,
    normal: vec3<f32>,
    travelled: f32,
    termination_reason: u32,
    last_sdf_distance: f32,
};

struct ProbeRadianceTraceResult {
    radiance: vec3<f32>,
    termination_reason: u32,
};

const LIGHTING_KIND_UNLIT: u32 = 0u;
const LIGHTING_KIND_DIRECT: u32 = 1u;
const LIGHTING_KIND_SHADOWED: u32 = 2u;
const LIGHTING_KIND_EMISSIVE: u32 = 3u;
const LIGHTING_KIND_SKY: u32 = 4u;
const TRACE_TERMINATION_HIT: u32 = 1u;
const TRACE_TERMINATION_MAX_STEPS: u32 = 2u;
const TRACE_TERMINATION_MAX_DISTANCE: u32 = 3u;
const TRACE_TERMINATION_GRID_EXIT: u32 = 4u;

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(0) @binding(1)
var shadow_texture: texture_depth_2d_array;

@group(0) @binding(2)
var shadow_sampler: sampler_comparison;

@group(0) @binding(3)
var<storage, read> irradiance: IrradianceData;

@group(0) @binding(4)
var spot_shadow_texture: texture_depth_2d;

@group(0) @binding(5)
var spot_shadow_sampler: sampler_comparison;

@group(0) @binding(6)
var point_shadow_texture: texture_depth_2d_array;

@group(0) @binding(7)
var point_shadow_sampler: sampler_comparison;

@group(0) @binding(8)
var sdf_clipmap_texture: texture_3d<f32>;

@group(0) @binding(9)
var<uniform> sdf_clipmap: SdfClipmapData;

@group(0) @binding(10)
var sdf_emission_texture: texture_3d<f32>;

@group(0) @binding(11)
var<uniform> near_radiance_field: RadianceFieldData;

@group(0) @binding(12)
var<storage, read_write> near_radiance_field_values: RadianceFieldValues;

@group(0) @binding(13)
var<uniform> far_radiance_field: RadianceFieldData;

@group(0) @binding(14)
var<storage, read_write> far_radiance_field_values: RadianceFieldValues;

@group(0) @binding(15)
var<uniform> surface_probe_settings: SurfaceProbeUniform;

@group(0) @binding(16)
var<storage, read_write> surface_probes: SurfaceProbeValues;

@group(0) @binding(20)
var<storage, read_write> radiance_field_trace_debug: RadianceFieldTraceDebugValues;

@group(1) @binding(0)
var scene_depth_texture: texture_depth_2d;

@group(1) @binding(1)
var scene_depth_sampler: sampler;

@group(1) @binding(2)
var surface_normal_texture: texture_2d<f32>;

@group(1) @binding(3)
var surface_id_texture: texture_2d<u32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) material_base_color: vec4<f32>,
    @location(4) material_params: vec4<f32>,
    @location(5) material_emission: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) shadow_position_0: vec4<f32>,
    @location(4) world_position: vec3<f32>,
    @location(5) material_base_color: vec4<f32>,
    @location(6) material_params: vec4<f32>,
    @location(7) material_emission: vec4<f32>,
    @location(8) indirect: vec3<f32>,
    @location(9) shadow_position_1: vec4<f32>,
    @location(10) shadow_position_2: vec4<f32>,
};

struct FragmentOutput {
    @location(0) color: vec4<f32>,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.color = input.color;
    output.material_base_color = input.material_base_color;
    output.material_params = input.material_params;
    output.material_emission = input.material_emission;
    let shadow_origin = input.position;
    output.shadow_position_0 = camera.shadow_view_projection[0] * vec4<f32>(shadow_origin, 1.0);
    output.shadow_position_1 = camera.shadow_view_projection[1] * vec4<f32>(shadow_origin, 1.0);
    output.shadow_position_2 = camera.shadow_view_projection[2] * vec4<f32>(shadow_origin, 1.0);
    output.world_position = input.position;
    output.indirect = vec3<f32>(0.0);
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let albedo = input.color.rgb * input.material_base_color.rgb;
    let emission = input.material_emission.rgb * input.material_params.w;
    if camera.debug_mode.x == 1u || camera.debug_mode.x == 2u {
        return FragmentOutput(vec4<f32>(albedo + emission, input.color.a * input.material_base_color.a));
    }
    let normal = normalize(input.normal);
    let ambient_occlusion = select(1.0, sample_ambient_occlusion(input.clip_position), camera.shadow_flags.z == 1u);
    var indirect = vec3<f32>(0.0);
    if camera.debug_mode.z == 1u {
        if sdf_clipmap.settings.x == 1u {
            if camera.gi_settings.x == 1u {
                let screen_pixel = clamp(vec2<i32>(input.clip_position.xy), vec2<i32>(0), vec2<i32>(camera.viewport_size.xy) - vec2<i32>(1));
                let surface_id = textureLoad(surface_id_texture, screen_pixel, 0).x;
                let surface_lighting = sample_surface_probe_field(input.world_position, normal, surface_id, vec2<u32>(screen_pixel));
                indirect = surface_lighting.radiance;
                if surface_lighting.support < 0.99 {
                    let coarse_radiance = sample_world_radiance_field(input.world_position, normal);
                    indirect = mix(coarse_radiance, surface_lighting.radiance, surface_lighting.support);
                }
            } else {
                indirect = sample_world_radiance_field(input.world_position, normal);
            }
            indirect *= ambient_occlusion;
        } else {
            indirect = sample_irradiance(input.world_position, normal) * ambient_occlusion;
        }
    }
    if camera.debug_mode.x == 4u {
        return FragmentOutput(vec4<f32>(max(indirect, vec3<f32>(0.0)), 1.0));
    }
    let metallic = input.material_params.x;
    let roughness = input.material_params.y;
    let view_direction = normalize(camera.camera_position.xyz - input.world_position);
    var lighting = vec3<f32>(camera.ambient_light.x) * ambient_occlusion;
    var primary_shadow = 1.0;
    var direct_visibility = 1.0;
    for (var index = 0u; index < camera.light_count.x; index++) {
        var light_direction = normalize(camera.light_directions[index].xyz);
        var attenuation = 1.0;
        if camera.light_kinds[index].x == 1u {
            let to_light = camera.light_positions[index].xyz - input.world_position;
            let distance_to_light = length(to_light);
            light_direction = normalize(to_light);
            attenuation = pow(max(1.0 - distance_to_light / max(camera.light_params[index].x, 0.001), 0.0), 2.0);
        } else if camera.light_kinds[index].x == 2u {
            let to_light = camera.light_positions[index].xyz - input.world_position;
            let distance_to_light = length(to_light);
            light_direction = normalize(to_light);
            let cone = dot(normalize(-to_light), normalize(camera.light_directions[index].xyz));
            let cone_range = max(camera.light_params[index].y - camera.light_params[index].z, 0.001);
            let cone_factor = clamp((cone - camera.light_params[index].z) / cone_range, 0.0, 1.0);
            attenuation = pow(max(1.0 - distance_to_light / max(camera.light_params[index].x, 0.001), 0.0), 2.0) * cone_factor;
        }
        let diffuse = max(dot(normal, light_direction), 0.0) * attenuation;
        var shadow = 1.0;
        if (camera.debug_mode.y == 1u && camera.light_shadow_modes[index].x == 1u) {
            let view_depth = -(camera.view * vec4<f32>(input.world_position, 1.0)).z;
            var cascade = 0u;
            if camera.debug_mode.w > 1u && view_depth > camera.shadow_cascade_splits.y { cascade = 1u; }
            if camera.debug_mode.w > 2u && view_depth > camera.shadow_cascade_splits.z { cascade = 2u; }
            let shadow_position = select(select(input.shadow_position_0, input.shadow_position_1, cascade == 1u), input.shadow_position_2, cascade == 2u);
            let projected = shadow_position.xyz / shadow_position.w;
            if (projected.x >= -1.0 && projected.x <= 1.0 && projected.y >= -1.0 && projected.y <= 1.0 && projected.z >= 0.0 && projected.z <= 1.0) {
                let texel_size = camera.shadow_settings.z / camera.shadow_settings.w;
                let normal_factor = 1.0 - max(dot(normal, normalize(camera.light_directions[0].xyz)), 0.0);
                let compare_depth = projected.z + camera.shadow_settings.x - camera.shadow_settings.y * normal_factor;
                let visibility = sample_directional_shadow(shadow_position, cascade, texel_size, compare_depth);
                shadow = visibility;
                if cascade < 2u {
                    let split = select(camera.shadow_cascade_splits.y, camera.shadow_cascade_splits.z, cascade == 1u);
                    let blend_width = max(split * 0.12, 0.5);
                    if view_depth > split - blend_width {
                        let next_cascade = cascade + 1u;
                        let next_position = select(input.shadow_position_1, input.shadow_position_2, next_cascade == 2u);
                        let next_compare_depth = (next_position.z / next_position.w) + camera.shadow_settings.x - camera.shadow_settings.y * normal_factor;
                        let next_visibility = sample_directional_shadow(next_position, next_cascade, texel_size, next_compare_depth);
                        shadow = mix(visibility, next_visibility, clamp((view_depth - (split - blend_width)) / blend_width, 0.0, 1.0));
                    }
                }
                if camera.shadow_flags.x == 1u && view_depth < 10.0 {
                    shadow = min(shadow, sample_contact_shadow(input.world_position, normal, light_direction));
                }
                primary_shadow = shadow;
            }
        } else if (camera.debug_mode.y == 1u && camera.light_shadow_modes[index].x == 2u) {
            let spot_slot = min(camera.light_shadow_modes[index].y, 7u);
            let projected_position = camera.spot_shadow_view_projections[spot_slot] * vec4<f32>(input.world_position, 1.0);
            let projected = projected_position.xyz / projected_position.w;
            if (projected.x >= -1.0 && projected.x <= 1.0 && projected.y >= -1.0 && projected.y <= 1.0 && projected.z >= 0.0 && projected.z <= 1.0) {
                let shadow_rect = camera.spot_shadow_rects[spot_slot];
                let local_uv = vec2<f32>(projected.x * 0.5 + 0.5, 1.0 - (projected.y * 0.5 + 0.5));
                let shadow_uv = local_uv * shadow_rect.xy + shadow_rect.zw;
                let texel_size = vec2<f32>(1.0, 1.0) / camera.spot_shadow_atlas_size.xy;
                var visibility = 0.0;
                for (var offset_y = -1; offset_y <= 1; offset_y++) {
                    for (var offset_x = -1; offset_x <= 1; offset_x++) {
                        let offset = vec2<f32>(f32(offset_x), f32(offset_y)) * texel_size;
                        visibility += textureSampleCompare(spot_shadow_texture, spot_shadow_sampler, shadow_uv + offset, projected.z - 0.0005);
                    }
                }
                shadow = visibility / 9.0;
            }
        } else if (camera.debug_mode.y == 1u && camera.light_shadow_modes[index].x == 3u) {
            let to_light = camera.light_positions[index].xyz - input.world_position;
            let from_light = -to_light;
            let absolute_direction = abs(from_light);
            var face = 0u;
            if absolute_direction.y > absolute_direction.x && absolute_direction.y >= absolute_direction.z { face = select(3u, 2u, from_light.y >= 0.0); }
            else if absolute_direction.z > absolute_direction.x { face = select(5u, 4u, from_light.z >= 0.0); }
            else { face = select(1u, 0u, from_light.x >= 0.0); }
            let point_slot = min(camera.light_shadow_modes[index].y, 1u);
            let matrix_index = point_slot * 6u + face;
            let projected_position = camera.point_shadow_view_projections[matrix_index] * vec4<f32>(input.world_position, 1.0);
            let projected = projected_position.xyz / projected_position.w;
            if projected.x >= -1.0 && projected.x <= 1.0 && projected.y >= -1.0 && projected.y <= 1.0 && projected.z >= 0.0 && projected.z <= 1.0 {
                let shadow_uv = vec2<f32>(projected.x * 0.5 + 0.5, 1.0 - (projected.y * 0.5 + 0.5));
                shadow = textureSampleCompare(point_shadow_texture, point_shadow_sampler, shadow_uv, i32(point_slot * 6u + face), projected.z - 0.0001);
            }
        }
        if camera.shadow_flags.y == 1u && index > 0u {
            shadow = min(shadow, primary_shadow);
        }
        if camera.shadow_flags.w == 0u {
            shadow = 1.0;
        }
        direct_visibility = min(direct_visibility, shadow);
        let half_direction = normalize(light_direction + view_direction);
        let specular_power = mix(64.0, 4.0, roughness);
        let specular = pow(max(dot(normal, half_direction), 0.0), specular_power) * (1.0 - metallic) * 0.04 + metallic * pow(max(dot(normal, half_direction), 0.0), specular_power);
        lighting += camera.light_colors[index].rgb * camera.light_colors[index].a * (diffuse + specular) * shadow;
    }
    let final_color = albedo * (lighting + indirect) + emission;
    if camera.debug_mode.x == 3u {
        return FragmentOutput(vec4<f32>(vec3<f32>(direct_visibility), 1.0));
    }
    let color = vec4<f32>(final_color, input.color.a * input.material_base_color.a);
    return FragmentOutput(color);
}

fn sample_directional_shadow(position: vec4<f32>, cascade: u32, texel_size: f32, compare_depth: f32) -> f32 {
    let projected = position.xyz / position.w;
    let shadow_uv = vec2<f32>(projected.x * 0.5 + 0.5, 1.0 - (projected.y * 0.5 + 0.5));
    let center_visibility = textureSampleCompare(shadow_texture, shadow_sampler, shadow_uv, cascade, compare_depth);
    var visibility = 0.0;
    for (var offset_y = -1; offset_y <= 1; offset_y++) {
        for (var offset_x = -1; offset_x <= 1; offset_x++) {
            let offset = vec2<f32>(f32(offset_x), f32(offset_y)) * texel_size;
            visibility += textureSampleCompare(shadow_texture, shadow_sampler, shadow_uv + offset, cascade, compare_depth);
        }
    }
    if cascade == 0u {
        return center_visibility;
    }
    return min(visibility / 9.0, center_visibility);
}

fn sample_contact_shadow(position: vec3<f32>, normal: vec3<f32>, light_direction: vec3<f32>) -> f32 {
    let facing = dot(normal, light_direction);
    if facing <= 0.05 {
        return 1.0;
    }
    for (var step_index = 1u; step_index <= 4u; step_index++) {
        let sample_position = position + normal * 0.02 + light_direction * (f32(step_index) * 0.04);
        let projected = camera.view_projection * vec4<f32>(sample_position, 1.0);
        let normalized = projected.xyz / projected.w;
        let uv = vec2<f32>(normalized.x * 0.5 + 0.5, 1.0 - (normalized.y * 0.5 + 0.5));
        if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
            continue;
        }
        let scene_depth = textureLoad(scene_depth_texture, vec2<i32>(uv * camera.viewport_size.xy), 0);
        if scene_depth + 0.0005 < normalized.z {
            return 0.6;
        }
    }
    return 1.0;
}

fn sample_ambient_occlusion(clip_position: vec4<f32>) -> f32 {
    let screen = clip_position.xy / max(clip_position.w, 0.001);
    let uv = vec2<f32>(screen.x * 0.5 + 0.5, 1.0 - (screen.y * 0.5 + 0.5));
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }
    let pixel = vec2<i32>(uv * camera.viewport_size.xy);
    let center_depth = textureLoad(scene_depth_texture, pixel, 0);
    var occluded = 0.0;
    let offsets = array<vec2<i32>, 4>(vec2<i32>(-2, 0), vec2<i32>(2, 0), vec2<i32>(0, -2), vec2<i32>(0, 2));
    for (var index = 0u; index < 4u; index++) {
        let sample_pixel = clamp(pixel + offsets[index], vec2<i32>(0), vec2<i32>(camera.viewport_size.xy) - vec2<i32>(1));
        let sample_depth = textureLoad(scene_depth_texture, sample_pixel, 0);
        occluded += select(0.0, 1.0, sample_depth + 0.002 < center_depth);
    }
    return 1.0 - occluded * 0.12;
}

fn sample_irradiance(position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    var blended_weight = 0.0;
    var remaining_weight = 1.0;
    let normal_direction = normalize(normal);
    for (var volume_index = 0u; volume_index < irradiance.volume_count.x; volume_index++) {
        let volume = irradiance.volumes[volume_index];
        if (position.x < volume.minimum.x || position.y < volume.minimum.y || position.z < volume.minimum.z || position.x > volume.maximum.x || position.y > volume.maximum.y || position.z > volume.maximum.z) {
            continue;
        }
        let size = max(volume.maximum.xyz - volume.minimum.xyz, vec3<f32>(0.001));
        let maximum = volume.resolution.xyz - vec3<u32>(1u);
        let probe_spacing = size / vec3<f32>(max(maximum, vec3<u32>(1u)));
        let receiver_bias_scale = select(0.75, 0.25, abs(normal_direction.y) < 0.5);
        let receiver_bias = dot(abs(normal_direction), probe_spacing) * receiver_bias_scale;
        let wall_receiver_lift = select(0.0, probe_spacing.y * 0.5, abs(normal_direction.y) < 0.5);
        let sample_position = position + normal_direction * receiver_bias + vec3<f32>(0.0, wall_receiver_lift, 0.0);
        let coordinate = clamp((sample_position - volume.minimum.xyz) / size, vec3<f32>(0.0), vec3<f32>(1.0)) * vec3<f32>(maximum);
        let base = vec3<u32>(floor(coordinate));
        let fraction = fract(coordinate);
        let side_blend_width = max(min(min(probe_spacing.x, probe_spacing.y), probe_spacing.z), 0.001);
        var volume_result = vec3<f32>(0.0);
        var volume_direction = vec3<f32>(0.0);
        var volume_direction_weight = 0.0;
        var volume_weight = 0.0;
        var volume_color_weight = 0.0;
        for (var z = 0u; z <= 1u; z++) {
            for (var y = 0u; y <= 1u; y++) {
                for (var x = 0u; x <= 1u; x++) {
                    let sample_coordinate = min(base + vec3<u32>(x, y, z), maximum);
                    let index = sample_coordinate.x + sample_coordinate.y * volume.resolution.x + sample_coordinate.z * volume.resolution.x * volume.resolution.y;
                    let probe_index = volume.probe_offset + index;
                    let probe_position = irradiance.probe_positions[probe_index];
                    if probe_position.w < 0.5 {
                        continue;
                    }
                    let probe_offset = probe_position.xyz - position;
                    let front_distance = dot(normal_direction, probe_offset);
                    let probe_distance = length(probe_offset);
                    if front_distance <= 0.0 || probe_distance <= 0.0001 {
                        continue;
                    }
                    let face_alignment = front_distance / probe_distance;
                    if face_alignment <= 0.0 {
                        continue;
                    }
                    let side_weight = smoothstep(0.0, side_blend_width, front_distance);
                    let facing_weight = smoothstep(0.0, 0.25, face_alignment);
                    let trilinear_weight = select(1.0 - fraction.x, fraction.x, x == 1u) * select(1.0 - fraction.y, fraction.y, y == 1u) * select(1.0 - fraction.z, fraction.z, z == 1u);
                    let interpolation_weight = trilinear_weight * side_weight;
                    let weight = interpolation_weight * facing_weight;
                    volume_result += irradiance.probes[probe_index].rgb * weight;
                    volume_direction += irradiance.directions[probe_index].xyz * irradiance.directions[probe_index].w * weight;
                    volume_direction_weight += irradiance.directions[probe_index].w * weight;
                    volume_weight += interpolation_weight;
                    volume_color_weight += weight;
                }
            }
        }
        let support_weight = smoothstep(0.0, 0.1, volume_weight);
        if support_weight > 0.0 && volume_color_weight > 0.0001 {
            volume_result /= volume_color_weight;
            volume_direction /= volume_color_weight;
            volume_direction_weight /= volume_color_weight;
            if volume_direction_weight > 0.0 {
                let direction_alignment = max(dot(normal_direction, normalize(volume_direction)), 0.0);
                volume_result *= 1.0 + direction_alignment * min(volume_direction_weight, 1.0) * 0.35;
            }
            let edge_distance = min(min(min(position.x - volume.minimum.x, volume.maximum.x - position.x), min(position.y - volume.minimum.y, volume.maximum.y - position.y)), min(position.z - volume.minimum.z, volume.maximum.z - position.z));
            var coarser_probe_spacing = 0.0;
            var coarser_edge_distance = 0.0;
            var has_later_volume = false;
            for (var later_index = volume_index + 1u; later_index < irradiance.volume_count.x; later_index++) {
                let later_volume = irradiance.volumes[later_index];
                if position.x >= later_volume.minimum.x && position.y >= later_volume.minimum.y && position.z >= later_volume.minimum.z && position.x <= later_volume.maximum.x && position.y <= later_volume.maximum.y && position.z <= later_volume.maximum.z {
                    let later_size = max(later_volume.maximum.xyz - later_volume.minimum.xyz, vec3<f32>(0.001));
                    let later_maximum = later_volume.resolution.xyz - vec3<u32>(1u);
                    let later_spacing = later_size / vec3<f32>(max(later_maximum, vec3<u32>(1u)));
                    coarser_probe_spacing = max(max(later_spacing.x, later_spacing.y), later_spacing.z);
                    coarser_edge_distance = min(min(min(position.x - later_volume.minimum.x, later_volume.maximum.x - position.x), min(position.y - later_volume.minimum.y, later_volume.maximum.y - position.y)), min(position.z - later_volume.minimum.z, later_volume.maximum.z - position.z));
                    has_later_volume = true;
                    break;
                }
            }
            let transition_width = max(max(max(probe_spacing.x, probe_spacing.y), probe_spacing.z), coarser_probe_spacing) * 2.0;
            let bounded_transition_width = max(min(transition_width, coarser_edge_distance), 0.001);
            let edge_weight = select(1.0, smoothstep(0.0, bounded_transition_width, edge_distance), has_later_volume);
            let volume_confidence = edge_weight * support_weight;
            let blend_weight = remaining_weight * volume_confidence;
            result += volume_result * blend_weight;
            blended_weight += blend_weight;
            remaining_weight *= 1.0 - volume_confidence;
        }
    }
    return result / max(blended_weight, 0.0001);
}

fn sample_sdf_distance(position: vec3<f32>) -> f32 {
    let clipmap_size = sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz;
    let normalized_position = (position - sdf_clipmap.minimum.xyz) / clipmap_size;
    if any(normalized_position < vec3<f32>(0.0)) || any(normalized_position >= vec3<f32>(1.0)) {
        return 16.0;
    }
    let coordinate = clamp(normalized_position * vec3<f32>(sdf_clipmap.resolution.xyz) - vec3<f32>(0.5), vec3<f32>(0.0), vec3<f32>(sdf_clipmap.resolution.xyz - vec3<u32>(1u)));
    let base = vec3<i32>(floor(coordinate));
    let fraction = fract(coordinate);
    var distance = 0.0;
    for (var z = 0u; z <= 1u; z++) {
        for (var y = 0u; y <= 1u; y++) {
            for (var x = 0u; x <= 1u; x++) {
                let sample_coordinate = clamp(base + vec3<i32>(i32(x), i32(y), i32(z)), vec3<i32>(0), vec3<i32>(sdf_clipmap.resolution.xyz) - vec3<i32>(1));
                let weight = select(1.0 - fraction.x, fraction.x, x == 1u) * select(1.0 - fraction.y, fraction.y, y == 1u) * select(1.0 - fraction.z, fraction.z, z == 1u);
                distance += textureLoad(sdf_clipmap_texture, sdf_physical_coordinate(sample_coordinate), 0).x * weight;
            }
        }
    }
    return distance;
}

fn sample_sdf_normal(position: vec3<f32>, voxel_size: vec3<f32>) -> vec3<f32> {
    let x_offset = vec3<f32>(voxel_size.x, 0.0, 0.0);
    let y_offset = vec3<f32>(0.0, voxel_size.y, 0.0);
    let z_offset = vec3<f32>(0.0, 0.0, voxel_size.z);
    let gradient = vec3<f32>(
        sample_sdf_distance(position + x_offset) - sample_sdf_distance(position - x_offset),
        sample_sdf_distance(position + y_offset) - sample_sdf_distance(position - y_offset),
        sample_sdf_distance(position + z_offset) - sample_sdf_distance(position - z_offset),
    );
    return normalize(gradient);
}

fn trace_conservative_voxels(origin: vec3<f32>, direction: vec3<f32>, maximum_distance: f32, maximum_steps: u32) -> SdfTraceResult {
    let clipmap_size = sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz;
    let resolution = vec3<f32>(sdf_clipmap.resolution.xyz);
    let voxel_size = clipmap_size / resolution;
    let minimum_voxel_size = min(min(voxel_size.x, voxel_size.y), voxel_size.z);
    let start_offset = minimum_voxel_size * 0.15;
    var travelled = start_offset;
    var position = origin + direction * travelled;
    let start_normalized = (position - sdf_clipmap.minimum.xyz) / clipmap_size;
    if any(start_normalized < vec3<f32>(0.0)) || any(start_normalized >= vec3<f32>(1.0)) {
        return SdfTraceResult(position, vec3<f32>(0.0), travelled, TRACE_TERMINATION_GRID_EXIT, 16.0);
    }
    let initial_coordinate = vec3<i32>(floor(start_normalized * resolution));
    let initial_sample = textureLoad(sdf_clipmap_texture, sdf_physical_coordinate(initial_coordinate), 0);
    let ignore_initial_occupied_cell = initial_sample.x <= 0.0;
    var last_sdf_distance = initial_sample.x;
    for (var step = 0u; step < maximum_steps; step++) {
        if travelled >= maximum_distance {
            return SdfTraceResult(position, vec3<f32>(0.0), travelled, TRACE_TERMINATION_MAX_DISTANCE, last_sdf_distance);
        }
        position = origin + direction * travelled;
        let normalized_position = (position - sdf_clipmap.minimum.xyz) / clipmap_size;
        if any(normalized_position < vec3<f32>(0.0)) || any(normalized_position >= vec3<f32>(1.0)) {
            return SdfTraceResult(position, vec3<f32>(0.0), travelled, TRACE_TERMINATION_GRID_EXIT, last_sdf_distance);
        }
        let coordinate = vec3<i32>(floor(normalized_position * resolution));
        let sdf_value = textureLoad(sdf_clipmap_texture, sdf_physical_coordinate(coordinate), 0);
        last_sdf_distance = sdf_value.x;
        let is_initial_cell = all(coordinate == initial_coordinate);
        if sdf_value.x <= 0.0 && !(ignore_initial_occupied_cell && is_initial_cell) {
            let gradient = sample_sdf_normal(position, voxel_size);
            let normal = select(-direction, normalize(gradient), dot(gradient, gradient) > 0.000001);
            return SdfTraceResult(position, normal, travelled, TRACE_TERMINATION_HIT, sdf_value.x);
        }
        let cell_center = sdf_clipmap.minimum.xyz + (vec3<f32>(coordinate) + vec3<f32>(0.5)) * voxel_size;
        let safe_center_distance = textureLoad(sdf_emission_texture, sdf_physical_coordinate(coordinate), 0).a;
        let safe_distance = max(safe_center_distance - distance(position, cell_center), 0.0);
        if safe_distance > minimum_voxel_size {
            travelled += min(safe_distance * 0.8, maximum_distance - travelled);
            continue;
        }
        let cell_minimum = sdf_clipmap.minimum.xyz + vec3<f32>(coordinate) * voxel_size;
        let cell_maximum = cell_minimum + voxel_size;
        let large_distance = 1.0e30;
        let distance_x = select((cell_minimum.x - position.x) / direction.x, (cell_maximum.x - position.x) / direction.x, direction.x > 0.0);
        let distance_y = select((cell_minimum.y - position.y) / direction.y, (cell_maximum.y - position.y) / direction.y, direction.y > 0.0);
        let distance_z = select((cell_minimum.z - position.z) / direction.z, (cell_maximum.z - position.z) / direction.z, direction.z > 0.0);
        let next_x = select(large_distance, distance_x, abs(direction.x) > 0.000001);
        let next_y = select(large_distance, distance_y, abs(direction.y) > 0.000001);
        let next_z = select(large_distance, distance_z, abs(direction.z) > 0.000001);
        let next_boundary_distance = min(min(next_x, next_y), next_z);
        if next_boundary_distance >= large_distance * 0.5 {
            return SdfTraceResult(position, vec3<f32>(0.0), travelled, TRACE_TERMINATION_MAX_STEPS, last_sdf_distance);
        }
        travelled += max(next_boundary_distance, 0.0) + minimum_voxel_size * 0.001;
    }
    return SdfTraceResult(position, vec3<f32>(0.0), travelled, TRACE_TERMINATION_MAX_STEPS, last_sdf_distance);
}

fn radiance_lobe_direction(index: u32) -> vec3<f32> {
    let count = 6.0;
    let z = 1.0 - 2.0 * (f32(index) + 0.5) / count;
    let radius = sqrt(max(1.0 - z * z, 0.0));
    let angle = f32(index) * 2.39996323;
    return vec3<f32>(radius * cos(angle), z, radius * sin(angle));
}

fn trace_world_radiance(origin: vec3<f32>, direction: vec3<f32>) -> RadianceTraceResult {
    let voxel_size = (sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz) / vec3<f32>(sdf_clipmap.resolution.xyz);
    let trace = trace_conservative_voxels(origin, direction, 12.0, 96u);
    if trace.termination_reason == TRACE_TERMINATION_HIT {
        let hit_radiance = evaluate_sdf_hit_radiance(trace.position, trace.normal);
        return RadianceTraceResult(max(hit_radiance.radiance, vec3<f32>(0.0)), trace.position, trace.normal, trace.travelled, trace.termination_reason, trace.last_sdf_distance, hit_radiance.lighting_kind);
    }
    if trace.termination_reason == TRACE_TERMINATION_MAX_DISTANCE {
        return RadianceTraceResult(sdf_clipmap.sky_radiance.rgb, trace.position, vec3<f32>(0.0), trace.travelled, trace.termination_reason, trace.last_sdf_distance, LIGHTING_KIND_SKY);
    }
    return RadianceTraceResult(vec3<f32>(0.0), trace.position, vec3<f32>(0.0), trace.travelled, trace.termination_reason, trace.last_sdf_distance, LIGHTING_KIND_UNLIT);
}

fn radiance_field_cell_index(coordinate: vec3<u32>, resolution: vec3<u32>) -> u32 {
    return coordinate.x + coordinate.y * resolution.x + coordinate.z * resolution.x * resolution.y;
}

fn update_radiance_field_cell(invocation: vec3<u32>, field: RadianceFieldData, is_near: bool) {
    let resolution = field.resolution.xyz;
    if any(invocation >= resolution) {
        return;
    }
    let shift = field.scroll_shift.xyz;
    let dirty_x = (shift.x > 0 && invocation.x >= resolution.x - min(u32(shift.x), resolution.x)) || (shift.x < 0 && invocation.x < min(u32(-shift.x), resolution.x));
    let dirty_y = (shift.y > 0 && invocation.y >= resolution.y - min(u32(shift.y), resolution.y)) || (shift.y < 0 && invocation.y < min(u32(-shift.y), resolution.y));
    let dirty_z = (shift.z > 0 && invocation.z >= resolution.z - min(u32(shift.z), resolution.z)) || (shift.z < 0 && invocation.z < min(u32(-shift.z), resolution.z));
    let logical_index = radiance_field_cell_index(invocation, resolution);
    let scheduled_refresh = field.settings.y > 0u && (logical_index + field.settings.x) % field.settings.y == 0u;
    if !dirty_x && !dirty_y && !dirty_z && !scheduled_refresh {
        return;
    }
    let cell_size = (field.maximum.xyz - field.minimum.xyz) / vec3<f32>(resolution);
    let world_position = field.minimum.xyz + (vec3<f32>(invocation) + vec3<f32>(0.5)) * cell_size;
    let physical_coordinate = (invocation + field.grid_offset.xyz) % resolution;
    let physical_index = radiance_field_cell_index(physical_coordinate, resolution) * field.settings.z;
    if sample_sdf_distance(world_position) <= 0.0 {
        for (var direction_index = 0u; direction_index < field.settings.z; direction_index++) {
            let value_index = physical_index + direction_index;
            let debug_index = value_index + select(0u, resolution.x * resolution.y * resolution.z * field.settings.z, !is_near);
            if is_near {
                near_radiance_field_values.values[value_index] = vec4<f32>(0.0);
            } else {
                far_radiance_field_values.values[value_index] = vec4<f32>(0.0);
            }
            radiance_field_trace_debug.values[debug_index] = RadianceFieldTraceDebug(vec4<f32>(world_position, 0.0), vec4<f32>(world_position, 0.0), vec4<f32>(0.0), vec4<f32>(radiance_lobe_direction(direction_index), 0.0));
        }
        return;
    }
    for (var direction_index = 0u; direction_index < field.settings.z; direction_index++) {
        let direction = radiance_lobe_direction(direction_index);
        let trace = trace_world_radiance(world_position + direction * 0.05, direction);
        let radiance = trace.radiance;
        let value_index = physical_index + direction_index;
        var previous = far_radiance_field_values.values[value_index];
        if is_near {
            previous = near_radiance_field_values.values[value_index];
        }
        let smoothed_radiance = mix(previous.rgb, radiance, 0.25);
        let use_history = previous.a > 0.0 && field.settings.w == 0u && !dirty_x && !dirty_y && !dirty_z;
        let stored_radiance = select(radiance, smoothed_radiance, use_history);
        var debug_index = physical_index + direction_index;
        if !is_near {
            debug_index += resolution.x * resolution.y * resolution.z * field.settings.z;
        }
    var debug_normal = trace.hit_normal;
    if trace.termination_reason != TRACE_TERMINATION_HIT {
        debug_normal = vec3<f32>(trace.last_sdf_distance, 0.0, 0.0);
    }
    radiance_field_trace_debug.values[debug_index] = RadianceFieldTraceDebug(vec4<f32>(world_position, 1.0), vec4<f32>(trace.hit_position, f32(trace.lighting_kind)), vec4<f32>(debug_normal, trace.hit_distance), vec4<f32>(direction, f32(trace.termination_reason)));
        let trace_resolved = trace.termination_reason == TRACE_TERMINATION_HIT || trace.termination_reason == TRACE_TERMINATION_MAX_DISTANCE;
        if trace_resolved {
            if is_near {
                near_radiance_field_values.values[value_index] = vec4<f32>(stored_radiance, 1.0);
            } else {
                far_radiance_field_values.values[value_index] = vec4<f32>(stored_radiance, 1.0);
            }
        } else if dirty_x || dirty_y || dirty_z {
            if is_near {
                near_radiance_field_values.values[value_index] = vec4<f32>(0.0);
            } else {
                far_radiance_field_values.values[value_index] = vec4<f32>(0.0);
            }
        }
    }
}

@compute @workgroup_size(4, 4, 4)
fn update_world_radiance_field(@builtin(global_invocation_id) invocation: vec3<u32>) {
    if (surface_probe_settings.cache_settings.w & 0x80000000u) != 0u {
        return;
    }
    update_radiance_field_cell(invocation, near_radiance_field, true);
    update_radiance_field_cell(invocation, far_radiance_field, false);
}

fn reconstruct_world_position(pixel: vec2<u32>, depth: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / vec2<f32>(surface_probe_settings.screen_resolution.zw);
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = camera.inverse_view_projection * clip;
    return world.xyz / max(abs(world.w), 0.00001) * sign(world.w);
}

fn trace_surface_probe_radiance(origin: vec3<f32>, direction: vec3<f32>) -> ProbeRadianceTraceResult {
    let trace = trace_conservative_voxels(origin, direction, 8.0, 96u);
    if trace.termination_reason == TRACE_TERMINATION_HIT {
        let direct_and_emissive = evaluate_sdf_hit_radiance(trace.position, trace.normal).radiance;
        let material = sample_sdf_material(trace.position);
        let indirect = sample_world_radiance_field(trace.position + trace.normal * 0.1, trace.normal) * material;
        return ProbeRadianceTraceResult(max(direct_and_emissive + indirect * 0.6, vec3<f32>(0.0)), trace.termination_reason);
    }
    if trace.termination_reason == TRACE_TERMINATION_MAX_DISTANCE {
        return ProbeRadianceTraceResult(sdf_clipmap.sky_radiance.rgb, trace.termination_reason);
    }
    return ProbeRadianceTraceResult(vec3<f32>(0.0), trace.termination_reason);
}

fn classify_surface_probe(pixel: vec2<u32>, tile: vec2<u32>) -> SurfaceProbeCandidate {
    let is_base_sample = (tile.x % 2u == 0u || surface_probe_settings.screen_resolution.x < 2u) && (tile.y % 2u == 0u || surface_probe_settings.screen_resolution.y < 2u);
    if !is_base_sample {
        return SurfaceProbeCandidate(pixel, 0u, 0.0, 0u);
    }
    let center_depth = textureLoad(scene_depth_texture, vec2<i32>(pixel), 0);
    let center_surface = textureLoad(surface_normal_texture, vec2<i32>(pixel), 0);
    let center_valid = center_depth < 0.999999 && center_surface.a >= 0.5 && dot(center_surface.xyz, center_surface.xyz) >= 0.25;
    var selected_pixel = pixel;
    var selected_valid = center_valid;
    let half_tile = i32(surface_probe_settings.cache_settings.x / 2u) - 1;
    let offsets = array<vec2<i32>, 8>(vec2<i32>(-half_tile, -half_tile), vec2<i32>(0, -half_tile), vec2<i32>(half_tile, -half_tile), vec2<i32>(-half_tile, 0), vec2<i32>(half_tile, 0), vec2<i32>(-half_tile, half_tile), vec2<i32>(0, half_tile), vec2<i32>(half_tile, half_tile));
    for (var sample_index = 0u; sample_index < 8u; sample_index++) {
        let sample_pixel = vec2<u32>(clamp(vec2<i32>(pixel) + offsets[sample_index], vec2<i32>(0), vec2<i32>(surface_probe_settings.screen_resolution.zw) - vec2<i32>(1)));
        let sample_depth = textureLoad(scene_depth_texture, vec2<i32>(sample_pixel), 0);
        let sample_surface = textureLoad(surface_normal_texture, vec2<i32>(sample_pixel), 0);
        let sample_valid = sample_depth < 0.999999 && sample_surface.a >= 0.5 && dot(sample_surface.xyz, sample_surface.xyz) >= 0.25;
        if !selected_valid && sample_valid {
            selected_pixel = sample_pixel;
            selected_valid = true;
        }
    }
    if !selected_valid {
        return SurfaceProbeCandidate(pixel, 0u, 0.0, 0u);
    }
    return SurfaceProbeCandidate(selected_pixel, 0u, 0.0, 1u);
}

fn visible_probe_slot(tile: vec2<u32>) -> u32 {
    let tile_size = surface_probe_settings.screen_resolution.xy;
    let base_grid_size = (tile_size + vec2<u32>(1u)) / 2u;
    let base_count = base_grid_size.x * base_grid_size.y;
    let visible_probe_budget = min(base_count, 3000u);
    let base_index = (tile.y / 2u) * base_grid_size.x + tile.x / 2u;
    if visible_probe_budget >= base_count {
        return base_index;
    }
    let first_slot = base_index * visible_probe_budget / base_count;
    let next_slot = (base_index + 1u) * visible_probe_budget / base_count;
    return select(0xffffffffu, next_slot - 1u, next_slot != first_slot);
}

@compute @workgroup_size(8, 8, 1)
fn update_surface_probes(@builtin(global_invocation_id) invocation: vec3<u32>) {
    if any(invocation.xy >= surface_probe_settings.screen_resolution.xy) {
        return;
    }
    let pixel = min(invocation.xy * surface_probe_settings.cache_settings.x + vec2<u32>(surface_probe_settings.cache_settings.x / 2u), surface_probe_settings.screen_resolution.zw - vec2<u32>(1u));
    let classification = classify_surface_probe(pixel, invocation.xy);
    if classification.valid == 0u {
        return;
    }
    let sample_pixel = classification.pixel;
    let depth = textureLoad(scene_depth_texture, vec2<i32>(sample_pixel), 0);
    let surface = textureLoad(surface_normal_texture, vec2<i32>(sample_pixel), 0);
    let position = reconstruct_world_position(sample_pixel, depth);
    let normal = normalize(surface.xyz);
    let surface_id = textureLoad(surface_id_texture, vec2<i32>(sample_pixel), 0).x;
    let probe_index = visible_probe_slot(invocation.xy);
    if probe_index == 0xffffffffu || probe_index >= surface_probe_settings.cache_settings.y {
        return;
    }
    let previous_position_confidence = surface_probes.values[probe_index].position_confidence;
    let previous_normal_age = surface_probes.values[probe_index].normal_age;
    let voxel_size = (sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz) / vec3<f32>(sdf_clipmap.resolution.xyz);
    let maximum_voxel_size = max(max(voxel_size.x, voxel_size.y), voxel_size.z);
    let same_surface = previous_position_confidence.w > 0.0 && surface_probes.values[probe_index].surface_id == surface_id && dot(previous_normal_age.xyz, previous_normal_age.xyz) > 0.25;
    let anchor_distance = distance(position, previous_position_confidence.xyz);
    let is_existing = same_surface && dot(normal, normalize(previous_normal_age.xyz)) > 0.72 && anchor_distance <= maximum_voxel_size;
    let movement_threshold = max(maximum_voxel_size * 0.25, 0.05);
    let anchor_moved = is_existing && anchor_distance > movement_threshold;
    var reason = classification.reason;
    if !is_existing && reason == 0u {
        reason = 4u;
    }
    var probe_position = position;
    var probe_normal = normal;
    if is_existing {
        if (surface_probe_settings.cache_settings.w & 0x80000000u) != 0u {
            surface_probes.values[probe_index].normal_age.w = 0.0;
            return;
        }
        let update_period = select(16u, select(8u, 4u, reason >= 3u), reason > 0u);
        let frame_index = surface_probe_settings.cache_settings.w & 0x7fffffffu;
        if !anchor_moved && (probe_index + frame_index) % update_period != 0u {
            surface_probes.values[probe_index].position_confidence.w = exp(-anchor_distance * anchor_distance * 8.0);
            return;
        }
    }
    if !is_existing {
        surface_probes.values[probe_index].position_confidence = vec4<f32>(probe_position, 1.0);
        surface_probes.values[probe_index].normal_age = vec4<f32>(probe_normal, 0.0);
        surface_probes.values[probe_index].irradiance = vec4<f32>(0.0);
        surface_probes.values[probe_index].sample_reason = reason;
        surface_probes.values[probe_index].surface_id = surface_id;
    } else {
        surface_probes.values[probe_index].sample_reason = max(surface_probes.values[probe_index].sample_reason, reason);
        surface_probes.values[probe_index].surface_id = surface_id;
    }

    let tangent = normalize(cross(probe_normal, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(probe_normal.y) > 0.9)));
    let bitangent = normalize(cross(probe_normal, tangent));
    let stable_seed = probe_index * 747796405u + 2891336453u;
    let stable_hash = ((stable_seed >> ((stable_seed >> 28u) + 4u)) ^ stable_seed) * 277803737u;
    let random_angle = f32((stable_hash >> 22u) ^ stable_hash) / 4294967295.0 * 6.28318530718;
    let rotated_tangent = tangent * cos(random_angle) + bitangent * sin(random_angle);
    let rotated_bitangent = normalize(cross(normal, rotated_tangent));
    let ray_directions = array<vec3<f32>, 4>(normalize(probe_normal + rotated_tangent * 0.45 + rotated_bitangent * 0.15), normalize(probe_normal - rotated_tangent * 0.35 + rotated_bitangent * 0.5), normalize(probe_normal + rotated_tangent * 0.2 - rotated_bitangent * 0.55), normalize(probe_normal - rotated_tangent * 0.55 - rotated_bitangent * 0.25));
    var traced_radiance = vec3<f32>(0.0);
    var resolved_ray_count = 0u;
    for (var ray_index = 0u; ray_index < 4u; ray_index++) {
        let trace = trace_surface_probe_radiance(probe_position + probe_normal * 0.08, ray_directions[ray_index]);
        if trace.termination_reason == TRACE_TERMINATION_HIT || trace.termination_reason == TRACE_TERMINATION_MAX_DISTANCE {
            traced_radiance += trace.radiance;
            resolved_ray_count += 1u;
        }
    }
    if resolved_ray_count == 0u {
        return;
    }
    traced_radiance /= f32(resolved_ray_count);
    let previous = surface_probes.values[probe_index].irradiance.rgb;
    let old_confidence = surface_probes.values[probe_index].irradiance.a;
    var blend = 1.0;
    if is_existing && old_confidence > 0.0 {
        blend = select(0.65, select(0.8, 0.9, reason >= 3u), reason > 0u);
        if anchor_moved {
            blend = 1.0;
        }
    }
    let result = mix(previous, traced_radiance, blend);
    surface_probes.values[probe_index].position_confidence = vec4<f32>(probe_position, 1.0);
    surface_probes.values[probe_index].normal_age = vec4<f32>(probe_normal, 0.0);
    surface_probes.values[probe_index].irradiance = vec4<f32>(result, 1.0);
}

@compute @workgroup_size(64, 1, 1)
fn age_surface_probes(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let index = invocation.x;
    if index < surface_probe_settings.cache_settings.y {
        let confidence = surface_probes.values[index].position_confidence.w;
        if confidence > 0.0 {
            surface_probes.values[index].normal_age.w += 1.0;
            if surface_probes.values[index].normal_age.w >= 240.0 {
                surface_probes.values[index].position_confidence.w = 0.0;
                surface_probes.values[index].irradiance = vec4<f32>(0.0);
            }
        }
    }
}

fn radiance_cell_visibility_weight(receiver: vec3<f32>, cell_position: vec3<f32>, normal: vec3<f32>, offset_target: bool) -> f32 {
    let origin = receiver + normal * 0.12;
    let target_offset = select(vec3<f32>(0.0), normal * 0.12, offset_target);
    let segment = cell_position + target_offset - origin;
    let segment_length = length(segment);
    if segment_length <= 0.2 {
        return 1.0;
    }
    let direction = segment / segment_length;
    var travelled = 0.04;
    var confidence = 1.0;
    for (var step = 0u; step < 5u; step++) {
        let distance = sample_sdf_distance(origin + direction * travelled);
        confidence = min(confidence, smoothstep(-0.08, 0.24, distance));
        travelled += clamp(max(distance, 0.0) - 0.02, 0.06, 0.5);
        if travelled >= segment_length - 0.05 {
            return confidence;
        }
    }
    return confidence * select(0.75, 1.0, travelled >= segment_length - 0.05);
}

fn sample_radiance_field(position: vec3<f32>, normal: vec3<f32>, field: RadianceFieldData, is_near: bool) -> vec3<f32> {
    if any(position < field.minimum.xyz) || any(position >= field.maximum.xyz) {
        return vec3<f32>(0.0);
    }
    let resolution = field.resolution.xyz;
    let cell_size = (field.maximum.xyz - field.minimum.xyz) / vec3<f32>(resolution);
    let coordinate = (position - field.minimum.xyz) / cell_size - vec3<f32>(0.5);
    let base = vec3<i32>(floor(coordinate));
    let fraction = fract(coordinate);
    var irradiance = vec3<f32>(0.0);
    var total_spatial_weight = 0.0;
    for (var z = 0u; z < 2u; z++) {
        for (var y = 0u; y < 2u; y++) {
            for (var x = 0u; x < 2u; x++) {
                let logical_coordinate = clamp(base + vec3<i32>(i32(x), i32(y), i32(z)), vec3<i32>(0), vec3<i32>(resolution) - vec3<i32>(1));
                let interpolation_weight = select(1.0 - fraction.x, fraction.x, x == 1u) * select(1.0 - fraction.y, fraction.y, y == 1u) * select(1.0 - fraction.z, fraction.z, z == 1u);
                let physical_coordinate = (vec3<u32>(logical_coordinate) + field.grid_offset.xyz) % resolution;
                let cell_index = radiance_field_cell_index(physical_coordinate, resolution) * field.settings.z;
                let sample_position = field.minimum.xyz + (vec3<f32>(logical_coordinate) + vec3<f32>(0.5)) * cell_size;
                let visibility_weight = radiance_cell_visibility_weight(position, sample_position, normal, false);
                var cell_radiance = vec3<f32>(0.0);
                var cell_angular_weight = 0.0;
                var cell_validity = 0.0;
                for (var direction_index = 0u; direction_index < field.settings.z; direction_index++) {
                    var sample = far_radiance_field_values.values[cell_index + direction_index];
                    if is_near {
                        sample = near_radiance_field_values.values[cell_index + direction_index];
                    }
                    let angular_weight = max(dot(normalize(normal), radiance_lobe_direction(direction_index)), 0.0);
                    let weight = angular_weight * sample.a;
                    cell_radiance += sample.rgb * weight;
                    cell_angular_weight += weight;
                    cell_validity = max(cell_validity, sample.a);
                }
                if cell_angular_weight > 0.0001 {
                    // The Fibonacci lobes have equal solid angles, so normalization converts the cosine-weighted sum to diffuse radiance/pi.
                    irradiance += (cell_radiance / cell_angular_weight) * interpolation_weight * visibility_weight;
                    total_spatial_weight += interpolation_weight * visibility_weight * cell_validity;
                }
            }
        }
    }
    return select(vec3<f32>(0.0), irradiance / max(total_spatial_weight, 0.0001), total_spatial_weight > 0.0001);
}

fn sample_world_radiance_field(position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    let near_inside = all(position >= near_radiance_field.minimum.xyz) && all(position < near_radiance_field.maximum.xyz);
    if !near_inside {
        let far_radiance = sample_radiance_field(position, normal, far_radiance_field, false);
        return select(sdf_clipmap.sky_radiance.rgb, far_radiance, dot(far_radiance, far_radiance) > 0.000001);
    }
    let near_radiance = sample_radiance_field(position, normal, near_radiance_field, true);
    let near_value = select(sdf_clipmap.sky_radiance.rgb, near_radiance, dot(near_radiance, near_radiance) > 0.000001);
    let edge_distance = min(min(min(position.x - near_radiance_field.minimum.x, near_radiance_field.maximum.x - position.x), min(position.y - near_radiance_field.minimum.y, near_radiance_field.maximum.y - position.y)), min(position.z - near_radiance_field.minimum.z, near_radiance_field.maximum.z - position.z));
    let near_weight = smoothstep(0.5, 2.0, edge_distance);
    if near_weight >= 1.0 {
        return near_value;
    }
    let far_radiance = sample_radiance_field(position, normal, far_radiance_field, false);
    let far_value = select(sdf_clipmap.sky_radiance.rgb, far_radiance, dot(far_radiance, far_radiance) > 0.000001);
    return mix(far_value, near_value, near_weight);
}

fn sample_surface_probe_field(position: vec3<f32>, normal: vec3<f32>, surface_id: u32, pixel: vec2<u32>) -> SurfaceProbeLighting {
    var nearest_indices = array<u32, 12>(0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu, 0xffffffffu);
    var nearest_distances = array<f32, 12>(1e30, 1e30, 1e30, 1e30, 1e30, 1e30, 1e30, 1e30, 1e30, 1e30, 1e30, 1e30);
    let base_grid_size = (surface_probe_settings.screen_resolution.xy + vec2<u32>(1u)) / 2u;
    let screen_probe_spacing = surface_probe_settings.cache_settings.x * 2u;
    let tile_coordinate = vec2<i32>(pixel / screen_probe_spacing);
    for (var tile_y = -2; tile_y <= 2; tile_y++) {
        for (var tile_x = -2; tile_x <= 2; tile_x++) {
            let neighbor_tile = tile_coordinate + vec2<i32>(tile_x, tile_y);
            if any(neighbor_tile < vec2<i32>(0)) || any(neighbor_tile >= vec2<i32>(base_grid_size)) {
                continue;
            }
            let probe_index = visible_probe_slot(vec2<u32>(neighbor_tile) * 2u);
            if probe_index == 0xffffffffu || probe_index >= surface_probe_settings.cache_settings.y || surface_probes.values[probe_index].position_confidence.w <= 0.0 || surface_probes.values[probe_index].surface_id != surface_id {
                continue;
            }
            let probe_position = surface_probes.values[probe_index].position_confidence.xyz;
            let delta = position - probe_position;
            let plane_distance = dot(delta, normal);
            let tangent_delta = delta - normal * plane_distance;
            let tangent_distance = length(tangent_delta);
            let probe_normal = normalize(surface_probes.values[probe_index].normal_age.xyz);
            let normal_alignment = dot(normal, probe_normal);
            let plane_error = abs(plane_distance);
            if normal_alignment <= 0.1 {
                continue;
            }
            let candidate_distance = tangent_distance * tangent_distance + plane_error * plane_error * 4.0;
            var insert_at = 12u;
            for (var slot = 0u; slot < 12u; slot++) {
                if insert_at == 12u && candidate_distance < nearest_distances[slot] {
                    insert_at = slot;
                }
            }
            if insert_at < 12u {
                var shift = 11u;
                loop {
                    if shift <= insert_at {
                        break;
                    }
                    nearest_distances[shift] = nearest_distances[shift - 1u];
                    nearest_indices[shift] = nearest_indices[shift - 1u];
                    shift -= 1u;
                }
                nearest_distances[insert_at] = candidate_distance;
                nearest_indices[insert_at] = probe_index;
            }
        }
    }
    var local_radiance = vec3<f32>(0.0);
    var total_weight = 0.0;
    for (var slot = 0u; slot < 12u; slot++) {
        let probe_index = nearest_indices[slot];
        if probe_index != 0xffffffffu {
            let probe_position = surface_probes.values[probe_index].position_confidence.xyz;
            let delta = position - probe_position;
            let plane_distance = dot(delta, normal);
            let tangent_delta = delta - normal * plane_distance;
            let tangent_distance = length(tangent_delta);
            let probe_normal = normalize(surface_probes.values[probe_index].normal_age.xyz);
            let plane_error = abs(plane_distance);
            let normal_weight = smoothstep(0.1, 0.9, dot(normal, probe_normal));
            let plane_weight = 1.0 - smoothstep(0.35, 0.9, plane_error);
            let visibility = radiance_cell_visibility_weight(position, probe_position, normal, true);
            let distance_weight = exp(-tangent_distance * tangent_distance * 0.8 - plane_error * plane_error * 2.5);
            let age_weight = 1.0 - clamp(surface_probes.values[probe_index].normal_age.w / 240.0, 0.0, 1.0);
            let weight = distance_weight * normal_weight * plane_weight * visibility * age_weight * surface_probes.values[probe_index].position_confidence.w * surface_probes.values[probe_index].irradiance.a;
            local_radiance += surface_probes.values[probe_index].irradiance.rgb * weight;
            total_weight += weight;
        }
    }
    let support = smoothstep(0.01, 0.3, total_weight);
    let local_value = local_radiance / max(total_weight, 0.0001);
    return SurfaceProbeLighting(local_value, support);
}

fn sample_sdf_material(position: vec3<f32>) -> vec3<f32> {
    let normalized_position = (position - sdf_clipmap.minimum.xyz) / (sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz);
    if any(normalized_position < vec3<f32>(0.0)) || any(normalized_position >= vec3<f32>(1.0)) {
        return vec3<f32>(0.0);
    }
    let coordinate = clamp(vec3<i32>(normalized_position * vec3<f32>(sdf_clipmap.resolution.xyz)), vec3<i32>(0), vec3<i32>(sdf_clipmap.resolution.xyz) - vec3<i32>(1));
    return textureLoad(sdf_clipmap_texture, sdf_physical_coordinate(coordinate), 0).yzw;
}

fn sample_sdf_emission(position: vec3<f32>) -> vec3<f32> {
    let normalized_position = (position - sdf_clipmap.minimum.xyz) / (sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz);
    if any(normalized_position < vec3<f32>(0.0)) || any(normalized_position >= vec3<f32>(1.0)) {
        return vec3<f32>(0.0);
    }
    let coordinate = clamp(vec3<i32>(normalized_position * vec3<f32>(sdf_clipmap.resolution.xyz)), vec3<i32>(0), vec3<i32>(sdf_clipmap.resolution.xyz) - vec3<i32>(1));
    return textureLoad(sdf_emission_texture, sdf_physical_coordinate(coordinate), 0).rgb;
}

fn sdf_physical_coordinate(logical_coordinate: vec3<i32>) -> vec3<i32> {
    let dimensions = vec3<i32>(sdf_clipmap.resolution.xyz);
    return (logical_coordinate + vec3<i32>(sdf_clipmap.grid_offset.xyz)) % dimensions;
}

fn trace_sdf_visibility(origin: vec3<f32>, direction: vec3<f32>, maximum_distance: f32) -> f32 {
    let trace = trace_conservative_voxels(origin, direction, maximum_distance, 96u);
    if trace.termination_reason == TRACE_TERMINATION_MAX_DISTANCE {
        return 1.0;
    }
    return 0.0;
}

fn evaluate_sdf_hit_radiance(position: vec3<f32>, normal: vec3<f32>) -> SdfHitRadianceResult {
    let voxel_size = (sdf_clipmap.maximum.xyz - sdf_clipmap.minimum.xyz) / vec3<f32>(sdf_clipmap.resolution.xyz);
    let normal_direction = normalize(normal);
    let surface_offset = min(min(voxel_size.x, voxel_size.y), voxel_size.z) * 0.15;
    let origin = position + normal_direction * surface_offset;
    let albedo = sample_sdf_material(position);
    var direct_radiance = vec3<f32>(0.0);
    var has_visible_direct_light = false;
    var has_shadowed_direct_light = false;
    for (var index = 0u; index < camera.light_count.x; index++) {
        var light_direction = normalize(camera.light_directions[index].xyz);
        var attenuation = 1.0;
        var maximum_distance = 12.0;
        if camera.light_kinds[index].x == 1u || camera.light_kinds[index].x == 2u {
            let to_light = camera.light_positions[index].xyz - origin;
            maximum_distance = length(to_light);
            light_direction = normalize(to_light);
            attenuation = pow(max(1.0 - maximum_distance / max(camera.light_positions[index].w, 0.001), 0.0), 2.0);
            if camera.light_kinds[index].x == 2u {
                let cone = dot(normalize(-to_light), normalize(camera.light_directions[index].xyz));
                let cone_range = max(camera.light_params[index].y - camera.light_params[index].z, 0.001);
                attenuation *= clamp((cone - camera.light_params[index].z) / cone_range, 0.0, 1.0);
            }
        }
        let diffuse = max(dot(normal_direction, light_direction), 0.0) * attenuation;
        if diffuse > 0.0 && camera.light_colors[index].a > 0.0 {
            let visibility = trace_sdf_visibility(origin, light_direction, maximum_distance);
            direct_radiance += camera.light_colors[index].rgb * camera.light_colors[index].a * diffuse * visibility;
            has_visible_direct_light = has_visible_direct_light || visibility > 0.0;
            has_shadowed_direct_light = has_shadowed_direct_light || visibility <= 0.0;
        }
    }
    let emissive_radiance = sample_sdf_emission(position);
    let direct_strength = max(max(direct_radiance.x, direct_radiance.y), direct_radiance.z);
    let emissive_strength = max(max(emissive_radiance.x, emissive_radiance.y), emissive_radiance.z);
    var lighting_kind = LIGHTING_KIND_UNLIT;
    if emissive_strength > direct_strength && emissive_strength > 0.0001 {
        lighting_kind = LIGHTING_KIND_EMISSIVE;
    } else if has_visible_direct_light && direct_strength > 0.0001 {
        lighting_kind = LIGHTING_KIND_DIRECT;
    } else if has_shadowed_direct_light {
        lighting_kind = LIGHTING_KIND_SHADOWED;
    }
    return SdfHitRadianceResult(albedo * direct_radiance + emissive_radiance, lighting_kind);
}
