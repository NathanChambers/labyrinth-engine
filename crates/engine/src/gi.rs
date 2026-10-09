use math::{Color, Vec3};

use crate::gizmo::GizmoCanvas;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IrradianceVolumeId(usize);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IrradianceMode {
    #[default]
    Volumes,
    Clipmap,
    Combined,
    AabbSdf,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrradianceClipmapLevel {
    pub extent: Vec3,
    pub resolution: [u32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrradianceClipmapDesc {
    pub levels: [IrradianceClipmapLevel; 3],
    pub level_count: u32,
}

impl Default for IrradianceClipmapDesc {
    fn default() -> Self {
        Self {
            levels: [
                IrradianceClipmapLevel { extent: Vec3::new(40.0, 20.0, 40.0), resolution: [13, 14, 13] },
                IrradianceClipmapLevel { extent: Vec3::new(64.0, 48.0, 64.0), resolution: [8, 5, 8] },
                IrradianceClipmapLevel { extent: Vec3::new(128.0, 96.0, 128.0), resolution: [6, 4, 6] },
            ],
            level_count: 3,
        }
    }
}

impl IrradianceClipmapDesc {
    pub fn volumes_at(self, camera_position: Vec3) -> [IrradianceVolumeDesc; 3] {
        self.levels.map(|level| level.volume_at(camera_position))
    }

    pub fn active_level_count(self) -> usize {
        self.level_count.clamp(1, self.levels.len() as u32) as usize
    }
}

impl IrradianceClipmapLevel {
    fn volume_at(self, camera_position: Vec3) -> IrradianceVolumeDesc {
        let resolution = self.resolution.map(|value| value.max(1));
        let extent = Vec3::new(self.extent.x.max(0.001), self.extent.y.max(0.001), self.extent.z.max(0.001));
        let spacing =
            Vec3::new(extent.x / resolution[0].saturating_sub(1).max(1) as f32, extent.y / resolution[1].saturating_sub(1).max(1) as f32, extent.z / resolution[2].saturating_sub(1).max(1) as f32);
        let center = Vec3::new((camera_position.x / spacing.x).round() * spacing.x, (camera_position.y / spacing.y).round() * spacing.y, (camera_position.z / spacing.z).round() * spacing.z);
        let half_extent = extent * 0.5;
        IrradianceVolumeDesc { minimum: center - half_extent, maximum: center + half_extent, resolution, enabled: true }
    }
}

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
        self.draw_debug_gizmos_for_mode(gizmos, IrradianceMode::Volumes, &[]);
    }

    pub fn draw_debug_gizmos_for_mode(&self, gizmos: &mut GizmoCanvas, mode: IrradianceMode, clipmaps: &[IrradianceVolumeDesc]) {
        if matches!(mode, IrradianceMode::Volumes | IrradianceMode::Combined) {
            let authored_limit = if mode == IrradianceMode::Combined { 4usize.saturating_sub(clipmaps.len()) } else { 4 };
            for (_, volume) in self.iter().filter(|(_, volume)| volume.desc.enabled).take(authored_limit) {
                draw_volume_debug_gizmos(gizmos, volume.desc, Color::rgba(0.2, 0.8, 1.0, 0.9));
            }
        }
        if matches!(mode, IrradianceMode::Clipmap | IrradianceMode::Combined) {
            for (level, clipmap) in clipmaps.iter().enumerate() {
                let color = match level {
                    0 => Color::rgba(0.95, 0.55, 0.15, 0.9),
                    1 => Color::rgba(0.9, 0.3, 0.75, 0.75),
                    _ => Color::rgba(0.25, 0.65, 0.95, 0.65),
                };
                draw_volume_debug_gizmos(gizmos, *clipmap, color);
            }
        }
    }
}

fn draw_volume_debug_gizmos(gizmos: &mut GizmoCanvas, desc: IrradianceVolumeDesc, color: Color) {
    gizmos.wire_box(desc.minimum, desc.maximum, color);
    let center = (desc.minimum + desc.maximum) * 0.5;
    let extent = desc.maximum - desc.minimum;
    gizmos.ring(center, Vec3::new(extent.x, 0.0, extent.z).length() * 0.15, color);
    let resolution = desc.resolution.map(|value| value.max(1));
    for z in 0..resolution[2] {
        for y in 0..resolution[1] {
            for x in 0..resolution[0] {
                gizmos.dot(probe_position(desc, [x, y, z]), 0.025, Color::rgb(1.0, 0.9, 0.1));
            }
        }
    }
}

fn probe_position(desc: IrradianceVolumeDesc, coordinate: [u32; 3]) -> Vec3 {
    let denominator = [desc.resolution[0].saturating_sub(1).max(1), desc.resolution[1].saturating_sub(1).max(1), desc.resolution[2].saturating_sub(1).max(1)];
    let fraction = Vec3::new(coordinate[0] as f32 / denominator[0] as f32, coordinate[1] as f32 / denominator[1] as f32, coordinate[2] as f32 / denominator[2] as f32);
    desc.minimum + (desc.maximum - desc.minimum) * fraction
}
