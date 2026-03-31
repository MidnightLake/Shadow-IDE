struct CameraUniforms {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

struct ModelUniforms {
    model: mat4x4<f32>,
    color: vec4<f32>,
    selected: f32,
    _pad: vec3<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var<uniform> model: ModelUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_normal: vec3<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = model.model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.world_position = world_pos.xyz;
    let normal_mat = mat3x3<f32>(model.model[0].xyz, model.model[1].xyz, model.model[2].xyz);
    out.world_normal = normalize(normal_mat * in.normal);
    out.color = in.color * model.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let light_dir = normalize(vec3<f32>(0.4, 0.8, 0.3));
    let light_color = vec3<f32>(1.0, 0.98, 0.95);
    let ambient = vec3<f32>(0.15, 0.17, 0.22);

    let n = normalize(in.world_normal);
    let ndotl = max(dot(n, light_dir), 0.0);

    let view_dir = normalize(camera.camera_pos.xyz - in.world_position);
    let half_dir = normalize(light_dir + view_dir);
    let spec = pow(max(dot(n, half_dir), 0.0), 32.0) * 0.3;

    let diffuse = in.color.rgb * light_color * ndotl;
    let color = ambient * in.color.rgb + diffuse + spec * light_color;

    // Selection highlight
    let sel = model.selected;
    let highlight = vec3<f32>(0.91, 0.67, 0.37) * sel * 0.15;

    return vec4<f32>(color + highlight, in.color.a);
}
