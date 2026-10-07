use math::{Color, Vec3};

use crate::gizmo::GizmoCanvas;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IrradianceVolumeId(usize);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrradianceVolumeDesc {
    pub minimum: Vec3,
    pub maximum: Vec3,
    pub resolution: [u32; 3],
    pub enabled: bool,
}

impl Default for IrradianceVolumeDesc {
    fn default() -> Self {
        Self { minimum: Vec3::splat(-4.0), maximum: Vec3::splat(4.0), resolution: [8, 4, 8], enabled: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrradianceVolume {
    pub desc: IrradianceVolumeDesc,
}

#[derive(Default)]
pub struct IrradianceVolumes {
    volumes: Vec<IrradianceVolume>,
}

impl IrradianceVolumes {
    pub fn add(&mut self, desc: IrradianceVolumeDesc) -> IrradianceVolumeId {
        let id = IrradianceVolumeId(self.volumes.len());
        self.volumes.push(IrradianceVolume { desc });
        id
    }

    pub fn get(&self, id: IrradianceVolumeId) -> Option<&IrradianceVolume> {
        self.volumes.get(id.0)
    }

    pub fn get_mut(&mut self, id: IrradianceVolumeId) -> Option<&mut IrradianceVolume> {
        self.volumes.get_mut(id.0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (IrradianceVolumeId, &IrradianceVolume)> {
        self.volumes.iter().enumerate().map(|(index, volume)| (IrradianceVolumeId(index), volume))
    }

    pub fn draw_debug_gizmos(&self, gizmos: &mut GizmoCanvas) {
        for (_, volume) in self.iter() {
            if !volume.desc.enabled {
                continue;
            }
            let color = Color::rgba(0.2, 0.8, 1.0, 0.9);
            gizmos.wire_box(volume.desc.minimum, volume.desc.maximum, color);
            let center = (volume.desc.minimum + volume.desc.maximum) * 0.5;
            let extent = volume.desc.maximum - volume.desc.minimum;
            gizmos.ring(center, Vec3::new(extent.x, 0.0, extent.z).length() * 0.15, color);
            let resolution = volume.desc.resolution.map(|value| value.max(1));
            for z in 0..resolution[2] {
                for y in 0..resolution[1] {
                    for x in 0..resolution[0] {
                        let position = probe_position(volume.desc, [x, y, z]);
                        gizmos.dot(position, 0.025, Color::rgb(1.0, 0.9, 0.1));
                    }
                }
            }
        }
    }
}

fn probe_position(desc: IrradianceVolumeDesc, coordinate: [u32; 3]) -> Vec3 {
    let denominator = [desc.resolution[0].saturating_sub(1).max(1), desc.resolution[1].saturating_sub(1).max(1), desc.resolution[2].saturating_sub(1).max(1)];
    let fraction = Vec3::new(coordinate[0] as f32 / denominator[0] as f32, coordinate[1] as f32 / denominator[1] as f32, coordinate[2] as f32 / denominator[2] as f32);
    desc.minimum + (desc.maximum - desc.minimum) * fraction
}
