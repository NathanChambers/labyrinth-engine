use bytemuck::{Pod, Zeroable};
use math::Color;
use math::Transform;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MESH_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub base_color: Color,
    pub metallic: f32,
    pub roughness: f32,
    pub emission: Color,
    pub emission_strength: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self { base_color: Color::WHITE, metallic: 0.0, roughness: 0.5, emission: Color::BLACK, emission_strength: 0.0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct UiVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Clone, Debug)]
struct MeshGeometry {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
    id: u64,
    revision: u64,
}

#[derive(Clone, Debug)]
pub struct Mesh {
    geometry: Arc<MeshGeometry>,
}

impl Default for Mesh {
    fn default() -> Self {
        Self { geometry: new_geometry(Vec::new(), Vec::new()) }
    }
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_data(vertices: Vec<MeshVertex>, indices: Vec<u32>) -> Result<Self, String> {
        if indices.iter().any(|index| *index as usize >= vertices.len()) {
            return Err("mesh index is outside the vertex data".into());
        }

        if !indices.len().is_multiple_of(3) {
            return Err("mesh indices must contain complete triangles".into());
        }

        Ok(Self { geometry: new_geometry(vertices, indices) })
    }

    pub fn add_vertex(&mut self, vertex: MeshVertex) -> u32 {
        let geometry = self.geometry_mut();
        let index = geometry.vertices.len() as u32;
        geometry.vertices.push(vertex);
        geometry.revision = geometry.revision.wrapping_add(1);
        index
    }

    pub fn add_triangle(&mut self, first: u32, second: u32, third: u32) {
        assert!((first as usize) < self.geometry.vertices.len(), "first mesh index is outside the vertex data");
        assert!((second as usize) < self.geometry.vertices.len(), "second mesh index is outside the vertex data");
        assert!((third as usize) < self.geometry.vertices.len(), "third mesh index is outside the vertex data");
        let geometry = self.geometry_mut();
        geometry.indices.extend_from_slice(&[first, second, third]);
        geometry.revision = geometry.revision.wrapping_add(1);
    }

    pub fn add_quad(&mut self, vertices: [MeshVertex; 4]) {
        let first = self.add_vertex(vertices[0]);
        let second = self.add_vertex(vertices[1]);
        let third = self.add_vertex(vertices[2]);
        let fourth = self.add_vertex(vertices[3]);
        self.add_triangle(first, second, third);
        self.add_triangle(first, third, fourth);
    }

    pub fn vertices(&self) -> &[MeshVertex] {
        &self.geometry.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.geometry.indices
    }

    pub fn geometry_version(&self) -> (u64, u64) {
        (self.geometry.id, self.geometry.revision)
    }

    pub fn is_empty(&self) -> bool {
        self.geometry.indices.is_empty()
    }

    fn geometry_mut(&mut self) -> &mut MeshGeometry {
        if Arc::strong_count(&self.geometry) > 1 {
            let mut geometry = (*self.geometry).clone();
            geometry.id = NEXT_MESH_ID.fetch_add(1, Ordering::Relaxed);
            self.geometry = Arc::new(geometry);
        }
        Arc::make_mut(&mut self.geometry)
    }
}

fn new_geometry(vertices: Vec<MeshVertex>, indices: Vec<u32>) -> Arc<MeshGeometry> {
    Arc::new(MeshGeometry { vertices, indices, id: NEXT_MESH_ID.fetch_add(1, Ordering::Relaxed), revision: 0 })
}

#[derive(Clone, Debug)]
pub struct MeshInstance {
    pub mesh: Mesh,
    pub transform: Transform,
    pub material: Material,
    pub probe_dynamic: bool,
}

impl MeshInstance {
    pub fn new(mesh: Mesh, transform: Transform) -> Self {
        Self { mesh, transform, material: Material::default(), probe_dynamic: true }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mesh, MeshVertex};

    fn vertex() -> MeshVertex {
        MeshVertex { position: [0.0, 0.0, 0.0], normal: [0.0, 1.0, 0.0], color: [1.0, 1.0, 1.0, 1.0] }
    }

    #[test]
    fn rejects_invalid_indices() {
        let result = Mesh::from_data(vec![vertex()], vec![0, 1, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_incomplete_triangles() {
        let result = Mesh::from_data(vec![vertex(); 3], vec![0, 1]);
        assert!(result.is_err());
    }
}
