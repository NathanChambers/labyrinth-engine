struct Camera {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(3) model_0: vec4<f32>,
    @location(4) model_1: vec4<f32>,
    @location(5) model_2: vec4<f32>,
    @location(6) model_3: vec4<f32>,
    @location(7) normal_0: vec4<f32>,
    @location(8) normal_1: vec4<f32>,
    @location(9) normal_2: vec4<f32>,
    @location(13) surface_id: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) @interpolate(flat) surface_id: u32,
};

struct FragmentOutput {
    @location(0) world_normal: vec4<f32>,
    @location(1) surface_id: u32,
};

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    let normal_matrix = mat3x3<f32>(input.normal_0.xyz, input.normal_1.xyz, input.normal_2.xyz);
    output.clip_position = camera.view_projection * model * vec4<f32>(input.position, 1.0);
    output.world_normal = normalize(normal_matrix * input.normal);
    output.surface_id = input.surface_id;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    return FragmentOutput(vec4<f32>(normalize(input.world_normal), 1.0), input.surface_id);
}
