use math::{Camera, DirectionalLight, Light, PointLight, SpotLight, Transform};
use mesh::{Material, Mesh, MeshInstance};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MeshId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CameraId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LightId(usize);

#[derive(Default)]
pub struct Scene {
    meshes: Vec<MeshInstance>,
    cameras: Vec<Camera>,
    lights: Vec<Light>,
}

impl Scene {
    pub fn spawn_mesh(&mut self, mesh: Mesh, transform: Transform) -> MeshId {
        let id = MeshId(self.meshes.len());
        self.meshes.push(MeshInstance::new(mesh, transform));
        id
    }

    pub fn spawn_static_mesh(&mut self, mesh: Mesh, transform: Transform) -> MeshId {
        let id = MeshId(self.meshes.len());
        let mut instance = MeshInstance::new(mesh, transform);
        instance.probe_dynamic = false;
        self.meshes.push(instance);
        id
    }

    pub fn spawn_mesh_with_material(&mut self, mesh: Mesh, transform: Transform, material: Material) -> MeshId {
        let id = MeshId(self.meshes.len());
        let mut instance = MeshInstance::new(mesh, transform);
        instance.material = material;
        self.meshes.push(instance);
        id
    }

    pub fn spawn_camera(&mut self, camera: Camera) -> CameraId {
        let id = CameraId(self.cameras.len());
        self.cameras.push(camera);
        id
    }

    pub fn spawn_light(&mut self, light: DirectionalLight) -> LightId {
        let id = LightId(self.lights.len());
        self.lights.push(Light::Directional(light));
        id
    }

    pub fn spawn_point_light(&mut self, light: PointLight) -> LightId {
        let id = LightId(self.lights.len());
        self.lights.push(Light::Point(light));
        id
    }

    pub fn spawn_spot_light(&mut self, light: SpotLight) -> LightId {
        let id = LightId(self.lights.len());
        self.lights.push(Light::Spot(light));
        id
    }

    pub fn get_mesh_mut(&mut self, mesh: MeshId) -> Option<&mut MeshInstance> {
        self.meshes.get_mut(mesh.0)
    }

    pub fn get_camera(&self, camera: CameraId) -> Option<&Camera> {
        self.cameras.get(camera.0)
    }

    pub fn get_camera_mut(&mut self, camera: CameraId) -> Option<&mut Camera> {
        self.cameras.get_mut(camera.0)
    }

    pub fn get_light(&self, light: LightId) -> Option<&Light> {
        self.lights.get(light.0)
    }

    pub fn get_directional_light_mut(&mut self, light: LightId) -> Option<&mut DirectionalLight> {
        match self.lights.get_mut(light.0) {
            Some(Light::Directional(light)) => Some(light),
            _ => None,
        }
    }

    pub fn lights(&self) -> &[Light] {
        &self.lights
    }

    pub fn instances(&self) -> &[MeshInstance] {
        &self.meshes
    }
}
