use bytemuck::{Pod, Zeroable};
use math::Transform;

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

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
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

        Ok(Self { vertices, indices })
    }

    pub fn add_vertex(&mut self, vertex: MeshVertex) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(vertex);
        index
    }

    pub fn add_triangle(&mut self, first: u32, second: u32, third: u32) {
        assert!((first as usize) < self.vertices.len(), "first mesh index is outside the vertex data");
        assert!((second as usize) < self.vertices.len(), "second mesh index is outside the vertex data");
        assert!((third as usize) < self.vertices.len(), "third mesh index is outside the vertex data");
        self.indices.extend_from_slice(&[first, second, third]);
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
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct MeshInstance {
    pub mesh: Mesh,
    pub transform: Transform,
}

impl MeshInstance {
    pub fn new(mesh: Mesh, transform: Transform) -> Self {
        Self { mesh, transform }
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
