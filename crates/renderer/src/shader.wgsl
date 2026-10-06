struct Camera {
    view_projection: mat4x4<f32>,
    light_directions: array<vec4<f32>, 8>,
    light_colors: array<vec4<f32>, 8>,
    light_count: vec4<u32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.normal = input.normal;
    output.color = input.color;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    var lighting = vec3<f32>(0.25, 0.25, 0.25);
    for (var index = 0u; index < camera.light_count.x; index++) {
        let diffuse = max(dot(normal, normalize(camera.light_directions[index].xyz)), 0.0);
        lighting += camera.light_colors[index].rgb * camera.light_colors[index].a * diffuse;
    }
    return vec4<f32>(input.color.rgb * lighting, input.color.a);
}
