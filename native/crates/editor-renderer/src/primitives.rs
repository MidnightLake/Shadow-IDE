use crate::mesh::PbrVertex;

pub fn cube_vertices(color: [f32; 4]) -> (Vec<PbrVertex>, Vec<u32>) {
    let c = color;
    #[rustfmt::skip]
    let vertices = vec![
        // Front (+Z)
        PbrVertex { position: [-0.5, -0.5,  0.5], normal: [0.0, 0.0, 1.0], color: c },
        PbrVertex { position: [ 0.5, -0.5,  0.5], normal: [0.0, 0.0, 1.0], color: c },
        PbrVertex { position: [ 0.5,  0.5,  0.5], normal: [0.0, 0.0, 1.0], color: c },
        PbrVertex { position: [-0.5,  0.5,  0.5], normal: [0.0, 0.0, 1.0], color: c },
        // Back (-Z)
        PbrVertex { position: [ 0.5, -0.5, -0.5], normal: [0.0, 0.0, -1.0], color: c },
        PbrVertex { position: [-0.5, -0.5, -0.5], normal: [0.0, 0.0, -1.0], color: c },
        PbrVertex { position: [-0.5,  0.5, -0.5], normal: [0.0, 0.0, -1.0], color: c },
        PbrVertex { position: [ 0.5,  0.5, -0.5], normal: [0.0, 0.0, -1.0], color: c },
        // Top (+Y)
        PbrVertex { position: [-0.5,  0.5,  0.5], normal: [0.0, 1.0, 0.0], color: c },
        PbrVertex { position: [ 0.5,  0.5,  0.5], normal: [0.0, 1.0, 0.0], color: c },
        PbrVertex { position: [ 0.5,  0.5, -0.5], normal: [0.0, 1.0, 0.0], color: c },
        PbrVertex { position: [-0.5,  0.5, -0.5], normal: [0.0, 1.0, 0.0], color: c },
        // Bottom (-Y)
        PbrVertex { position: [-0.5, -0.5, -0.5], normal: [0.0, -1.0, 0.0], color: c },
        PbrVertex { position: [ 0.5, -0.5, -0.5], normal: [0.0, -1.0, 0.0], color: c },
        PbrVertex { position: [ 0.5, -0.5,  0.5], normal: [0.0, -1.0, 0.0], color: c },
        PbrVertex { position: [-0.5, -0.5,  0.5], normal: [0.0, -1.0, 0.0], color: c },
        // Right (+X)
        PbrVertex { position: [ 0.5, -0.5,  0.5], normal: [1.0, 0.0, 0.0], color: c },
        PbrVertex { position: [ 0.5, -0.5, -0.5], normal: [1.0, 0.0, 0.0], color: c },
        PbrVertex { position: [ 0.5,  0.5, -0.5], normal: [1.0, 0.0, 0.0], color: c },
        PbrVertex { position: [ 0.5,  0.5,  0.5], normal: [1.0, 0.0, 0.0], color: c },
        // Left (-X)
        PbrVertex { position: [-0.5, -0.5, -0.5], normal: [-1.0, 0.0, 0.0], color: c },
        PbrVertex { position: [-0.5, -0.5,  0.5], normal: [-1.0, 0.0, 0.0], color: c },
        PbrVertex { position: [-0.5,  0.5,  0.5], normal: [-1.0, 0.0, 0.0], color: c },
        PbrVertex { position: [-0.5,  0.5, -0.5], normal: [-1.0, 0.0, 0.0], color: c },
    ];

    let mut indices = Vec::new();
    for face in 0..6u32 {
        let base = face * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    (vertices, indices)
}

pub fn sphere_vertices(color: [f32; 4], rings: u32, segments: u32) -> (Vec<PbrVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    for ring in 0..=rings {
        let phi = std::f32::consts::PI * ring as f32 / rings as f32;
        let y = phi.cos();
        let r = phi.sin();

        for seg in 0..=segments {
            let theta = 2.0 * std::f32::consts::PI * seg as f32 / segments as f32;
            let x = r * theta.cos();
            let z = r * theta.sin();

            vertices.push(PbrVertex {
                position: [x * 0.5, y * 0.5, z * 0.5],
                normal: [x, y, z],
                color,
            });
        }
    }

    for ring in 0..rings {
        for seg in 0..segments {
            let a = ring * (segments + 1) + seg;
            let b = a + segments + 1;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }

    (vertices, indices)
}

pub fn plane_vertices(color: [f32; 4], size: f32) -> (Vec<PbrVertex>, Vec<u32>) {
    let h = size * 0.5;
    let vertices = vec![
        PbrVertex { position: [-h, 0.0, -h], normal: [0.0, 1.0, 0.0], color },
        PbrVertex { position: [ h, 0.0, -h], normal: [0.0, 1.0, 0.0], color },
        PbrVertex { position: [ h, 0.0,  h], normal: [0.0, 1.0, 0.0], color },
        PbrVertex { position: [-h, 0.0,  h], normal: [0.0, 1.0, 0.0], color },
    ];
    let indices = vec![0, 1, 2, 0, 2, 3];
    (vertices, indices)
}

pub fn arrow_vertices(color: [f32; 4], length: f32, radius: f32) -> (Vec<PbrVertex>, Vec<u32>) {
    let segments = 8u32;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();

    // Shaft (cylinder along Y)
    let shaft_len = length * 0.75;
    for i in 0..=segments {
        let theta = 2.0 * std::f32::consts::PI * i as f32 / segments as f32;
        let x = radius * theta.cos();
        let z = radius * theta.sin();
        let n = [theta.cos(), 0.0, theta.sin()];
        vertices.push(PbrVertex { position: [x, 0.0, z], normal: n, color });
        vertices.push(PbrVertex { position: [x, shaft_len, z], normal: n, color });
    }
    for i in 0..segments {
        let a = i * 2;
        let b = a + 2;
        indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
    }

    // Cone tip
    let base_offset = vertices.len() as u32;
    let cone_radius = radius * 2.5;
    let tip_idx = base_offset + segments + 1;
    for i in 0..=segments {
        let theta = 2.0 * std::f32::consts::PI * i as f32 / segments as f32;
        let x = cone_radius * theta.cos();
        let z = cone_radius * theta.sin();
        vertices.push(PbrVertex { position: [x, shaft_len, z], normal: [theta.cos(), 0.3, theta.sin()], color });
    }
    vertices.push(PbrVertex { position: [0.0, length, 0.0], normal: [0.0, 1.0, 0.0], color });
    for i in 0..segments {
        indices.extend_from_slice(&[base_offset + i, base_offset + i + 1, tip_idx]);
    }

    (vertices, indices)
}
