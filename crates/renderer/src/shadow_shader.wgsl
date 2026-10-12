struct ShadowUniform {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> shadow: ShadowUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(3) model_0: vec4<f32>,
    @location(4) model_1: vec4<f32>,
    @location(5) model_2: vec4<f32>,
    @location(6) model_3: vec4<f32>,
};

@vertex
fn vertex_main(input: VertexInput) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(input.model_0, input.model_1, input.model_2, input.model_3);
    return shadow.view_projection * model * vec4<f32>(input.position, 1.0);
}
