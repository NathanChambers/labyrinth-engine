struct Camera {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(6) surface_id: u32,
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
    output.clip_position = camera.view_projection * vec4<f32>(input.position, 1.0);
    output.world_normal = input.normal;
    output.surface_id = input.surface_id;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    return FragmentOutput(vec4<f32>(normalize(input.world_normal), 1.0), input.surface_id);
}
