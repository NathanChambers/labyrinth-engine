struct Camera {
    view_projection: mat4x4<f32>,
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
    probes: array<vec4<f32>, 1024>,
    directions: array<vec4<f32>, 1024>,
    volume_count: vec4<u32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(0) @binding(1)
var shadow_texture: texture_depth_2d_array;

@group(0) @binding(2)
var shadow_sampler: sampler_comparison;

@group(0) @binding(3)
var<uniform> irradiance: IrradianceData;

@group(0) @binding(4)
var spot_shadow_texture: texture_depth_2d;

@group(0) @binding(5)
var spot_shadow_sampler: sampler_comparison;

@group(0) @binding(6)
var point_shadow_texture: texture_depth_2d_array;

@group(0) @binding(7)
var point_shadow_sampler: sampler_comparison;

@group(1) @binding(0)
var scene_depth_texture: texture_depth_2d;

@group(1) @binding(1)
var scene_depth_sampler: sampler;


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
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = input.color.rgb * input.material_base_color.rgb;
    let emission = input.material_emission.rgb * input.material_params.w;
    if camera.debug_mode.x == 1u || camera.debug_mode.x == 2u {
        return vec4<f32>(albedo + emission, input.color.a * input.material_base_color.a);
    }
    let normal = normalize(input.normal);
    let ambient_occlusion = select(1.0, sample_ambient_occlusion(input.clip_position), camera.shadow_flags.z == 1u);
    var indirect = vec3<f32>(0.0);
    if camera.debug_mode.z == 1u {
        indirect = sample_irradiance(input.world_position, normal) * ambient_occlusion;
    }
    if camera.debug_mode.x == 4u {
        return vec4<f32>(max(indirect, vec3<f32>(0.0)), 1.0);
    }
    let metallic = input.material_params.x;
    let roughness = input.material_params.y;
    let view_direction = normalize(camera.camera_position.xyz - input.world_position);
    var lighting = vec3<f32>(0.02, 0.02, 0.02) * ambient_occlusion;
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
        return vec4<f32>(vec3<f32>(direct_visibility), 1.0);
    }
    let color = vec4<f32>(final_color, input.color.a * input.material_base_color.a);
    return color;
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
    var direction = vec3<f32>(0.0);
    var direction_weight = 0.0;
    for (var volume_index = 0u; volume_index < irradiance.volume_count.x; volume_index++) {
        let volume = irradiance.volumes[volume_index];
        if (position.x < volume.minimum.x || position.y < volume.minimum.y || position.z < volume.minimum.z || position.x > volume.maximum.x || position.y > volume.maximum.y || position.z > volume.maximum.z) {
            continue;
        }
        let size = max(volume.maximum.xyz - volume.minimum.xyz, vec3<f32>(0.001));
        let coordinate = clamp((position - volume.minimum.xyz) / size, vec3<f32>(0.0), vec3<f32>(1.0)) * vec3<f32>(volume.resolution.xyz - vec3<u32>(1u));
        let base = vec3<u32>(floor(coordinate));
        let fraction = fract(coordinate);
        let maximum = volume.resolution.xyz - vec3<u32>(1u);
        for (var z = 0u; z <= 1u; z++) {
            for (var y = 0u; y <= 1u; y++) {
                for (var x = 0u; x <= 1u; x++) {
                    let sample_coordinate = min(base + vec3<u32>(x, y, z), maximum);
                    let index = sample_coordinate.x + sample_coordinate.y * volume.resolution.x + sample_coordinate.z * volume.resolution.x * volume.resolution.y;
                    let weight = select(1.0 - fraction.x, fraction.x, x == 1u) * select(1.0 - fraction.y, fraction.y, y == 1u) * select(1.0 - fraction.z, fraction.z, z == 1u);
                    result += irradiance.probes[volume.probe_offset + index].rgb * weight;
                    direction += irradiance.directions[volume.probe_offset + index].xyz * irradiance.directions[volume.probe_offset + index].w * weight;
                    direction_weight += irradiance.directions[volume.probe_offset + index].w * weight;
                }
            }
        }
        if direction_weight > 0.0 {
            let direction_alignment = max(dot(normal, normalize(direction)), 0.0);
            result *= 1.0 + direction_alignment * min(direction_weight, 1.0) * 0.35;
        }
        break;
    }
    return result;
}
