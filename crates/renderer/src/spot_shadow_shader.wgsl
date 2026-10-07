struct ShadowUniform {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> shadow: ShadowUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

@vertex
fn vertex_main(input: VertexInput) -> @builtin(position) vec4<f32> {
    return shadow.view_projection * vec4<f32>(input.position, 1.0);
}
