use std::f32::consts::{PI, TAU};

use math::Color;
use mesh::{Mesh, MeshVertex};

pub fn plane(size: f32, color: Color) -> Mesh {
    quad(size, size, 0.0, color)
}

pub fn quad(width: f32, depth: f32, y: f32, color: Color) -> Mesh {
    let half_width = width * 0.5;
    let half_depth = depth * 0.5;
    let positions = [[-half_width, y, -half_depth], [-half_width, y, half_depth], [half_width, y, half_depth], [half_width, y, -half_depth]];
    let mut mesh = Mesh::new();
    mesh.add_quad(positions.map(|position| vertex(position, [0.0, 1.0, 0.0], color)));
    mesh
}

pub fn cube(center: [f32; 3], size: f32, color: Color) -> Mesh {
    let half_size = size * 0.5;
    let faces = [
        ([0.0, 0.0, 1.0], [[-half_size, -half_size, half_size], [half_size, -half_size, half_size], [half_size, half_size, half_size], [-half_size, half_size, half_size]]),
        ([0.0, 0.0, -1.0], [[half_size, -half_size, -half_size], [-half_size, -half_size, -half_size], [-half_size, half_size, -half_size], [half_size, half_size, -half_size]]),
        ([-1.0, 0.0, 0.0], [[-half_size, -half_size, -half_size], [-half_size, -half_size, half_size], [-half_size, half_size, half_size], [-half_size, half_size, -half_size]]),
        ([1.0, 0.0, 0.0], [[half_size, -half_size, half_size], [half_size, -half_size, -half_size], [half_size, half_size, -half_size], [half_size, half_size, half_size]]),
        ([0.0, 1.0, 0.0], [[-half_size, half_size, half_size], [half_size, half_size, half_size], [half_size, half_size, -half_size], [-half_size, half_size, -half_size]]),
        ([0.0, -1.0, 0.0], [[-half_size, -half_size, -half_size], [half_size, -half_size, -half_size], [half_size, -half_size, half_size], [-half_size, -half_size, half_size]]),
    ];
    let mut mesh = Mesh::new();
    for (normal, positions) in faces {
        let translated = positions.map(|position| [position[0] + center[0], position[1] + center[1], position[2] + center[2]]);
        mesh.add_quad(translated.map(|position| vertex(position, normal, color)));
    }
    mesh
}

pub fn sphere(center: [f32; 3], radius: f32, color: Color, segments: u32, rings: u32) -> Mesh {
    assert!(segments >= 3 && rings >= 2, "sphere needs at least 3 segments and 2 rings");
    let mut mesh = Mesh::new();
    let mut vertices = vec![vec![0_u32; (segments + 1) as usize]; (rings + 1) as usize];
    for ring in 0..=rings {
        let phi = PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let theta = TAU * segment as f32 / segments as f32;
            let normal = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
            let position = [center[0] + radius * normal[0], center[1] + radius * normal[1], center[2] + radius * normal[2]];
            vertices[ring as usize][segment as usize] = mesh.add_vertex(vertex(position, normal, color));
        }
    }
    add_grid_triangles(&mut mesh, &vertices, segments, rings);
    mesh
}

pub fn cylinder(center: [f32; 3], radius: f32, depth: f32, color: Color, segments: u32) -> Mesh {
    assert!(segments >= 3, "cylinder needs at least 3 segments");
    let mut mesh = Mesh::new();
    let half_depth = depth * 0.5;
    for segment in 0..segments {
        let next = (segment + 1) % segments;
        let (angle, next_angle) = (TAU * segment as f32 / segments as f32, TAU * next as f32 / segments as f32);
        let (x, z) = (angle.cos(), angle.sin());
        let (next_x, next_z) = (next_angle.cos(), next_angle.sin());
        let normal = [x, 0.0, z];
        let next_normal = [next_x, 0.0, next_z];
        let bottom = [center[0] + radius * x, center[1] - half_depth, center[2] + radius * z];
        let top = [center[0] + radius * x, center[1] + half_depth, center[2] + radius * z];
        let next_bottom = [center[0] + radius * next_x, center[1] - half_depth, center[2] + radius * next_z];
        let next_top = [center[0] + radius * next_x, center[1] + half_depth, center[2] + radius * next_z];
        mesh.add_quad([vertex(bottom, normal, color), vertex(top, normal, color), vertex(next_top, next_normal, color), vertex(next_bottom, next_normal, color)]);
        let top_center = mesh.add_vertex(vertex([center[0], center[1] + half_depth, center[2]], [0.0, 1.0, 0.0], color));
        let top_cap_next = mesh.add_vertex(vertex(next_top, [0.0, 1.0, 0.0], color));
        let top_cap_current = mesh.add_vertex(vertex(top, [0.0, 1.0, 0.0], color));
        mesh.add_triangle(top_center, top_cap_next, top_cap_current);
        let bottom_center = mesh.add_vertex(vertex([center[0], center[1] - half_depth, center[2]], [0.0, -1.0, 0.0], color));
        let bottom_cap_current = mesh.add_vertex(vertex(bottom, [0.0, -1.0, 0.0], color));
        let bottom_cap_next = mesh.add_vertex(vertex(next_bottom, [0.0, -1.0, 0.0], color));
        mesh.add_triangle(bottom_center, bottom_cap_current, bottom_cap_next);
    }
    mesh
}

