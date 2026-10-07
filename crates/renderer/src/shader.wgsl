struct Camera {
    view_projection: mat4x4<f32>,
    shadow_view_projection: mat4x4<f32>,
    spot_shadow_view_projection: mat4x4<f32>,
    light_directions: array<vec4<f32>, 8>,
    light_colors: array<vec4<f32>, 8>,
    light_positions: array<vec4<f32>, 8>,
    light_params: array<vec4<f32>, 8>,
    light_kinds: array<vec4<u32>, 8>,
    light_count: vec4<u32>,
    camera_position: vec4<f32>,
    debug_mode: vec4<u32>,
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
var shadow_texture: texture_depth_2d;

@group(0) @binding(2)
var shadow_sampler: sampler_comparison;

@group(0) @binding(3)
var<uniform> irradiance: IrradianceData;

@group(0) @binding(4)
var spot_shadow_texture: texture_depth_2d;

@group(0) @binding(5)
var spot_shadow_sampler: sampler_comparison;

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
    @location(2) shadow_position: vec4<f32>,
    @location(3) spot_shadow_position: vec4<f32>,
    @location(4) world_position: vec3<f32>,
    @location(5) material_base_color: vec4<f32>,
    @location(6) material_params: vec4<f32>,
    @location(7) material_emission: vec4<f32>,
    @location(8) indirect: vec3<f32>,
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
    output.shadow_position = camera.shadow_view_projection * vec4<f32>(input.position, 1.0);
    output.spot_shadow_position = camera.spot_shadow_view_projection * vec4<f32>(input.position, 1.0);
    output.world_position = input.position;
    output.indirect = vec3<f32>(0.0);
    if camera.debug_mode.z == 1u {
        output.indirect = sample_irradiance(input.position, normalize(input.normal));
    }
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
    let metallic = input.material_params.x;
    let roughness = input.material_params.y;
    let view_direction = normalize(camera.camera_position.xyz - input.world_position);
    var lighting = vec3<f32>(0.02, 0.02, 0.02);
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
        if (camera.debug_mode.y == 1u && index == 0u && camera.light_kinds[index].x == 0u) {
            let projected = input.shadow_position.xyz / input.shadow_position.w;
            if (projected.x >= -1.0 && projected.x <= 1.0 && projected.y >= -1.0 && projected.y <= 1.0 && projected.z >= 0.0 && projected.z <= 1.0) {
                let shadow_uv = vec2<f32>(projected.x * 0.5 + 0.5, 1.0 - (projected.y * 0.5 + 0.5));
                let texel_size = 1.0 / 2048.0;
                var visibility = 0.0;
                for (var offset_y = -1; offset_y <= 1; offset_y++) {
                    for (var offset_x = -1; offset_x <= 1; offset_x++) {
                        let offset = vec2<f32>(f32(offset_x), f32(offset_y)) * texel_size;
                        visibility += textureSampleCompare(shadow_texture, shadow_sampler, shadow_uv + offset, projected.z - 0.0001);
                    }
                }
                shadow = visibility / 9.0;
            }
        } else if (camera.debug_mode.y == 1u && camera.light_kinds[index].x == 2u) {
            let projected = input.spot_shadow_position.xyz / input.spot_shadow_position.w;
            if (projected.x >= -1.0 && projected.x <= 1.0 && projected.y >= -1.0 && projected.y <= 1.0 && projected.z >= 0.0 && projected.z <= 1.0) {
                let shadow_uv = vec2<f32>(projected.x * 0.5 + 0.5, 1.0 - (projected.y * 0.5 + 0.5));
                let texel_size = 1.0 / 2048.0;
                var visibility = 0.0;
                for (var offset_y = -1; offset_y <= 1; offset_y++) {
                    for (var offset_x = -1; offset_x <= 1; offset_x++) {
                        let offset = vec2<f32>(f32(offset_x), f32(offset_y)) * texel_size;
                        visibility += textureSampleCompare(spot_shadow_texture, spot_shadow_sampler, shadow_uv + offset, projected.z - 0.0005);
                    }
                }
                shadow = visibility / 9.0;
            }
        }
        let half_direction = normalize(light_direction + view_direction);
        let specular_power = mix(64.0, 4.0, roughness);
        let specular = pow(max(dot(normal, half_direction), 0.0), specular_power) * (1.0 - metallic) * 0.04 + metallic * pow(max(dot(normal, half_direction), 0.0), specular_power);
        lighting += camera.light_colors[index].rgb * camera.light_colors[index].a * (diffuse + specular) * shadow;
    }
    let indirect = input.indirect;
    let final_color = albedo * (lighting + indirect) + emission;
    let color = vec4<f32>(final_color, input.color.a * input.material_base_color.a);
    return color;
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
