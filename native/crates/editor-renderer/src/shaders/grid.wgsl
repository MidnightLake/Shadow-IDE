struct CameraUniforms {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) near_point: vec3<f32>,
    @location(1) far_point: vec3<f32>,
};

// Fullscreen quad positions
var<private> positions: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(-1.0, -1.0, 0.0),
    vec3<f32>( 1.0, -1.0, 0.0),
    vec3<f32>( 1.0,  1.0, 0.0),
    vec3<f32>(-1.0, -1.0, 0.0),
    vec3<f32>( 1.0,  1.0, 0.0),
    vec3<f32>(-1.0,  1.0, 0.0),
);

fn unproject_point(p: vec3<f32>, inv_view_proj: mat4x4<f32>) -> vec3<f32> {
    let unprojected = inv_view_proj * vec4<f32>(p, 1.0);
    return unprojected.xyz / unprojected.w;
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    let p = positions[idx];
    let inv_vp = camera.view_proj;
    // We need the inverse — compute manually using the adjugate or pass it.
    // For simplicity, we compute the grid in world space using a large plane approach.
    var out: VertexOutput;

    // Large ground plane from -500 to +500
    let scale = 500.0;
    var world_pos: vec3<f32>;
    if idx == 0u { world_pos = vec3<f32>(-scale, 0.0, -scale); }
    else if idx == 1u { world_pos = vec3<f32>( scale, 0.0, -scale); }
    else if idx == 2u { world_pos = vec3<f32>( scale, 0.0,  scale); }
    else if idx == 3u { world_pos = vec3<f32>(-scale, 0.0, -scale); }
    else if idx == 4u { world_pos = vec3<f32>( scale, 0.0,  scale); }
    else { world_pos = vec3<f32>(-scale, 0.0,  scale); }

    out.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.near_point = world_pos;
    out.far_point = camera.camera_pos.xyz;
    return out;
}

fn grid(pos: vec3<f32>, scale: f32) -> vec4<f32> {
    let coord = pos.xz * scale;
    let derivative = fwidth(coord);
    let grid_val = abs(fract(coord - 0.5) - 0.5) / derivative;
    let line = min(grid_val.x, grid_val.y);
    let minimumz = min(derivative.y, 1.0);
    let minimumx = min(derivative.x, 1.0);
    var color = vec4<f32>(0.35, 0.4, 0.45, 1.0 - min(line, 1.0));

    // X axis (red)
    if pos.z > -0.1 * minimumz && pos.z < 0.1 * minimumz {
        color = vec4<f32>(0.9, 0.2, 0.2, 1.0);
    }
    // Z axis (blue)
    if pos.x > -0.1 * minimumx && pos.x < 0.1 * minimumx {
        color = vec4<f32>(0.2, 0.3, 0.9, 1.0);
    }

    return color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let pos = in.near_point;

    // Distance fade
    let dist = length(camera.camera_pos.xyz - pos);
    let fade = 1.0 - smoothstep(40.0, 200.0, dist);

    // Two grid scales
    let grid1 = grid(pos, 1.0);   // 1m grid
    let grid10 = grid(pos, 0.1);  // 10m grid

    var color = grid1;
    color.a = max(color.a, grid10.a * 0.6);
    color.a *= fade * 0.7;

    if color.a < 0.01 {
        discard;
    }

    return color;
}
