use math::{Color, Light, Vec3};
use mesh::MeshInstance;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoVertex {
    pub position: Vec3,
    pub color: Color,
}

#[derive(Default)]
pub struct GizmoCanvas {
    vertices: Vec<GizmoVertex>,
}

impl GizmoCanvas {
    pub fn clear(&mut self) {
        self.vertices.clear();
    }

    pub fn line(&mut self, start: Vec3, end: Vec3, color: Color) {
        self.vertices.push(GizmoVertex { position: start, color });
        self.vertices.push(GizmoVertex { position: end, color });
    }

    pub fn wire_box(&mut self, minimum: Vec3, maximum: Vec3, color: Color) {
        let corners = [
            Vec3::new(minimum.x, minimum.y, minimum.z),
            Vec3::new(maximum.x, minimum.y, minimum.z),
            Vec3::new(maximum.x, maximum.y, minimum.z),
            Vec3::new(minimum.x, maximum.y, minimum.z),
            Vec3::new(minimum.x, minimum.y, maximum.z),
            Vec3::new(maximum.x, minimum.y, maximum.z),
            Vec3::new(maximum.x, maximum.y, maximum.z),
            Vec3::new(minimum.x, maximum.y, maximum.z),
        ];
        for (start, end) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
            self.line(corners[start], corners[end], color);
        }
    }

    pub fn ring(&mut self, center: Vec3, radius: f32, color: Color) {
        let segments = 24;
        for index in 0..segments {
            let first = std::f32::consts::TAU * index as f32 / segments as f32;
            let second = std::f32::consts::TAU * (index + 1) as f32 / segments as f32;
            self.line(center + Vec3::new(first.cos() * radius, 0.0, first.sin() * radius), center + Vec3::new(second.cos() * radius, 0.0, second.sin() * radius), color);
        }
    }

    pub fn dot(&mut self, position: Vec3, radius: f32, color: Color) {
        self.line(position - Vec3::X * radius, position + Vec3::X * radius, color);
        self.line(position - Vec3::Y * radius, position + Vec3::Y * radius, color);
        self.line(position - Vec3::Z * radius, position + Vec3::Z * radius, color);
    }

    pub fn lights(&mut self, lights: &[Light]) {
        for light in lights {
            match light {
                Light::Directional(light) => {
                    let direction = light.direction.normalize_or_zero();
                    let color = light.color;
                    self.line(Vec3::ZERO, direction * 2.0, color);
                    self.dot(direction * 2.0, 0.12, color);
                }
                Light::Point(light) => {
                    self.dot(light.position, 0.16, light.color);
                    self.ring_axis(light.position, Vec3::Y, light.range, light.color);
                    self.ring_axis(light.position, Vec3::X, light.range, light.color);
                    self.ring_axis(light.position, Vec3::Z, light.range, light.color);
                }
                Light::Spot(light) => {
                    let direction = light.direction.normalize_or_zero();
                    let color = light.color;
                    let base = light.position + direction * light.range;
                    let (right, up) = cone_basis(direction);
                    let radius = light.range * light.outer_angle.sin();
                    self.dot(light.position, 0.16, color);
                    self.line(light.position, base, color);
                    for index in 0..16 {
                        let first = std::f32::consts::TAU * index as f32 / 16.0;
                        let second = std::f32::consts::TAU * (index + 1) as f32 / 16.0;
                        let first_point = base + (right * first.cos() + up * first.sin()) * radius;
                        let second_point = base + (right * second.cos() + up * second.sin()) * radius;
                        self.line(first_point, second_point, color);
                        if index % 4 == 0 {
                            self.line(light.position, first_point, color);
                        }
                    }
                }
            }
        }
    }

    pub fn mesh_normals(&mut self, instances: &[MeshInstance], length: f32) {
        let color = Color::rgb(1.0, 0.35, 0.1);
        for instance in instances {
            for triangle in instance.mesh.indices().as_chunks::<3>().0.iter().step_by(4) {
                let Some(vertex_a) = instance.mesh.vertices().get(triangle[0] as usize) else { continue };
                let Some(vertex_b) = instance.mesh.vertices().get(triangle[1] as usize) else { continue };
                let Some(vertex_c) = instance.mesh.vertices().get(triangle[2] as usize) else { continue };
                let position_a = instance.transform.transform_point(Vec3::from_array(vertex_a.position));
                let position_b = instance.transform.transform_point(Vec3::from_array(vertex_b.position));
                let position_c = instance.transform.transform_point(Vec3::from_array(vertex_c.position));
                let midpoint = (position_a + position_b + position_c) / 3.0;
                let normal = (position_b - position_a).cross(position_c - position_a).normalize_or_zero();
                self.line(midpoint, midpoint + normal * length, color);
            }
        }
    }

    fn ring_axis(&mut self, center: Vec3, axis: Vec3, radius: f32, color: Color) {
        let (right, up) = cone_basis(axis.normalize_or_zero());
        for index in 0..24 {
            let first = std::f32::consts::TAU * index as f32 / 24.0;
            let second = std::f32::consts::TAU * (index + 1) as f32 / 24.0;
            self.line(center + (right * first.cos() + up * first.sin()) * radius, center + (right * second.cos() + up * second.sin()) * radius, color);
        }
    }

    pub fn vertices(&self) -> &[GizmoVertex] {
        &self.vertices
    }
}

fn cone_basis(axis: Vec3) -> (Vec3, Vec3) {
    let reference = if axis.dot(Vec3::Y).abs() < 0.95 { Vec3::Y } else { Vec3::X };
    let right = axis.cross(reference).normalize_or_zero();
    let up = right.cross(axis).normalize_or_zero();
    (right, up)
}