pub fn cone(center: [f32; 3], radius: f32, depth: f32, color: Color, segments: u32) -> Mesh {
    assert!(segments >= 3, "cone needs at least 3 segments");
    let mut mesh = Mesh::new();
    let half_depth = depth * 0.5;
    let apex = [center[0], center[1] + half_depth, center[2]];
    let slope = radius / depth;
    for segment in 0..segments {
        let next = (segment + 1) % segments;
        let (angle, next_angle) = (TAU * segment as f32 / segments as f32, TAU * next as f32 / segments as f32);
        let (x, z) = (angle.cos(), angle.sin());
        let (next_x, next_z) = (next_angle.cos(), next_angle.sin());
        let bottom = [center[0] + radius * x, center[1] - half_depth, center[2] + radius * z];
        let next_bottom = [center[0] + radius * next_x, center[1] - half_depth, center[2] + radius * next_z];
        let normal = [x, slope, z];
        let next_normal = [next_x, slope, next_z];
        mesh.add_quad([vertex(bottom, normal, color), vertex(apex, normal, color), vertex(apex, next_normal, color), vertex(next_bottom, next_normal, color)]);
        let bottom_center = mesh.add_vertex(vertex([center[0], center[1] - half_depth, center[2]], [0.0, -1.0, 0.0], color));
        let bottom_cap_next = mesh.add_vertex(vertex(next_bottom, [0.0, -1.0, 0.0], color));
        let bottom_cap_current = mesh.add_vertex(vertex(bottom, [0.0, -1.0, 0.0], color));
        mesh.add_triangle(bottom_center, bottom_cap_next, bottom_cap_current);
    }
    mesh
}

pub fn torus(center: [f32; 3], major_radius: f32, minor_radius: f32, color: Color, major_segments: u32, minor_segments: u32) -> Mesh {
    assert!(major_segments >= 3 && minor_segments >= 3, "torus needs at least 3 segments in each direction");
    let mut mesh = Mesh::new();
    let mut vertices = vec![vec![0_u32; (minor_segments + 1) as usize]; (major_segments + 1) as usize];
    for major in 0..=major_segments {
        let u = TAU * major as f32 / major_segments as f32;
        for minor in 0..=minor_segments {
            let v = TAU * minor as f32 / minor_segments as f32;
            let normal = [v.cos() * u.cos(), v.sin(), v.cos() * u.sin()];
            let distance = major_radius + minor_radius * v.cos();
            let position = [center[0] + distance * u.cos(), center[1] + minor_radius * v.sin(), center[2] + distance * u.sin()];
            vertices[major as usize][minor as usize] = mesh.add_vertex(vertex(position, normal, color));
        }
    }
    for major in 0..major_segments {
        for minor in 0..minor_segments {
            let top_left = vertices[major as usize][minor as usize];
            let top_right = vertices[(major + 1) as usize][minor as usize];
            let bottom_left = vertices[major as usize][(minor + 1) as usize];
            let bottom_right = vertices[(major + 1) as usize][(minor + 1) as usize];
            mesh.add_triangle(top_left, top_right, bottom_left);
            mesh.add_triangle(top_right, bottom_right, bottom_left);
        }
    }
    mesh
}

fn vertex(position: [f32; 3], normal: [f32; 3], color: Color) -> MeshVertex {
    MeshVertex { position, normal, color: color.to_array() }
}

fn add_grid_triangles(mesh: &mut Mesh, vertices: &[Vec<u32>], segments: u32, rings: u32) {
    for ring in 0..rings {
        for segment in 0..segments {
            let top_left = vertices[ring as usize][segment as usize];
            let top_right = vertices[ring as usize][(segment + 1) as usize];
            let bottom_left = vertices[(ring + 1) as usize][segment as usize];
            let bottom_right = vertices[(ring + 1) as usize][(segment + 1) as usize];
            mesh.add_triangle(top_left, top_right, bottom_left);
            mesh.add_triangle(top_right, bottom_right, bottom_left);
        }
    }
}

#[cfg(test)]
mod tests {
    use math::Color;

    use super::{cone, cube, cylinder, plane, quad, sphere, torus};

    #[test]
    fn common_primitives_generate_triangles() {
        let meshes = [
            plane(2.0, Color::WHITE),
            quad(2.0, 3.0, 0.5, Color::WHITE),
            cube([0.0, 0.0, 0.0], 2.0, Color::WHITE),
            sphere([0.0, 0.0, 0.0], 1.0, Color::WHITE, 16, 8),
            cylinder([0.0, 0.0, 0.0], 1.0, 2.0, Color::WHITE, 16),
            cone([0.0, 0.0, 0.0], 1.0, 2.0, Color::WHITE, 16),
            torus([0.0, 0.0, 0.0], 1.5, 0.4, Color::WHITE, 16, 8),
        ];
        assert!(meshes.iter().all(|mesh| !mesh.is_empty()));
    }
}
