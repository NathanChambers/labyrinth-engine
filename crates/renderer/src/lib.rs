use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::mem;
use std::ops::Range;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use math::{Camera, Color, DirectionalLight, Light, Mat3, Mat4, SpotLight, Vec3, Vec4};
use mesh::{MeshInstance, MeshVertex, UiVertex};
use wgpu::util::DeviceExt;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_DIRECTIONAL_LIGHTS: usize = 8;
const MAX_SPOT_SHADOWS: usize = 8;
const MAX_POINT_SHADOWS: usize = 2;
const POINT_SHADOW_FACE_COUNT: u32 = 6;
const SHADOW_MAP_SIZE: u32 = 4096;
const SPOT_SHADOW_MAP_SIZE: u32 = 2048;
const SHADOW_CASCADE_COUNT: usize = 3;
const MAX_IRRADIANCE_VOLUMES: usize = 4;
const MAX_IRRADIANCE_PROBES: usize = 4096;
const PROBE_SURFACE_SAMPLE_TARGET_AREA: f32 = 0.1;
const PROBE_SURFACE_SAMPLE_MIN_COUNT: usize = 24;
const PROBE_SURFACE_SAMPLE_MAX_COUNT: usize = 256;
const PROBE_SURFACE_SAMPLE_SCALE: f32 = 0.5;
const VISIBILITY_REBUILD_INTERVAL: u32 = 4;
const GPU_TIMESTAMP_COUNT: u32 = 14;
const SDF_TARGET_CELL_SIZE: f32 = 0.375;
const SDF_CLIPMAP_PADDING: Vec3 = Vec3::new(8.0, 4.0, 8.0);
const SDF_MAX_DISTANCE: f32 = 16.0;
const SDF_CLEARANCE_RADIUS_VOXELS: u16 = 4;
const RADIANCE_FIELD_TARGET_CELL_SIZE: f32 = 1.5;
const RADIANCE_FIELD_PADDING: Vec3 = Vec3::new(8.0, 4.0, 8.0);
const RADIANCE_FIELD_MAX_CELL_COUNT: u32 = 16_384;
pub const RADIANCE_FIELD_RAY_COUNT: usize = 8;
const RADIANCE_FIELD_MAX_RAYS_PER_CELL: usize = 64;
pub const RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS: usize = 64;
pub const RADIANCE_SURFACE_CANDIDATE_COUNT: usize = 8;
const RADIANCE_SURFACE_INSPECTION_VEC4_COUNT: usize = 4 + RADIANCE_SURFACE_CANDIDATE_COUNT * 9;
const RADIANCE_SURFACE_INSPECTION_SIZE: u64 = (RADIANCE_SURFACE_INSPECTION_VEC4_COUNT * mem::size_of::<[f32; 4]>()) as u64;
const RADIANCE_FIELD_DIRTY_CELL_BUDGET: usize = 32;
const RADIANCE_FIELD_RAY_WORK_BUDGET: usize = 1024;
const RADIANCE_FIELD_BOOTSTRAP_RAYS_PER_CELL: usize = 32;
const RADIANCE_FIELD_RAY_COUNT_SHIFT: u32 = 24;
const RADIANCE_FIELD_CELL_INDEX_MASK: u32 = 0x00ff_ffff;
const RADIANCE_FIELD_COVERAGE_READBACK_INTERVAL: u32 = 30;
const RADIANCE_FIELD_RESET_CELL_FLAG: u32 = 1 << 31;
const RADIANCE_FIELD_TRACE_DEBUG_VEC4_COUNT: usize = 13;
const RADIANCE_FIELD_TRACE_DEBUG_RECORD_SIZE: u64 = (RADIANCE_FIELD_TRACE_DEBUG_VEC4_COUNT * mem::size_of::<[f32; 4]>()) as u64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct RendererGizmoVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenderDebugMode {
    #[default]
    LitMaterials,
    UnlitMaterials,
    Wireframe,
    ShadowVisibility,
    GiOnly,
    GiSupport,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadowQuality {
    Low,
    Medium,
    High,
    Ultra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowUpdatePolicy {
    EveryFrame,
    OnChange,
    Periodic,
    Cached,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowResourceKind {
    DirectionalCascade,
    SpotMap,
    PointCube,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowResourceAllocation {
    pub light_index: u32,
    pub kind: ShadowResourceKind,
    pub slot: u32,
    pub resolution: u32,
    pub atlas_layer: u32,
    pub atlas_offset: [u32; 2],
}

impl ShadowResourceAllocation {
    pub fn atlas_uv_scale_offset(&self, atlas_size: u32) -> [f32; 4] {
        let inv_size = 1.0 / atlas_size.max(1) as f32;

        [self.resolution as f32 * inv_size, self.resolution as f32 * inv_size, self.atlas_offset[0] as f32 * inv_size, self.atlas_offset[1] as f32 * inv_size]
    }

    pub fn point_face_layer(&self, face: u32) -> Option<u32> {
        (self.kind == ShadowResourceKind::PointCube && face < POINT_SHADOW_FACE_COUNT).then_some(self.atlas_layer + face)
    }
}

#[derive(Clone, Debug, Default)]
struct ShadowResourceTable {
    allocations: Vec<ShadowResourceAllocation>,
}

impl ShadowResourceTable {
    fn rebuild(&mut self, requests: &[ShadowRequest], resolution: u32, atlas_size: u32) {
        self.allocations.clear();
        for request in requests {
            let allocation = match request.resource_kind {
                ShadowResourceKind::DirectionalCascade => ShadowResourceAllocation { light_index: request.light_index, kind: request.resource_kind, slot: request.resource_slot, resolution, atlas_layer: request.resource_slot, atlas_offset: [0, 0] },
                ShadowResourceKind::SpotMap => {
                    let columns = (atlas_size / resolution.max(1)).max(1);
                    let cell = request.resource_slot % columns;
                    let row = request.resource_slot / columns;
                    if (row + 1) * resolution > atlas_size {
                        continue;
                    }
                    ShadowResourceAllocation { light_index: request.light_index, kind: request.resource_kind, slot: request.resource_slot, resolution, atlas_layer: 0, atlas_offset: [cell * resolution, row * resolution] }
                }
                ShadowResourceKind::PointCube => ShadowResourceAllocation { light_index: request.light_index, kind: request.resource_kind, slot: request.resource_slot, resolution, atlas_layer: request.resource_slot * POINT_SHADOW_FACE_COUNT, atlas_offset: [0, 0] },
            };
            if !self.allocations.contains(&allocation) {
                self.allocations.push(allocation);
            }
        }
    }

    fn allocation_for(&self, light_index: usize) -> Option<&ShadowResourceAllocation> {
        self.allocations.iter().find(|allocation| allocation.light_index as usize == light_index)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowBudget {
    pub max_dynamic_lights: u32,
    pub max_point_lights: u32,
    pub max_spot_lights: u32,
    pub max_updates_per_frame: u32,
    pub atlas_size: u32,
}

impl Default for ShadowBudget {
    fn default() -> Self {
        Self { max_dynamic_lights: 8, max_point_lights: 2, max_spot_lights: 8, max_updates_per_frame: 8, atlas_size: 4096 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowRequest {
    pub light_index: u32,
    pub resource_kind: ShadowResourceKind,
    pub resource_slot: u32,
    pub policy: ShadowUpdatePolicy,
    pub priority: u32,
    pub dirty: bool,
}

#[derive(Clone, Debug, Default)]
struct ShadowScheduler {
    requests: Vec<ShadowRequest>,
    dirty_lights: Vec<bool>,
}

impl ShadowScheduler {
    fn rebuild(&mut self, lights: &[Light], camera: &Camera, budget: ShadowBudget) {
        self.requests.clear();
        self.dirty_lights.resize(lights.len(), true);
        let mut spot_slot = 0;
        let mut point_slot = 0;
        let spot_cells_per_axis = (budget.atlas_size / SPOT_SHADOW_MAP_SIZE).max(1);
        let spot_capacity = spot_cells_per_axis.saturating_mul(spot_cells_per_axis);

        for (index, light) in lights.iter().enumerate() {
            let (resource_kind, resource_slot, policy, priority) = match light {
                Light::Directional(_) => (ShadowResourceKind::DirectionalCascade, 0, ShadowUpdatePolicy::EveryFrame, 100_000),
                Light::Spot(light) if budget.max_spot_lights > 0 && spot_slot < spot_capacity => {
                    let distance = light.position.distance(camera.transform.position);
                    let slot = spot_slot;
                    spot_slot += 1;
                    (ShadowResourceKind::SpotMap, slot, ShadowUpdatePolicy::OnChange, (20_000.0 / (1.0 + distance)).round() as u32)
                }
                Light::Point(light) if budget.max_point_lights > 0 => {
                    let distance = light.position.distance(camera.transform.position);
                    let slot = point_slot;
                    point_slot += 1;
                    (ShadowResourceKind::PointCube, slot, ShadowUpdatePolicy::OnChange, (16_000.0 / (1.0 + distance)).round() as u32)
                }
                _ => (ShadowResourceKind::DirectionalCascade, 0, ShadowUpdatePolicy::Disabled, 0),
            };
            if policy != ShadowUpdatePolicy::Disabled {
                self.requests
                    .push(ShadowRequest { light_index: index as u32, resource_kind, resource_slot, policy, priority, dirty: self.dirty_lights[index] });
            }
        }
        self.requests.sort_by_key(|request| std::cmp::Reverse(request.priority));

        self.requests.truncate(budget.max_dynamic_lights.min(budget.max_updates_per_frame) as usize);
    }

    #[cfg(test)]
    fn is_scheduled(&self, light_index: usize) -> bool {
        self.requests.iter().any(|request| request.light_index as usize == light_index && request.dirty)
    }

    fn counts(&self) -> (u32, u32) {
        (self.requests.len() as u32, self.requests.iter().filter(|request| request.dirty).count() as u32)
    }

    fn mark_all_dirty(&mut self) {
        self.dirty_lights.fill(true);
        for request in &mut self.requests {
            request.dirty = true;
        }
    }

    fn mark_clean(&mut self, light_index: usize) {
        if let Some(dirty) = self.dirty_lights.get_mut(light_index) {
            *dirty = false;
        }

        for request in &mut self.requests {
            if request.light_index as usize == light_index {
                request.dirty = false;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    pub shadows_enabled: bool,
    pub contact_shadows_enabled: bool,
    pub ambient_occlusion_enabled: bool,
    pub ambient_intensity: f32,
    pub irradiance_enabled: bool,
    pub triangle_probe_occlusion_enabled: bool,
    pub shadow_cascade_count: u32,
    pub shadow_resolution: u32,
    pub shadow_depth_bias: f32,
    pub shadow_normal_bias: f32,
    pub shadow_filter_radius: f32,
    pub secondary_light_shadowing: bool,
    pub direct_light_visibility_enabled: bool,
    pub shadow_budget: ShadowBudget,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            shadows_enabled: true,
            contact_shadows_enabled: false,
            ambient_occlusion_enabled: false,
            ambient_intensity: 0.02,
            irradiance_enabled: true,
            triangle_probe_occlusion_enabled: true,
            shadow_cascade_count: SHADOW_CASCADE_COUNT as u32,
            shadow_resolution: SHADOW_MAP_SIZE,
            shadow_depth_bias: 0.00005,
            shadow_normal_bias: 0.0,
            shadow_filter_radius: 1.5,
            secondary_light_shadowing: false,
            direct_light_visibility_enabled: true,
            shadow_budget: ShadowBudget::default(),
        }
    }
}

impl RenderSettings {
    pub fn with_shadow_quality(mut self, quality: ShadowQuality) -> Self {
        let (cascade_count, resolution, depth_bias, normal_bias, filter_radius) = match quality {
            ShadowQuality::Low => (1, 1024, 0.00015, 0.0, 0.35),
            ShadowQuality::Medium => (2, 2048, 0.00008, 0.0, 1.5),
            ShadowQuality::High => (3, 4096, 0.00004, 0.0, 1.5),
            ShadowQuality::Ultra => (3, 4096, 0.00002, 0.0, 1.0),
        };
        self.shadow_cascade_count = cascade_count;
        self.shadow_resolution = resolution;
        self.shadow_depth_bias = depth_bias;
        self.shadow_normal_bias = normal_bias;
        self.shadow_filter_radius = filter_radius;

        self
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RendererMeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    tex_coord: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RendererInstanceData {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 3],
    material_base_color: [f32; 4],
    material_params: [f32; 4],
    material_emission: [f32; 4],
    surface_id: u32,
    padding: [u32; 3],
}

#[derive(Clone, Debug)]
struct MeshDrawBatch {
    index_range: Range<u32>,
    wireframe_range: Range<u32>,
    instance_range: Range<usize>,
    instance_indices: Vec<usize>,
    material_texture_ids: MaterialTextureIds,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MaterialTextureIds {
    base_color: Option<u64>,
    normal: Option<u64>,
    metallic: Option<u64>,
    roughness: Option<u64>,
}

struct MaterialTextureBinding {
    _textures: Vec<wgpu::Texture>,
    bind_group: wgpu::BindGroup,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GraphicsApi {
    #[default]
    Vulkan,
    DirectX12,
    OpenGl,
    Metal,
}

impl GraphicsApi {
    fn backends(self) -> wgpu::Backends {
        match self {
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::DirectX12 => wgpu::Backends::DX12,
            Self::OpenGl => wgpu::Backends::GL,
            Self::Metal => wgpu::Backends::METAL,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    inverse_view_projection: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    shadow_view_projection: [[[f32; 4]; 4]; SHADOW_CASCADE_COUNT],
    shadow_cascade_splits: [f32; 4],
    spot_shadow_view_projections: [[[f32; 4]; 4]; MAX_SPOT_SHADOWS],
    spot_shadow_rects: [[f32; 4]; MAX_SPOT_SHADOWS],
    spot_shadow_atlas_size: [f32; 4],
    point_shadow_view_projections: [[[f32; 4]; 4]; MAX_POINT_SHADOWS * POINT_SHADOW_FACE_COUNT as usize],
    light_directions: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_colors: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_positions: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_params: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_kinds: [[u32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_shadow_modes: [[u32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_count: [u32; 4],
    camera_position: [f32; 4],
    debug_mode: [u32; 4],
    shadow_settings: [f32; 4],
    viewport_size: [f32; 4],
    shadow_flags: [u32; 4],
    ambient_light: [f32; 4],
    gi_settings: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IrradianceVolumeUniform {
    minimum: [f32; 4],
    maximum: [f32; 4],
    resolution: [u32; 4],
    probe_offset: u32,
    _padding: [u32; 7],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct IrradianceUniform {
    volumes: [IrradianceVolumeUniform; MAX_IRRADIANCE_VOLUMES],
    probes: [[f32; 4]; MAX_IRRADIANCE_PROBES],
    directions: [[f32; 4]; MAX_IRRADIANCE_PROBES],
    probe_positions: [[f32; 4]; MAX_IRRADIANCE_PROBES],
    volume_count: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SdfClipmapUniform {
    minimum: [f32; 4],
    maximum: [f32; 4],
    resolution: [u32; 4],
    settings: [u32; 4],
    sky_radiance: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SdfGpuUpdateSettings {
    minimum: [f32; 4],
    voxel_size: [f32; 4],
    counts: [u32; 4],
    update: [u32; 4],
    region_counts: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SdfGpuUpdateRegion {
    origin: [u32; 4],
    size: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RadianceFieldUniform {
    minimum: [f32; 4],
    maximum: [f32; 4],
    resolution: [u32; 4],
    settings: [u32; 4],
    scheduled_cells: [[u32; 4]; 8],
    schedule_metadata: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RadianceFieldAngularSample {
    radiance: [f32; 4],
    sampling: [u32; 4],
}

#[derive(Clone, Copy, PartialEq)]
struct SdfGeometry {
    bounds: GeometryBounds,
    albedo: Vec3,
    emission: Vec3,
    surface_signature: u64,
}

#[derive(Clone, Copy)]
struct SurfaceInspectionTriangle {
    vertices: [Vec3; 3],
    triangle_index: u32,
    albedo: Vec3,
    emission: Vec3,
}

#[derive(Clone)]
struct SurfaceInspectionMesh {
    instance_index: u32,
    bounds: GeometryBounds,
    triangles: Vec<SurfaceInspectionTriangle>,
}

struct SurfaceInspectionTriangleHit {
    instance_index: u32,
    triangle_index: u32,
    distance: f32,
    position: Vec3,
    normal: Vec3,
    barycentric: Vec3,
    vertices: [Vec3; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TriangleBvhNode {
    minimum: [f32; 4],
    maximum: [f32; 4],
    metadata: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TriangleBvhTriangle {
    vertices: [[f32; 4]; 3],
    source: [u32; 4],
    albedo: [f32; 4],
    emission: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TriangleBvhSettings {
    counts: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TriangleBvhCounterBuffer {
    counters: [u32; 4],
}

struct TriangleBvhBuildItem {
    triangle: TriangleBvhTriangle,
    bounds: GeometryBounds,
    centroid: Vec3,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum TriangleBvhBuildStrategy {
    Median,
    #[default]
    Sah,
}

struct SdfClipmap {
    resolution: [u32; 3],
    minimum: Vec3,
    maximum: Vec3,
    voxel_size: Vec3,
    values: Vec<[f32; 4]>,
    emission_values: Vec<[f32; 4]>,
    geometry_bounds: Vec<SdfGeometry>,
    geometry_bvh: SdfGeometryBvh,
}

struct SdfGeometryBvh {
    geometries: Vec<SdfGeometry>,
    geometry_indices: Vec<usize>,
    nodes: Vec<SdfBvhNode>,
}

#[derive(Clone, Copy)]
struct SdfBvhNode {
    bounds: GeometryBounds,
    left: usize,
    right: usize,
    start: usize,
    count: usize,
}

struct WorldRadianceField {
    resolution: [u32; 3],
    minimum: Vec3,
    maximum: Vec3,
    cell_size: Vec3,
    frame_index: u32,
    needs_full_refresh: bool,
    queued_reset_cells: VecDeque<u32>,
    queued_reset_cell_mask: Vec<bool>,
    queued_priority_cells: VecDeque<u32>,
    queued_priority_cell_mask: Vec<bool>,
    queued_camera_cells: VecDeque<u32>,
    queued_camera_cell_mask: Vec<bool>,
    queued_dirty_cells: VecDeque<u32>,
    queued_dirty_cell_mask: Vec<bool>,
    refinement_cursor: u32,
    camera_focus_cell: Option<[i32; 3]>,
}

#[derive(Clone, Copy)]
struct RadianceFieldPriority {
    position: Vec3,
    radius: f32,
}

#[derive(Clone, Copy)]
struct SdfTextureRegion {
    origin: [u32; 3],
    size: [u32; 3],
}

struct SdfClipmapUpdate {
    texture_regions: Vec<SdfTextureRegion>,
    clearance_regions: Vec<SdfTextureRegion>,
    voxels_updated: u32,
    geometry_bvh_build_ms: f32,
    voxel_build_ms: f32,
    clearance_build_ms: f32,
}

#[derive(Default)]
struct SdfClipmapBuildTiming {
    geometry_bvh_build_ms: f32,
    voxel_build_ms: f32,
    clearance_build_ms: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererIrradianceVolume {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    pub resolution: [u32; 3],
    pub enabled: bool,
}

/// Target world-space spacing between radiance-field probes, in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadianceFieldCellSize {
    pub meters: f32,
}

impl Default for RadianceFieldCellSize {
    fn default() -> Self {
        Self { meters: RADIANCE_FIELD_TARGET_CELL_SIZE }
    }
}

impl RadianceFieldCellSize {
    fn bounded(self) -> Self {
        Self { meters: if self.meters.is_finite() { self.meters.clamp(0.25, 8.0) } else { RADIANCE_FIELD_TARGET_CELL_SIZE } }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererRadianceFieldGrid {
    /// Minimum world-space corner of the field.
    pub minimum: [f32; 3],
    /// Maximum world-space corner of the field.
    pub maximum: [f32; 3],
    /// Cell count along X, Y, and Z.
    pub resolution: [u32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererSdfGrid {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    pub resolution: [u32; 3],
    pub voxel_size: [f32; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererViewportPick {
    pub world_position: [f32; 3],
    pub world_normal: [f32; 3],
    pub distance: f32,
    pub instance_index: u32,
    pub triangle_index: u32,
    pub barycentric: [f32; 3],
    pub sdf_voxel: Option<[u32; 3]>,
    pub radiance_cell: Option<[u32; 3]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererSdfVoxelDebug {
    pub coordinate: [u32; 3],
    pub world_position: [f32; 3],
    pub signed_distance: f32,
    pub albedo: [f32; 3],
    pub emission: [f32; 3],
    pub clearance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererSkyLighting {
    pub color: Color,
    pub intensity: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererPerformanceStats {
    pub frame_cpu_ms: f32,
    pub light_update_ms: f32,
    pub instance_update_ms: f32,
    pub sdf_build_cpu_ms: f32,
    pub sdf_geometry_bvh_build_ms: f32,
    pub sdf_voxel_build_ms: f32,
    pub sdf_clearance_build_ms: f32,
    pub sdf_upload_ms: f32,
    pub sdf_voxels_updated: u32,
    pub gi_region_coverage_percent: f32,
    pub gi_regions_covered_twice_percent: f32,
    pub gi_cells_with_coverage_percent: f32,
    pub gi_valid_cells_percent: f32,
    pub gi_regions_attempted_percent: f32,
    pub gi_coverage_available: bool,
    pub gi_pending_cells: u32,
    pub gi_cells_updated: u32,
    pub gi_rays_scheduled: u32,
    pub gi_angular_regions: u32,
    pub probe_update_ms: f32,
    pub irradiance_upload_ms: f32,
    pub visibility_rebuild_ms: f32,
    pub probes_updated: u32,
    pub probe_budget: u32,
    pub static_sample_count: u32,
    pub dynamic_sample_count: u32,
    pub region_count: u32,
    pub gpu_shadow_ms: f32,
    pub gpu_scene_ms: f32,
    pub gpu_sdf_gi_ms: f32,
    pub gpu_sdf_update_ms: f32,
    pub gpu_gizmo_ms: f32,
    pub gpu_ui_ms: f32,
    pub gpu_total_ms: f32,
    pub scheduled_shadow_lights: u32,
    pub dirty_shadow_lights: u32,
    pub triangle_bvh_segments: u32,
    pub triangle_bvh_bounds_tests: u32,
    pub triangle_bvh_triangle_tests: u32,
    pub triangle_bvh_hits: u32,
    pub triangle_bvh_build_ms: f32,
    pub triangle_bvh_refit_ms: f32,
    pub triangle_bvh_node_count: u32,
    pub triangle_bvh_triangle_count: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum RendererGiTraceDebugMode {
    #[default]
    Geometry,
    Radiance,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u32)]
pub enum RadianceAngularResolution {
    Low = 8,
    #[default]
    Balanced = 32,
    High = 64,
}

impl RadianceAngularResolution {
    pub fn segment_count(self) -> u32 {
        self as u32
    }

    fn bands_and_sectors(self) -> (u32, u32) {
        match self {
            Self::Low => (2, 4),
            Self::Balanced => (4, 8),
            Self::High => (8, 8),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererRadianceAngularSegment {
    pub sampled: bool,
    pub direction: [f32; 3],
    pub radiance: [f32; 3],
    pub sample_count: u32,
    pub attempt_count: u32,
    pub age_frames: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RendererRadianceProbeDebug {
    pub coordinate: [u32; 3],
    pub cell_center: [f32; 3],
    pub probe_position: [f32; 3],
    pub relocation_valid: bool,
    pub relocation_succeeded: bool,
    pub covered_once: u32,
    pub covered_twice: u32,
    pub attempted_regions: u32,
    pub angular_segments: Vec<RendererRadianceAngularSegment>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererGiRayDiagnostic {
    pub valid: bool,
    pub relocation_data_valid: bool,
    pub relocation_succeeded: bool,
    pub selected_cell: [u32; 3],
    pub termination_reason: u32,
    pub travelled_distance: f32,
    pub hit_position: [f32; 3],
    pub hit_normal: [f32; 3],
    pub original_cell_center: [f32; 3],
    pub ray_origin: [f32; 3],
    pub relocation_offset: [f32; 3],
    pub radiance: [f32; 3],
    pub direct_radiance: [f32; 3],
    pub emission_radiance: [f32; 3],
    pub sky_radiance: [f32; 3],
    pub stored_radiance: [f32; 3],
    pub radiance_difference: [f32; 3],
    pub last_update_age_frames: u32,
    pub ray_direction: [f32; 3],
    pub accumulated_sample_count: u32,
    pub attempted_sample_count: u32,
    pub blend_factor: f32,
    pub angular_segment_index: u32,
    pub segment_sample_count: u32,
    pub segment_attempt_count: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererRadianceSurfaceCandidate {
    pub grid_position: [u32; 3],
    pub probe_position: [f32; 3],
    pub probe_valid: bool,
    pub spatial_weight: f32,
    pub side_weight: f32,
    pub directional_radiance: [f32; 3],
    pub directional_support: f32,
    pub contribution_radiance: [f32; 3],
    pub final_weight: f32,
    pub first_blocking_voxel: [i32; 3],
    pub has_first_blocking_voxel: bool,
    pub first_occupied_after_start: [i32; 3],
    pub has_occupied_voxel_after_start: bool,
    pub first_occupied_after_start_minimum: [f32; 3],
    pub first_occupied_after_start_maximum: [f32; 3],
    pub triangle_occlusion_tested: bool,
    pub triangle_occluded: bool,
    pub triangle_instance_index: u32,
    pub triangle_index: u32,
    pub triangle_hit_distance: f32,
    pub triangle_hit_position: [f32; 3],
    pub triangle_hit_normal: [f32; 3],
    pub triangle_hit_barycentric: [f32; 3],
    pub triangle_hit_vertices: [[f32; 3]; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererRadianceSurfaceInspection {
    pub valid: bool,
    pub inside_field: bool,
    pub world_position: [f32; 3],
    pub normal: [f32; 3],
    pub sdf_start_voxel: [i32; 3],
    pub sdf_start_in_bounds: bool,
    pub sdf_start_occupied: bool,
    pub sdf_origin_offset: [f32; 3],
    pub candidates: [RendererRadianceSurfaceCandidate; RADIANCE_SURFACE_CANDIDATE_COUNT],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RadianceSurfaceInspectionSettings {
    pixel: [u32; 4],
}

struct GpuTimestampState {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    timestamp_period: f32,
}

#[derive(Clone, Copy)]
struct ProbeSurfaceSample {
    position: Vec3,
    normal: Vec3,
    color: Vec3,
    emission: Vec3,
    emission_strength: f32,
    weight: f32,
}

#[derive(Clone, Copy)]
struct ProbeSampleTriangle {
    vertices: [MeshVertex; 3],
    positions: [Vec3; 3],
    normals: [Vec3; 3],
    cumulative_area: f32,
}

impl Default for RendererSkyLighting {
    fn default() -> Self {
        Self { color: Color::rgb(0.12, 0.18, 0.3), intensity: 0.35 }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowUniform {
    view_projection: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpotShadowUniform {
    view_projection: [[f32; 4]; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
    pub surface_normals: Vec<[u8; 4]>,
    pub scene_depth: CapturedDepthBuffer,
    pub directional_shadow: CapturedDepthBuffer,
    pub spot_shadow: CapturedDepthBuffer,
    pub point_shadow: CapturedDepthBuffer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedDepthBuffer {
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Vec<f32>>,
}

pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    radiance_field_pipeline: wgpu::ComputePipeline,
    radiance_field_bounce_pipeline: wgpu::ComputePipeline,
    radiance_field_bounce_bind_group: wgpu::BindGroup,
    radiance_surface_inspection_pipeline: wgpu::ComputePipeline,
    depth_prepass_pipeline: wgpu::RenderPipeline,
    wireframe_pipeline: wgpu::RenderPipeline,
    gi_gizmo_pipeline: wgpu::RenderPipeline,
    angular_segment_pipeline: wgpu::RenderPipeline,
    sdf_occupancy_gizmo_pipeline: wgpu::RenderPipeline,
    gi_gizmo_bind_group: wgpu::BindGroup,
    gi_gizmo_selection_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    mesh_topology_signature: u64,
    instance_signatures: Vec<u64>,
    instance_signature_scratch: Vec<u64>,
    instance_data_scratch: Vec<RendererInstanceData>,
    instance_data_offsets: Vec<usize>,
    instance_data_dirty_slots: Vec<usize>,
    mesh_draw_batches: Vec<MeshDrawBatch>,
    wireframe_index_buffer: wgpu::Buffer,
    wireframe_index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    material_texture_bind_group_layout: wgpu::BindGroupLayout,
    material_texture_bindings: HashMap<MaterialTextureIds, MaterialTextureBinding>,
    material_texture_sampler: wgpu::Sampler,
    irradiance_buffer: wgpu::Buffer,
    irradiance_cache: Box<IrradianceUniform>,
    irradiance_direct_cache: Box<IrradianceUniform>,
    irradiance_volume_layout: Vec<RendererIrradianceVolume>,
    sdf_clipmap: SdfClipmap,
    sdf_geometry_cache: Vec<SdfGeometry>,
    sdf_geometry_cache_dirty: bool,
    surface_inspection_meshes: Vec<SurfaceInspectionMesh>,
    surface_inspection_meshes_dirty: bool,
    triangle_bvh_dirty: bool,
    triangle_bvh_topology_dirty: bool,
    sdf_uniform_buffer: wgpu::Buffer,
    _sdf_texture: wgpu::Texture,
    _sdf_emission_texture: wgpu::Texture,
    sdf_view: wgpu::TextureView,
    sdf_emission_view: wgpu::TextureView,
    _sdf_clearance_texture: wgpu::Texture,
    _sdf_clearance_work_texture: wgpu::Texture,
    sdf_clearance_work_view: wgpu::TextureView,
    _sdf_clearance_sdf_input_texture: wgpu::Texture,
    sdf_clearance_sdf_input_view: wgpu::TextureView,
    sdf_update_pipeline: wgpu::ComputePipeline,
    sdf_clearance_pipeline: wgpu::ComputePipeline,
    sdf_update_bind_group_layout: wgpu::BindGroupLayout,
    sdf_update_settings_buffer: wgpu::Buffer,
    sdf_update_dirty_buffer: Option<wgpu::Buffer>,
    sdf_update_dirty_capacity: u64,
    radiance_field: WorldRadianceField,
    radiance_field_uniform_buffer: wgpu::Buffer,
    _radiance_field_buffer: wgpu::Buffer,
    _radiance_field_bounce_buffer: wgpu::Buffer,
    _radiance_field_probe_buffer: wgpu::Buffer,
    _radiance_field_trace_debug_buffer: wgpu::Buffer,
    _radiance_field_coverage_buffer: wgpu::Buffer,
    radiance_field_coverage_readback_buffer: wgpu::Buffer,
    radiance_field_coverage_readback_receiver: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    gi_trace_readback_buffer: wgpu::Buffer,
    gi_trace_readback_receiver: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    gi_trace_readback_cell: [u32; 3],
    gi_trace_readback_frame: u32,
    gi_trace_readback_angular_resolution: RadianceAngularResolution,
    gi_trace_diagnostics: [RendererGiRayDiagnostic; RADIANCE_FIELD_RAY_COUNT],
    radiance_surface_inspection_settings_buffer: wgpu::Buffer,
    radiance_surface_inspection_values_buffer: wgpu::Buffer,
    radiance_surface_inspection_readback_buffer: wgpu::Buffer,
    radiance_surface_inspection_readback_receiver: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    radiance_surface_inspection_request: Option<[u32; 2]>,
    radiance_surface_inspection_mesh_snapshot: Option<Vec<SurfaceInspectionMesh>>,
    radiance_surface_inspection: RendererRadianceSurfaceInspection,
    sdf_gi_enabled: bool,
    gi_radiance_frozen: bool,
    gi_radiance_temporal_accumulation_enabled: bool,
    gi_radiance_probe_relocation_enabled: bool,
    irradiance_probe_ages: [u32; MAX_IRRADIANCE_PROBES],
    irradiance_update_budget: usize,
    sky_lighting: RendererSkyLighting,
    visibility_grid: VisibilityGrid,
    visibility_bounds: Vec<GeometryBounds>,
    visibility_rebuild_cooldown: u32,
    static_probe_samples: Vec<ProbeSurfaceSample>,
    dynamic_probe_samples: Vec<ProbeSurfaceSample>,
    static_probe_signature: u64,
    static_probe_samples_dirty: bool,
    dynamic_probe_samples_dirty: bool,
    shadow_bounds: GeometryBounds,
    performance_stats: RendererPerformanceStats,
    frame_index: u32,
    shadow_scheduler: ShadowScheduler,
    shadow_resources: ShadowResourceTable,
    shadow_pipeline: wgpu::RenderPipeline,
    _shadow_texture: wgpu::Texture,
    shadow_views: [wgpu::TextureView; SHADOW_CASCADE_COUNT],
    _shadow_sampler: wgpu::Sampler,
    shadow_uniform_buffers: [wgpu::Buffer; SHADOW_CASCADE_COUNT],
    shadow_bind_groups: [wgpu::BindGroup; SHADOW_CASCADE_COUNT],
    shadow_matrices: [Mat4; SHADOW_CASCADE_COUNT],
    shadow_cascade_splits: [f32; 4],
    local_mesh_bounds: Vec<GeometryBounds>,
    shadow_caster_bounds: Vec<GeometryBounds>,
    spot_shadow_pipeline: wgpu::RenderPipeline,
    _spot_shadow_texture: wgpu::Texture,
    spot_shadow_view: wgpu::TextureView,
    _point_shadow_texture: wgpu::Texture,
    point_shadow_views: Vec<wgpu::TextureView>,
    _spot_shadow_sampler: wgpu::Sampler,
    spot_shadow_uniform_buffer: wgpu::Buffer,
    spot_shadow_bind_group: wgpu::BindGroup,
    point_shadow_uniform_buffers: Vec<wgpu::Buffer>,
    point_shadow_bind_groups: Vec<wgpu::BindGroup>,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    _depth_sample_texture: wgpu::Texture,
    depth_sample_view: wgpu::TextureView,
    _surface_normal_texture: wgpu::Texture,
    surface_normal_view: wgpu::TextureView,
    _surface_id_texture: wgpu::Texture,
    surface_id_view: wgpu::TextureView,
    depth_bind_group_layout: wgpu::BindGroupLayout,
    depth_bind_group: wgpu::BindGroup,
    triangle_bvh_node_buffer: wgpu::Buffer,
    triangle_bvh_triangle_buffer: wgpu::Buffer,
    triangle_bvh_nodes: Vec<TriangleBvhNode>,
    triangle_bvh_triangles: Vec<TriangleBvhTriangle>,
    triangle_bvh_settings_buffer: wgpu::Buffer,
    triangle_bvh_settings: TriangleBvhSettings,
    triangle_bvh_diagnostics_buffer: wgpu::Buffer,
    triangle_bvh_diagnostics_readback_buffer: wgpu::Buffer,
    triangle_bvh_diagnostics_enabled: bool,
    triangle_bvh_build_strategy: TriangleBvhBuildStrategy,
    _depth_sampler: wgpu::Sampler,
    has_presented: bool,
    camera: Camera,
    lights: Vec<Light>,
    ui_pipeline: wgpu::RenderPipeline,
    ui_vertex_buffer: wgpu::Buffer,
    ui_vertex_capacity: usize,
    ui_vertex_count: u32,
    gizmo_pipeline: wgpu::RenderPipeline,
    gizmo_vertex_buffer: wgpu::Buffer,
    gizmo_vertex_capacity: usize,
    gizmo_vertex_count: u32,
    angular_segment_vertex_buffer: wgpu::Buffer,
    angular_segment_vertex_capacity: usize,
    angular_segment_vertex_count: u32,
    radiance_angular_resolution: RadianceAngularResolution,
    radiance_angular_segments: [RendererRadianceAngularSegment; RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS],
    radiance_angular_segments_cell: [u32; 3],
    world_radiance_gizmos_enabled: bool,
    world_radiance_trace_gizmos_enabled: bool,
    selected_radiance_cell: [u32; 3],
    sdf_occupancy_gizmos_enabled: bool,
    render_debug_mode: RenderDebugMode,
    render_settings: RenderSettings,
    gpu_timestamps: Option<GpuTimestampState>,
    sdf_gpu_updated_this_frame: bool,
}

impl Renderer {
    fn update_shadow_scheduler_stats(&mut self) {
        let (scheduled_shadow_lights, dirty_shadow_lights) = self.shadow_scheduler.counts();
        self.performance_stats.scheduled_shadow_lights = scheduled_shadow_lights;

        self.performance_stats.dirty_shadow_lights = dirty_shadow_lights;
    }

    fn rebuild_shadow_resources(&mut self) {
        self.shadow_resources
            .rebuild(&self.shadow_scheduler.requests, self.render_settings.shadow_resolution, self.render_settings.shadow_budget.atlas_size);
    }

    pub fn new(window: Arc<Window>, graphics_api: GraphicsApi, camera: &Camera, lights: &[Light], instances: &[MeshInstance], irradiance_volumes: &[RendererIrradianceVolume]) -> Result<Self, String> {
        Self::new_with_radiance_field_cell_size(window, graphics_api, camera, lights, instances, irradiance_volumes, RadianceFieldCellSize::default())
    }

    pub fn new_with_radiance_field_cell_size(window: Arc<Window>, graphics_api: GraphicsApi, camera: &Camera, lights: &[Light], instances: &[MeshInstance], irradiance_volumes: &[RendererIrradianceVolume], radiance_field_cell_size: RadianceFieldCellSize) -> Result<Self, String> {
        let radiance_field_cell_size = radiance_field_cell_size.bounded();
        let (vertices, indices, mesh_draw_batches) = flatten_meshes(instances)?;
        let (instance_data, instance_data_offsets) = build_instance_data(instances, &mesh_draw_batches);
        let local_mesh_bounds = mesh_local_bounds(instances);
        let shadow_caster_bounds = transformed_mesh_bounds(instances, &local_mesh_bounds);

        let backend = graphics_api.backends();

        if !wgpu::Instance::enabled_backend_features().contains(backend) {
            return Err(format!("{graphics_api:?} is unavailable on this platform"));
        }
        let descriptor = wgpu::InstanceDescriptor { backends: backend, ..wgpu::InstanceDescriptor::new_without_display_handle() };
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance.create_surface(window.clone()).map_err(|error| error.to_string())?;
        let options = wgpu::RequestAdapterOptions { compatible_surface: Some(&surface), ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&options)).map_err(|error| format!("{graphics_api:?} adapter unavailable: {error}"))?;
        let timestamp_features = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        let required_features = if adapter.features().contains(timestamp_features) { timestamp_features } else { wgpu::Features::empty() };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_features, ..Default::default() })).map_err(|error| error.to_string())?;
        let gpu_timestamps = if required_features.contains(timestamp_features) {
            let query_set = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("GPU pass timestamps"), ty: wgpu::QueryType::Timestamp, count: GPU_TIMESTAMP_COUNT });
            let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("GPU timestamp resolve"), size: u64::from(GPU_TIMESTAMP_COUNT) * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("GPU timestamp readback"), size: u64::from(GPU_TIMESTAMP_COUNT) * 8, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            Some(GpuTimestampState { query_set, resolve_buffer, readback_buffer, timestamp_period: queue.get_timestamp_period() })
        } else {
            None
        };

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let config = surface.get_default_config(&adapter, width, height).ok_or("selected graphics API cannot present to this window")?;
        let mut config = config;
        let surface_capabilities = surface.get_capabilities(&adapter);
        config.present_mode = if surface_capabilities.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        config.desired_maximum_frame_latency = 3;
        log::info!(present_mode:? = config.present_mode, gpu_timestamps = gpu_timestamps.is_some(); "Renderer configured");
        surface.configure(&device, &config);
        let shadow_bounds = combined_geometry_bounds(&geometry_bounds(instances));

        let (shadow_texture, shadow_array_view, shadow_views) = create_directional_shadow_texture(&device);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_matrices = lights
            .iter()
            .find_map(|light| match light {
                Light::Directional(light) => Some(cascade_shadow_view_projections(light, camera, width, height, RenderSettings::default().shadow_cascade_count)),
                _ => None,
            })
            .unwrap_or([Mat4::IDENTITY; SHADOW_CASCADE_COUNT]);
        let shadow_cascade_splits = cascade_splits(camera);
        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("shadow layout"), entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }] });
        let shadow_uniform_buffers =
            std::array::from_fn(|cascade_index| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("shadow cascade uniform"), contents: bytemuck::bytes_of(&ShadowUniform { view_projection: shadow_matrices[cascade_index].to_cols_array_2d() }), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }));
        let shadow_bind_groups = std::array::from_fn(|cascade_index| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("shadow cascade bind group"), layout: &shadow_bind_group_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_uniform_buffers[cascade_index].as_entire_binding() }] }));
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shadow shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shadow_shader.wgsl").into()) });
        let spot_shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("spot shadow shader"), source: wgpu::ShaderSource::Wgsl(include_str!("spot_shadow_shader.wgsl").into()) });
        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow pipeline layout"), bind_group_layouts: &[Some(&shadow_bind_group_layout)], immediate_size: 0 });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vertex_main"),
                buffers: &[
                    Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3] }),
                    Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererInstanceData>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4] }),
                ],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Front), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 0, slope_scale: 1.0, clamp: 0.0 } }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let (spot_shadow_texture, spot_shadow_view) = create_shadow_texture(&device, RenderSettings::default().shadow_budget.atlas_size);
        let (point_shadow_texture, point_shadow_array_view, point_shadow_views) = create_point_shadow_texture(&device);
        let spot_shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spot shadow sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let spot_shadow_matrix = lights
            .iter()
            .find_map(|light| match light {
                Light::Spot(light) => Some(spot_shadow_view_projection(light)),
                _ => None,
            })
            .unwrap_or(Mat4::IDENTITY);
        let spot_shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("spot shadow uniform"), contents: bytemuck::bytes_of(&SpotShadowUniform { view_projection: spot_shadow_matrix.to_cols_array_2d() }), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let spot_shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("spot shadow layout"), entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }] });
        let spot_shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("spot shadow bind group"), layout: &spot_shadow_bind_group_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: spot_shadow_uniform_buffer.as_entire_binding() }] });
        let point_shadow_uniform_buffers = (0..MAX_POINT_SHADOWS * POINT_SHADOW_FACE_COUNT as usize)
            .map(|face| device.create_buffer(&wgpu::BufferDescriptor { label: Some(&format!("point shadow uniform {face}")), size: mem::size_of::<SpotShadowUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }))
            .collect::<Vec<_>>();
        let point_shadow_bind_groups = point_shadow_uniform_buffers
            .iter()
            .map(|buffer| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("point shadow bind group"), layout: &spot_shadow_bind_group_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }] }))
            .collect::<Vec<_>>();
        let spot_shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("spot shadow pipeline layout"), bind_group_layouts: &[Some(&spot_shadow_bind_group_layout)], immediate_size: 0 });
        let spot_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spot shadow pipeline"),
            layout: Some(&spot_shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &spot_shadow_shader,
                entry_point: Some("vertex_main"),
                buffers: &[
                    Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3] }),
                    Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererInstanceData>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Instance, attributes: &wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4] }),
                ],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 0, slope_scale: 0.25, clamp: 0.0 } }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let camera_uniform = camera_uniform(width, height, camera, lights, RenderDebugMode::default(), RenderSettings::default(), shadow_bounds);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("camera uniform"), contents: bytemuck::bytes_of(&camera_uniform), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let sky_lighting = RendererSkyLighting::default();
        let visibility_bounds = geometry_bounds(instances);
        let visibility_grid = VisibilityGrid::from_bounds(&visibility_bounds);
        let sdf_geometries = sdf_geometries(instances);
        let surface_inspection_meshes = surface_inspection_meshes(instances);
        let triangle_bvh_build_strategy = TriangleBvhBuildStrategy::default();
        let triangle_bvh_build_start = Instant::now();
        let (triangle_bvh_nodes, triangle_bvh_triangles) = build_triangle_bvh(&surface_inspection_meshes, triangle_bvh_build_strategy);
        let triangle_bvh_build_ms = triangle_bvh_build_start.elapsed().as_secs_f32() * 1000.0;
        let triangle_bvh_settings = TriangleBvhSettings { counts: [triangle_bvh_nodes.len() as u32, triangle_bvh_triangles.len() as u32, 0, 0] };
        let triangle_bvh_node_buffer = storage_buffer_or_dummy(&device, "triangle visibility BVH nodes", &triangle_bvh_nodes);
        let triangle_bvh_triangle_buffer = storage_buffer_or_dummy(&device, "triangle visibility BVH triangles", &triangle_bvh_triangles);
        let triangle_bvh_settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("triangle visibility BVH settings"), contents: bytemuck::bytes_of(&triangle_bvh_settings), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let triangle_bvh_diagnostics_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("triangle visibility BVH diagnostics"), contents: bytemuck::bytes_of(&TriangleBvhCounterBuffer::zeroed()), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST });
        let triangle_bvh_diagnostics_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("triangle visibility BVH diagnostics readback"), size: mem::size_of::<TriangleBvhCounterBuffer>() as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let scene_bounds = combined_geometry_bounds(&geometry_bounds(instances));
        let mut sdf_clipmap = SdfClipmap::new(scene_bounds, &[]);
        let sdf_resolution = sdf_clipmap.resolution;
        let sdf_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("triangle surface SDF and albedo clipmap"),
            size: wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let sdf_view = sdf_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let sdf_emission_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("triangle surface SDF emission clipmap"),
            size: wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let sdf_emission_view = sdf_emission_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let sdf_clearance_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDF conservative clearance clipmap"),
            size: wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sdf_clearance_work_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDF conservative clearance update target"),
            size: wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sdf_clearance_view = sdf_clearance_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let sdf_clearance_work_view = sdf_clearance_work_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let sdf_clearance_sdf_input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDF clearance source snapshot"),
            size: wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sdf_clearance_sdf_input_view = sdf_clearance_sdf_input_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let initial_clearance = sdf_clipmap.emission_values.iter().map(|value| value[3]).collect::<Vec<_>>();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &sdf_clearance_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&initial_clearance),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sdf_resolution[0] * 4), rows_per_image: Some(sdf_resolution[1]) },
            wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
        );
        let radiance_field = WorldRadianceField::new(scene_bounds, radiance_field_cell_size.meters);
        let radiance_field_grid_resolution = radiance_field.resolution;
        let radiance_field_uniform = RadianceFieldUniform::zeroed();
        let radiance_field_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world radiance field uniform"), contents: bytemuck::bytes_of(&radiance_field_uniform), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let radiance_field_values = vec![RadianceFieldAngularSample::zeroed(); radiance_field_grid_resolution.iter().product::<u32>() as usize * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS];
        let radiance_field_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world radiance field angular regions"), contents: bytemuck::cast_slice(&radiance_field_values), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST });
        let radiance_field_bounce_values = vec![[0.0_f32; 4]; radiance_field_values.len()];
        let radiance_field_bounce_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world radiance field second bounce radiance"), contents: bytemuck::cast_slice(&radiance_field_bounce_values), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        let radiance_field_probe_values = vec![[0.0_f32; 4]; radiance_field_grid_resolution.iter().product::<u32>() as usize];
        let radiance_field_probe_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world radiance field probe relocation state"), contents: bytemuck::cast_slice(&radiance_field_probe_values), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST });
        let radiance_field_coverage_values = vec![[0_u32; 4]; radiance_field_grid_resolution.iter().product::<u32>() as usize];
        let radiance_field_coverage_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world radiance field coverage summaries"), contents: bytemuck::cast_slice(&radiance_field_coverage_values), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST });
        let radiance_field_coverage_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("world radiance field coverage readback"), size: (radiance_field_coverage_values.len() * mem::size_of::<[u32; 4]>()) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let radiance_field_trace_debug_count = radiance_field_grid_resolution.iter().product::<u32>() as usize;
        let radiance_field_trace_debug_values = vec![[[0.0_f32; 4]; RADIANCE_FIELD_TRACE_DEBUG_VEC4_COUNT]; radiance_field_trace_debug_count * RADIANCE_FIELD_RAY_COUNT];
        let radiance_field_trace_debug_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("world radiance field trace diagnostics"), contents: bytemuck::cast_slice(&radiance_field_trace_debug_values), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC });
        let gi_trace_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("selected GI ray radiance readback"),
            size: RADIANCE_FIELD_TRACE_DEBUG_RECORD_SIZE * RADIANCE_FIELD_RAY_COUNT as u64 + mem::size_of::<RadianceFieldAngularSample>() as u64 * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let radiance_surface_inspection_settings_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("GI surface inspection pixel"), contents: bytemuck::bytes_of(&RadianceSurfaceInspectionSettings { pixel: [0; 4] }), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let radiance_surface_inspection_values = vec![[0.0_f32; 4]; RADIANCE_SURFACE_INSPECTION_VEC4_COUNT];
        let radiance_surface_inspection_values_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("GI surface candidate contributions"), contents: bytemuck::cast_slice(&radiance_surface_inspection_values), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC });
        let radiance_surface_inspection_readback_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("GI surface inspection readback"), size: RADIANCE_SURFACE_INSPECTION_SIZE, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let gi_gizmo_selection_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("selected GI radiance cell"), contents: bytemuck::cast_slice(&[4_u32, 1, 4, 0, 0, 0, 0, 0]), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &sdf_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&sdf_clipmap.values),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sdf_resolution[0] * 16), rows_per_image: Some(sdf_resolution[1]) },
            wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &sdf_emission_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&sdf_clipmap.emission_values),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sdf_resolution[0] * 16), rows_per_image: Some(sdf_resolution[1]) },
            wgpu::Extent3d { width: sdf_resolution[0], height: sdf_resolution[1], depth_or_array_layers: sdf_resolution[2] },
        );
        sdf_clipmap.release_cpu_fields();
        let sdf_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("SDF clipmap uniform"), contents: bytemuck::bytes_of(&sdf_clipmap.uniform(false, sky_lighting)), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let (static_probe_samples, dynamic_probe_samples) = build_probe_samples(instances, true);
        let static_probe_signature = static_probe_signature(instances);
        let initial_direct_irradiance = Box::new(irradiance_uniform(irradiance_volumes, &static_probe_samples, &dynamic_probe_samples, lights, sky_lighting, &visibility_grid, true));
        let mut initial_irradiance = initial_direct_irradiance.clone();
        apply_second_diffuse_bounce(&mut initial_irradiance, &initial_direct_irradiance, &static_probe_samples, &dynamic_probe_samples, &visibility_grid);
        let irradiance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("irradiance probes"), contents: bytemuck::bytes_of(initial_irradiance.as_ref()), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST });
        let (depth_texture, depth_view) = create_depth_texture(&device, width, height);
        let (depth_sample_texture, depth_sample_view) = create_depth_sample_texture(&device, width, height);
        let (surface_normal_texture, surface_normal_view) = create_surface_normal_texture(&device, width, height);
        let (surface_id_texture, surface_id_view) = create_surface_id_texture(&device, width, height);
        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("scene depth sampler"), mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, ..Default::default() });
        let depth_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene depth layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let depth_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene depth bind group"),
            layout: &depth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth_sample_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&depth_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&surface_normal_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&surface_id_view) },
                wgpu::BindGroupEntry { binding: 4, resource: triangle_bvh_node_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: triangle_bvh_triangle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: triangle_bvh_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: triangle_bvh_diagnostics_buffer.as_entire_binding() },
            ],
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry { binding: 8, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 9, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 10, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 11, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 20, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 22, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 23, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 24, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 25, visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 26, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 27, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_array_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: irradiance_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&spot_shadow_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&spot_shadow_sampler) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&point_shadow_array_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&spot_shadow_sampler) },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&sdf_view) },
                wgpu::BindGroupEntry { binding: 9, resource: sdf_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&sdf_emission_view) },
                wgpu::BindGroupEntry { binding: 11, resource: radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 20, resource: radiance_field_trace_debug_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 22, resource: radiance_field_probe_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 23, resource: radiance_surface_inspection_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 24, resource: radiance_surface_inspection_values_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 25, resource: wgpu::BindingResource::TextureView(&sdf_clearance_view) },
                wgpu::BindGroupEntry { binding: 26, resource: radiance_field_coverage_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 27, resource: radiance_field_bounce_buffer.as_entire_binding() },
            ],
        });
        let gi_gizmo_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GI gizmo read-only layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 8, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 9, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 11, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 20, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 21, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 27, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let gi_gizmo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GI gizmo read-only resources"),
            layout: &gi_gizmo_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&sdf_view) },
                wgpu::BindGroupEntry { binding: 9, resource: sdf_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 20, resource: radiance_field_trace_debug_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 21, resource: gi_gizmo_selection_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 27, resource: radiance_field_bounce_buffer.as_entire_binding() },
            ],
        });

        let sdf_update_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("incremental SDF update layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba32Float, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba32Float, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::R32Float, view_dimension: wgpu::TextureViewDimension::D3 }, count: None },
            ],
        });
        let sdf_update_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("incremental SDF update shader"), source: wgpu::ShaderSource::Wgsl(include_str!("sdf_update.wgsl").into()) });
        let sdf_update_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("incremental SDF update pipeline layout"), bind_group_layouts: &[Some(&sdf_update_bind_group_layout)], immediate_size: 0 });
        let sdf_update_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("incremental SDF voxel update"), layout: Some(&sdf_update_layout), module: &sdf_update_shader, entry_point: Some("update_voxels"), compilation_options: Default::default(), cache: None });
        let sdf_update_settings_buffer = device.create_buffer(&wgpu::BufferDescriptor { label: Some("incremental SDF update settings"), size: mem::size_of::<SdfGpuUpdateSettings>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sdf_clearance_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("incremental SDF clearance update"), layout: Some(&sdf_update_layout), module: &sdf_update_shader, entry_point: Some("update_clearance"), compilation_options: Default::default(), cache: None });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("diffuse shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        let material_texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PBR material texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let material_texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PBR material texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let material_texture_bindings = create_material_texture_bindings(&device, &queue, &material_texture_bind_group_layout, &material_texture_sampler, instances);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout), Some(&depth_bind_group_layout), Some(&material_texture_bind_group_layout)], immediate_size: 0 });
        let gizmo_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gizmo pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout)], immediate_size: 0 });
        let vertex_layout = wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 14 => Float32x2] };
        let depth_vertex_layout = wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3] };
        let instance_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<RendererInstanceData>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4,
                7 => Float32x4, 8 => Float32x4, 9 => Float32x4,
                10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Uint32
            ],
        };
        let depth_instance_vertex_attributes = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 5 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 6 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 7 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 80, shader_location: 8 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 96, shader_location: 9 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint32, offset: 160, shader_location: 13 },
        ];
        let depth_instance_vertex_layout = wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererInstanceData>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Instance, attributes: &depth_instance_vertex_attributes };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout.clone()), Some(instance_vertex_layout.clone())], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });
        let radiance_field_bounce_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world radiance field second bounce layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 8, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 9, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 10, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 11, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 22, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 23, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 24, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 25, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 26, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 27, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let radiance_field_bounce_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world radiance field second bounce resources"),
            layout: &radiance_field_bounce_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&sdf_view) },
                wgpu::BindGroupEntry { binding: 9, resource: sdf_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&sdf_emission_view) },
                wgpu::BindGroupEntry { binding: 11, resource: radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 22, resource: radiance_field_probe_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 23, resource: radiance_surface_inspection_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 24, resource: radiance_surface_inspection_values_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 25, resource: wgpu::BindingResource::TextureView(&sdf_clearance_view) },
                wgpu::BindGroupEntry { binding: 26, resource: radiance_field_coverage_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 27, resource: radiance_field_bounce_buffer.as_entire_binding() },
            ],
        });
        let radiance_field_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("world radiance field compute pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout), Some(&depth_bind_group_layout)], immediate_size: 0 });
        let radiance_field_bounce_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("world radiance field second bounce pipeline layout"), bind_group_layouts: &[Some(&radiance_field_bounce_bind_group_layout), Some(&depth_bind_group_layout)], immediate_size: 0 });
        let radiance_field_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("world radiance field update pipeline"), layout: Some(&radiance_field_pipeline_layout), module: &shader, entry_point: Some("update_world_radiance_field"), compilation_options: Default::default(), cache: None });
        let radiance_field_bounce_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("world radiance field second bounce pipeline"), layout: Some(&radiance_field_bounce_pipeline_layout), module: &shader, entry_point: Some("update_world_radiance_field_bounces"), compilation_options: Default::default(), cache: None });
        let radiance_surface_inspection_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("GI surface contribution inspection pipeline"), layout: Some(&radiance_field_bounce_pipeline_layout), module: &shader, entry_point: Some("inspect_radiance_surface"), compilation_options: Default::default(), cache: None });
        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wireframe pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout.clone()), Some(instance_vertex_layout.clone())], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });
        let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("depth prepass shader"), source: wgpu::ShaderSource::Wgsl(include_str!("depth_prepass.wgsl").into()) });
        let depth_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("depth prepass pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout)], immediate_size: 0 });
        let depth_prepass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth prepass pipeline"),
            layout: Some(&depth_pipeline_layout),
            vertex: wgpu::VertexState { module: &depth_shader, entry_point: Some("vertex_main"), buffers: &[Some(depth_vertex_layout), Some(depth_instance_vertex_layout)], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &depth_shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: None, write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("UI shader"), source: wgpu::ShaderSource::Wgsl(include_str!("ui_shader.wgsl").into()) });
        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI pipeline"),
            layout: None,
            vertex: wgpu::VertexState { module: &ui_shader, entry_point: Some("vertex_main"), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<UiVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4] })], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &ui_shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });

        let gizmo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gizmo shader"), source: wgpu::ShaderSource::Wgsl(include_str!("gizmo_shader.wgsl").into()) });
        let gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gizmo pipeline"),
            layout: Some(&gizmo_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &gizmo_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererGizmoVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4] })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &gizmo_shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });
        let gi_gizmo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("GI cache gizmo shader"), source: wgpu::ShaderSource::Wgsl(include_str!("gi_gizmo.wgsl").into()) });
        let gi_gizmo_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("GI cache gizmo pipeline layout"), bind_group_layouts: &[Some(&gi_gizmo_bind_group_layout)], immediate_size: 0 });
        let gi_gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GI cache gizmo pipeline"),
            layout: Some(&gi_gizmo_pipeline_layout),
            vertex: wgpu::VertexState { module: &gi_gizmo_shader, entry_point: Some("vertex_main"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &gi_gizmo_shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });
        let angular_segment_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radiance angular segment heatmap pipeline"),
            layout: Some(&gi_gizmo_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &gizmo_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: mem::size_of::<RendererGizmoVertex>() as wgpu::BufferAddress, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4] })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &gizmo_shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });
        let sdf_occupancy_gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF occupancy gizmo pipeline"),
            layout: Some(&gi_gizmo_pipeline_layout),
            vertex: wgpu::VertexState { module: &gi_gizmo_shader, entry_point: Some("occupancy_vertex_main"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH_FORMAT, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &gi_gizmo_shader, entry_point: Some("fragment_main"), targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
            multiview_mask: None,
            cache: None,
        });

        let dummy_vertex = RendererMeshVertex::zeroed();
        let vertex_contents = if vertices.is_empty() { bytemuck::bytes_of(&dummy_vertex) } else { bytemuck::cast_slice(&vertices) };
        let dummy_index = 0_u32;
        let index_contents = if indices.is_empty() { bytemuck::bytes_of(&dummy_index) } else { bytemuck::cast_slice(&indices) };
        let wireframe_indices = wireframe_indices(&indices);
        let dummy_wireframe_index = 0_u32;
        let wireframe_contents = if wireframe_indices.is_empty() { bytemuck::bytes_of(&dummy_wireframe_index) } else { bytemuck::cast_slice(&wireframe_indices) };
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh vertices"), contents: vertex_contents, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });
        let dummy_instance = RendererInstanceData::zeroed();
        let instance_contents = if instance_data.is_empty() { bytemuck::bytes_of(&dummy_instance) } else { bytemuck::cast_slice(&instance_data) };
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh instance data"), contents: instance_contents, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh indices"), contents: index_contents, usage: wgpu::BufferUsages::INDEX });
        let wireframe_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("wireframe indices"), contents: wireframe_contents, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST });
        let ui_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("UI vertices"), contents: bytemuck::cast_slice(&[UiVertex { position: [0.0, 0.0], color: [0.0; 4] }; 6]), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });
        let gizmo_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gizmo vertices"), contents: bytemuck::cast_slice(&[RendererGizmoVertex { position: [0.0; 3], color: [0.0; 4] }; 2]), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });
        let angular_segment_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("radiance angular segment vertices"), contents: bytemuck::cast_slice(&[RendererGizmoVertex { position: [0.0; 3], color: [0.0; 4] }; 3]), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });

        Ok(Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            pipeline,
            radiance_field_pipeline,
            radiance_field_bounce_pipeline,
            radiance_surface_inspection_pipeline,
            depth_prepass_pipeline,
            wireframe_pipeline,
            gi_gizmo_pipeline,
            angular_segment_pipeline,
            sdf_occupancy_gizmo_pipeline,
            radiance_field_bounce_bind_group,
            vertex_buffer,
            instance_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            mesh_topology_signature: mesh_topology_signature(instances),
            instance_signatures: instances.iter().map(mesh_instance_signature).collect(),
            instance_signature_scratch: Vec::new(),
            instance_data_scratch: instance_data,
            instance_data_offsets,
            instance_data_dirty_slots: Vec::new(),
            mesh_draw_batches,
            wireframe_index_buffer,
            wireframe_index_count: wireframe_indices.len() as u32,
            camera_buffer,
            camera_bind_group,
            material_texture_bind_group_layout,
            material_texture_bindings,
            material_texture_sampler,
            irradiance_buffer,
            irradiance_cache: initial_irradiance,
            irradiance_direct_cache: initial_direct_irradiance,
            irradiance_volume_layout: irradiance_volumes.to_vec(),
            sdf_clipmap,
            sdf_geometry_cache: sdf_geometries,
            sdf_geometry_cache_dirty: false,
            surface_inspection_meshes,
            surface_inspection_meshes_dirty: false,
            triangle_bvh_dirty: false,
            triangle_bvh_topology_dirty: false,
            sdf_uniform_buffer,
            _sdf_texture: sdf_texture,
            _sdf_emission_texture: sdf_emission_texture,
            sdf_view,
            sdf_emission_view,
            _sdf_clearance_texture: sdf_clearance_texture,
            _sdf_clearance_work_texture: sdf_clearance_work_texture,
            sdf_clearance_work_view,
            _sdf_clearance_sdf_input_texture: sdf_clearance_sdf_input_texture,
            sdf_clearance_sdf_input_view,
            sdf_update_pipeline,
            sdf_clearance_pipeline,
            sdf_update_bind_group_layout,
            sdf_update_settings_buffer,
            sdf_update_dirty_buffer: None,
            sdf_update_dirty_capacity: 0,
            radiance_field,
            radiance_field_uniform_buffer,
            _radiance_field_buffer: radiance_field_buffer,
            _radiance_field_bounce_buffer: radiance_field_bounce_buffer,
            _radiance_field_probe_buffer: radiance_field_probe_buffer,
            _radiance_field_trace_debug_buffer: radiance_field_trace_debug_buffer,
            _radiance_field_coverage_buffer: radiance_field_coverage_buffer,
            radiance_field_coverage_readback_buffer,
            radiance_field_coverage_readback_receiver: None,
            gi_trace_readback_buffer,
            gi_trace_readback_receiver: None,
            gi_trace_readback_cell: [4, 1, 4],
            gi_trace_readback_frame: 0,
            gi_trace_readback_angular_resolution: RadianceAngularResolution::default(),
            gi_trace_diagnostics: [RendererGiRayDiagnostic::default(); RADIANCE_FIELD_RAY_COUNT],
            radiance_surface_inspection_settings_buffer,
            radiance_surface_inspection_values_buffer,
            radiance_surface_inspection_readback_buffer,
            radiance_surface_inspection_readback_receiver: None,
            radiance_surface_inspection_request: None,
            radiance_surface_inspection_mesh_snapshot: None,
            radiance_surface_inspection: RendererRadianceSurfaceInspection::default(),
            sdf_gi_enabled: false,
            gi_radiance_frozen: false,
            gi_radiance_temporal_accumulation_enabled: true,
            gi_radiance_probe_relocation_enabled: true,
            irradiance_probe_ages: [0; MAX_IRRADIANCE_PROBES],
            irradiance_update_budget: 8,
            sky_lighting,
            visibility_grid,
            visibility_bounds,
            visibility_rebuild_cooldown: 0,
            static_probe_samples,
            dynamic_probe_samples,
            static_probe_signature,
            static_probe_samples_dirty: false,
            dynamic_probe_samples_dirty: false,
            shadow_bounds,
            performance_stats: RendererPerformanceStats { triangle_bvh_build_ms, triangle_bvh_node_count: triangle_bvh_nodes.len() as u32, triangle_bvh_triangle_count: triangle_bvh_triangles.len() as u32, ..RendererPerformanceStats::default() },
            frame_index: 0,
            shadow_scheduler: {
                let mut scheduler = ShadowScheduler::default();
                scheduler.rebuild(lights, camera, RenderSettings::default().shadow_budget);
                scheduler
            },
            shadow_resources: ShadowResourceTable::default(),
            shadow_pipeline,
            _shadow_texture: shadow_texture,
            shadow_views,
            _shadow_sampler: shadow_sampler,
            shadow_uniform_buffers,
            shadow_bind_groups,
            shadow_matrices,
            shadow_cascade_splits,
            local_mesh_bounds,
            shadow_caster_bounds,
            spot_shadow_pipeline,
            _spot_shadow_texture: spot_shadow_texture,
            spot_shadow_view,
            _point_shadow_texture: point_shadow_texture,
            point_shadow_views,
            _spot_shadow_sampler: spot_shadow_sampler,
            spot_shadow_uniform_buffer,
            spot_shadow_bind_group,
            point_shadow_uniform_buffers,
            point_shadow_bind_groups,
            _depth_texture: depth_texture,
            depth_view,
            _depth_sample_texture: depth_sample_texture,
            depth_sample_view,
            _surface_normal_texture: surface_normal_texture,
            surface_normal_view,
            _surface_id_texture: surface_id_texture,
            surface_id_view,
            depth_bind_group_layout,
            depth_bind_group,
            triangle_bvh_node_buffer,
            triangle_bvh_triangle_buffer,
            triangle_bvh_nodes: triangle_bvh_nodes.clone(),
            triangle_bvh_triangles: triangle_bvh_triangles.clone(),
            triangle_bvh_settings_buffer,
            triangle_bvh_settings,
            triangle_bvh_diagnostics_buffer,
            triangle_bvh_diagnostics_readback_buffer,
            triangle_bvh_diagnostics_enabled: false,
            triangle_bvh_build_strategy,
            _depth_sampler: depth_sampler,
            has_presented: false,
            camera: *camera,
            lights: lights.to_vec(),
            ui_pipeline,
            ui_vertex_buffer,
            ui_vertex_capacity: 6,
            ui_vertex_count: 0,
            gizmo_pipeline,
            gi_gizmo_bind_group,
            gi_gizmo_selection_buffer,
            gizmo_vertex_buffer,
            gizmo_vertex_capacity: 2,
            gizmo_vertex_count: 0,
            angular_segment_vertex_buffer,
            angular_segment_vertex_capacity: 3,
            angular_segment_vertex_count: 0,
            radiance_angular_resolution: RadianceAngularResolution::default(),
            radiance_angular_segments: [RendererRadianceAngularSegment::default(); RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS],
            radiance_angular_segments_cell: [u32::MAX; 3],
            world_radiance_gizmos_enabled: false,
            world_radiance_trace_gizmos_enabled: false,
            selected_radiance_cell: [4, 1, 4],
            sdf_occupancy_gizmos_enabled: false,
            render_debug_mode: RenderDebugMode::default(),
            render_settings: RenderSettings::default(),
            gpu_timestamps,
            sdf_gpu_updated_this_frame: false,
        })
    }

    pub fn update_instances(&mut self, instances: &[MeshInstance]) -> Result<(), String> {
        let update_start = Instant::now();

        if instances.iter().any(|instance| instance.mesh.is_empty()) {
            return Err("renderer does not accept empty mesh instances".into());
        }
        let topology_signature = mesh_topology_signature(instances);
        self.instance_signature_scratch.clear();
        self.instance_signature_scratch.extend(instances.iter().map(mesh_instance_signature));
        let (unique_vertex_count, unique_index_count) = unique_mesh_geometry_counts(instances);
        let expected_vertex_bytes = unique_vertex_count * mem::size_of::<RendererMeshVertex>();
        let topology_changed = topology_signature != self.mesh_topology_signature || unique_index_count as u32 != self.index_count || expected_vertex_bytes.max(mem::size_of::<RendererMeshVertex>()) != self.vertex_buffer.size() as usize;
        let instances_changed = self.instance_signature_scratch != self.instance_signatures;
        if !topology_changed && !instances_changed {
            self.performance_stats.instance_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
            return Ok(());
        }

        if topology_changed {
            let (vertices, indices, mesh_draw_batches) = flatten_meshes(instances)?;
            let dummy_vertex = RendererMeshVertex::zeroed();
            let vertex_contents = if vertices.is_empty() { bytemuck::bytes_of(&dummy_vertex) } else { bytemuck::cast_slice(&vertices) };
            let dummy_index = 0_u32;
            let index_contents = if indices.is_empty() { bytemuck::bytes_of(&dummy_index) } else { bytemuck::cast_slice(&indices) };
            self.mesh_draw_batches = mesh_draw_batches;
            self.material_texture_bindings = create_material_texture_bindings(&self.device, &self.queue, &self.material_texture_bind_group_layout, &self.material_texture_sampler, instances);
            (self.instance_data_scratch, self.instance_data_offsets) = build_instance_data(instances, &self.mesh_draw_batches);
            let dummy_instance = RendererInstanceData::zeroed();
            let instance_contents = if self.instance_data_scratch.is_empty() { bytemuck::bytes_of(&dummy_instance) } else { bytemuck::cast_slice(&self.instance_data_scratch) };
            self.instance_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh instance data"), contents: instance_contents, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });
            self.vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh vertices"), contents: vertex_contents, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST });
            self.index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh indices"), contents: index_contents, usage: wgpu::BufferUsages::INDEX });
            self.index_count = indices.len() as u32;
            self.mesh_topology_signature = topology_signature;
            self.instance_signatures.clone_from(&self.instance_signature_scratch);
            let wireframe_indices = wireframe_indices(&indices);
            let dummy_wireframe_index = 0_u32;
            let wireframe_contents = if wireframe_indices.is_empty() { bytemuck::bytes_of(&dummy_wireframe_index) } else { bytemuck::cast_slice(&wireframe_indices) };
            self.wireframe_index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("wireframe indices"), contents: wireframe_contents, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST });
            self.wireframe_index_count = wireframe_indices.len() as u32;
            self.local_mesh_bounds = mesh_local_bounds(instances);
            self.shadow_caster_bounds = transformed_mesh_bounds(instances, &self.local_mesh_bounds);
            self.shadow_bounds = combined_geometry_bounds(&geometry_bounds(instances));
            self.visibility_bounds.clear();
            self.visibility_rebuild_cooldown = 0;
        } else {
            self.instance_data_dirty_slots.clear();
            for (instance_index, instance) in instances.iter().enumerate() {
                if self.instance_signature_scratch[instance_index] != self.instance_signatures[instance_index] {
                    let data_slot = self.instance_data_offsets[instance_index];
                    self.instance_data_scratch[data_slot] = renderer_instance_data(instance, instance_index);
                    self.instance_data_dirty_slots.push(data_slot);
                }
            }
            self.instance_data_dirty_slots.sort_unstable();
            let mut dirty_start = 0;
            while dirty_start < self.instance_data_dirty_slots.len() {
                let mut dirty_end = dirty_start + 1;
                while dirty_end < self.instance_data_dirty_slots.len() && self.instance_data_dirty_slots[dirty_end] == self.instance_data_dirty_slots[dirty_end - 1] + 1 {
                    dirty_end += 1;
                }
                let data_start = self.instance_data_dirty_slots[dirty_start];
                let data_end = self.instance_data_dirty_slots[dirty_end - 1] + 1;
                let byte_offset = (data_start * mem::size_of::<RendererInstanceData>()) as u64;
                self.queue.write_buffer(&self.instance_buffer, byte_offset, bytemuck::cast_slice(&self.instance_data_scratch[data_start..data_end]));
                dirty_start = dirty_end;
            }
            self.instance_signatures.clone_from(&self.instance_signature_scratch);
            self.shadow_caster_bounds = transformed_mesh_bounds(instances, &self.local_mesh_bounds);
            self.shadow_bounds = combined_geometry_bounds(&self.shadow_caster_bounds);
        }

        if self.render_settings.shadows_enabled {
            self.shadow_scheduler.mark_all_dirty();
        }
        self.dynamic_probe_samples_dirty = true;
        self.sdf_geometry_cache_dirty = true;
        self.surface_inspection_meshes_dirty = true;
        self.triangle_bvh_dirty = true;
        self.triangle_bvh_topology_dirty |= topology_changed;
        let signature = static_probe_signature(instances);
        let authored_gi_active = !self.sdf_gi_enabled && self.render_settings.irradiance_enabled && self.irradiance_volume_layout.iter().any(|volume| volume.enabled);
        if signature != self.static_probe_signature {
            self.static_probe_signature = signature;
            self.static_probe_samples_dirty = true;
        }

        if authored_gi_active && self.static_probe_samples_dirty {
            (self.static_probe_samples, self.dynamic_probe_samples) = build_probe_samples(instances, true);
            self.static_probe_samples_dirty = false;
            self.dynamic_probe_samples_dirty = false;
        } else if authored_gi_active {
            self.dynamic_probe_samples = build_dynamic_probe_samples(instances);
            self.dynamic_probe_samples_dirty = false;
        }
        self.performance_stats.instance_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;

        Ok(())
    }

    fn rebuild_triangle_bvh(&mut self) {
        let triangle_bvh_build_start = Instant::now();
        let (nodes, triangles) = build_triangle_bvh(&self.surface_inspection_meshes, self.triangle_bvh_build_strategy);
        self.performance_stats.triangle_bvh_build_ms = triangle_bvh_build_start.elapsed().as_secs_f32() * 1000.0;
        self.performance_stats.triangle_bvh_refit_ms = 0.0;
        self.performance_stats.triangle_bvh_node_count = nodes.len() as u32;
        self.performance_stats.triangle_bvh_triangle_count = triangles.len() as u32;
        self.triangle_bvh_nodes = nodes;
        self.triangle_bvh_triangles = triangles;
        self.triangle_bvh_node_buffer = storage_buffer_or_dummy(&self.device, "triangle visibility BVH nodes", &self.triangle_bvh_nodes);
        self.triangle_bvh_triangle_buffer = storage_buffer_or_dummy(&self.device, "triangle visibility BVH triangles", &self.triangle_bvh_triangles);
        self.triangle_bvh_settings = TriangleBvhSettings { counts: [self.triangle_bvh_nodes.len() as u32, self.triangle_bvh_triangles.len() as u32, u32::from(self.triangle_bvh_diagnostics_enabled), 0] };
        self.queue.write_buffer(&self.triangle_bvh_settings_buffer, 0, bytemuck::bytes_of(&self.triangle_bvh_settings));
        self.depth_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene depth bind group"),
            layout: &self.depth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.depth_sample_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self._depth_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.surface_normal_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.surface_id_view) },
                wgpu::BindGroupEntry { binding: 4, resource: self.triangle_bvh_node_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.triangle_bvh_triangle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.triangle_bvh_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.triangle_bvh_diagnostics_buffer.as_entire_binding() },
            ],
        });
    }

    fn refit_triangle_bvh(&mut self) -> bool {
        let refit_start = Instant::now();

        if !refit_triangle_bvh_data(&mut self.triangle_bvh_nodes, &mut self.triangle_bvh_triangles, &self.surface_inspection_meshes) {
            return false;
        }

        if !self.triangle_bvh_nodes.is_empty() {
            self.queue.write_buffer(&self.triangle_bvh_node_buffer, 0, bytemuck::cast_slice(&self.triangle_bvh_nodes));
        }

        if !self.triangle_bvh_triangles.is_empty() {
            self.queue.write_buffer(&self.triangle_bvh_triangle_buffer, 0, bytemuck::cast_slice(&self.triangle_bvh_triangles));
        }
        self.performance_stats.triangle_bvh_refit_ms = refit_start.elapsed().as_secs_f32() * 1000.0;

        true
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        let (depth_texture, depth_view) = create_depth_texture(&self.device, width, height);
        let (depth_sample_texture, depth_sample_view) = create_depth_sample_texture(&self.device, width, height);
        let (surface_normal_texture, surface_normal_view) = create_surface_normal_texture(&self.device, width, height);
        let (surface_id_texture, surface_id_view) = create_surface_id_texture(&self.device, width, height);
        self._depth_texture = depth_texture;
        self.depth_view = depth_view;
        self._depth_sample_texture = depth_sample_texture;
        self.depth_sample_view = depth_sample_view;
        self._surface_normal_texture = surface_normal_texture;
        self.surface_normal_view = surface_normal_view;
        self._surface_id_texture = surface_id_texture;
        self.surface_id_view = surface_id_view;
        self.depth_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene depth bind group"),
            layout: &self.depth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.depth_sample_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self._depth_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&self.surface_normal_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.surface_id_view) },
                wgpu::BindGroupEntry { binding: 4, resource: self.triangle_bvh_node_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.triangle_bvh_triangle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.triangle_bvh_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.triangle_bvh_diagnostics_buffer.as_entire_binding() },
            ],
        });
        let camera_uniform = camera_uniform(width, height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));
    }

    pub fn update_camera(&mut self, camera: &Camera) -> Result<(), String> {
        if self.camera == *camera {
            return Ok(());
        }
        self.camera = *camera;
        if self.render_settings.shadows_enabled {
            self.shadow_scheduler.rebuild(&self.lights, &self.camera, self.render_settings.shadow_budget);
            self.rebuild_shadow_resources();
            self.update_shadow_scheduler_stats();
            self.shadow_matrices = self
                .lights
                .iter()
                .find_map(|light| match light {
                    Light::Directional(light) => Some(cascade_shadow_view_projections(light, &self.camera, self.config.width, self.config.height, self.render_settings.shadow_cascade_count)),
                    _ => None,
                })
                .unwrap_or([Mat4::IDENTITY; SHADOW_CASCADE_COUNT]);
            self.shadow_cascade_splits = cascade_splits_for_count(&self.camera, self.render_settings.shadow_cascade_count);
            for (cascade_index, matrix) in self.shadow_matrices.iter().enumerate() {
                self.queue
                    .write_buffer(&self.shadow_uniform_buffers[cascade_index], 0, bytemuck::bytes_of(&ShadowUniform { view_projection: matrix.to_cols_array_2d() }));
            }
        }
        let camera_uniform = camera_uniform(self.config.width, self.config.height, camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));

        Ok(())
    }

    pub fn update_lights(&mut self, lights: &[Light]) -> Result<(), String> {
        let update_start = Instant::now();

        if lights.len() > MAX_DIRECTIONAL_LIGHTS {
            return Err(format!("renderer supports at most {MAX_DIRECTIONAL_LIGHTS} directional lights"));
        }

        if self.lights == lights {
            self.performance_stats.light_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
            return Ok(());
        }
        let previous_lights = self.lights.clone();
        if previous_lights != lights && self.sdf_gi_enabled && self.render_settings.irradiance_enabled {
            let priority = changed_light_priority(&previous_lights, lights);
            self.radiance_field.refresh_for_lighting_change(priority);
        }
        self.lights.clear();
        self.lights.extend_from_slice(lights);
        if self.render_settings.shadows_enabled {
            if shadow_geometry_changed(&previous_lights, &self.lights) {
                self.shadow_scheduler.mark_all_dirty();
            }
            self.shadow_scheduler.rebuild(&self.lights, &self.camera, self.render_settings.shadow_budget);
            self.rebuild_shadow_resources();
            self.update_shadow_scheduler_stats();
            let shadow_matrices = self
                .lights
                .iter()
                .find_map(|light| match light {
                    Light::Directional(light) => Some(cascade_shadow_view_projections(light, &self.camera, self.config.width, self.config.height, self.render_settings.shadow_cascade_count)),
                    _ => None,
                })
                .unwrap_or([Mat4::IDENTITY; SHADOW_CASCADE_COUNT]);
            self.shadow_matrices = shadow_matrices;
            self.shadow_cascade_splits = cascade_splits_for_count(&self.camera, self.render_settings.shadow_cascade_count);
            for (cascade_index, matrix) in shadow_matrices.iter().enumerate() {
                self.queue
                    .write_buffer(&self.shadow_uniform_buffers[cascade_index], 0, bytemuck::bytes_of(&ShadowUniform { view_projection: matrix.to_cols_array_2d() }));
            }
            let spot_shadow_matrix = self
                .lights
                .iter()
                .find_map(|light| match light {
                    Light::Spot(light) => Some(spot_shadow_view_projection(light)),
                    _ => None,
                })
                .unwrap_or(Mat4::IDENTITY);
            self.queue
                .write_buffer(&self.spot_shadow_uniform_buffer, 0, bytemuck::bytes_of(&SpotShadowUniform { view_projection: spot_shadow_matrix.to_cols_array_2d() }));
        }
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        self.performance_stats.light_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;

        Ok(())
    }

    pub fn update_irradiance_volumes(&mut self, volumes: &[RendererIrradianceVolume], instances: &[MeshInstance], lights: &[Light], sdf_gi_enabled: bool) {
        self.sdf_gpu_updated_this_frame = false;
        if sdf_gi_enabled != self.sdf_gi_enabled {
            self.sdf_gi_enabled = sdf_gi_enabled;
            self.radiance_field.invalidate();
            self.reset_radiance_field_coverage_stats();
        }
        self.performance_stats.sdf_build_cpu_ms = 0.0;
        self.performance_stats.sdf_geometry_bvh_build_ms = 0.0;
        self.performance_stats.sdf_voxel_build_ms = 0.0;
        self.performance_stats.sdf_clearance_build_ms = 0.0;
        self.performance_stats.sdf_upload_ms = 0.0;
        self.performance_stats.sdf_voxels_updated = 0;
        self.performance_stats.gi_pending_cells = self.radiance_field.pending_cell_count() as u32;
        self.performance_stats.gi_cells_updated = 0;
        self.performance_stats.gi_rays_scheduled = 0;
        self.performance_stats.gi_angular_regions = self.radiance_angular_resolution.segment_count();
        if self.surface_inspection_meshes_dirty {
            self.surface_inspection_meshes = surface_inspection_meshes(instances);
            self.surface_inspection_meshes_dirty = false;
        }

        if !sdf_gi_enabled && (!self.render_settings.irradiance_enabled || volumes.is_empty()) {
            self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(false, self.sky_lighting)));
            self.performance_stats.probe_update_ms = 0.0;
            self.performance_stats.irradiance_upload_ms = 0.0;
            self.performance_stats.visibility_rebuild_ms = 0.0;
            self.performance_stats.probes_updated = 0;
            return;
        }

        if sdf_gi_enabled && self.render_settings.irradiance_enabled {
            if self.sdf_geometry_cache_dirty {
                self.sdf_geometry_cache = sdf_geometries(instances);
                self.sdf_geometry_cache_dirty = false;
            }

            if (self.render_settings.triangle_probe_occlusion_enabled || sdf_gi_enabled) && self.triangle_bvh_dirty {
                if self.triangle_bvh_topology_dirty || !self.refit_triangle_bvh() {
                    self.rebuild_triangle_bvh();
                }
                self.triangle_bvh_dirty = false;
                self.triangle_bvh_topology_dirty = false;
            }
        } else if self.render_settings.irradiance_enabled && self.static_probe_samples_dirty {
            (self.static_probe_samples, self.dynamic_probe_samples) = build_probe_samples(instances, true);
            self.static_probe_samples_dirty = false;
            self.dynamic_probe_samples_dirty = false;
        } else if self.render_settings.irradiance_enabled && self.dynamic_probe_samples_dirty {
            self.dynamic_probe_samples = build_dynamic_probe_samples(instances);
            self.dynamic_probe_samples_dirty = false;
        }

        if !sdf_gi_enabled || !self.render_settings.irradiance_enabled {
            self.performance_stats.gpu_sdf_gi_ms = 0.0;
        }

        if sdf_gi_enabled && !self.render_settings.irradiance_enabled {
            self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(false, self.sky_lighting)));
            self.performance_stats.probe_update_ms = 0.0;
            self.performance_stats.irradiance_upload_ms = 0.0;
            self.performance_stats.visibility_rebuild_ms = 0.0;
            self.performance_stats.probes_updated = 0;
            return;
        }

        if sdf_gi_enabled {
            let sdf_build_start = Instant::now();
            let geometry_changed = self.sdf_clipmap.geometry_bounds != self.sdf_geometry_cache;
            let scene_bounds = combined_geometry_bounds(&geometry_bounds(instances));
            let sdf_update = self.sdf_clipmap.update_gpu(scene_bounds, &self.sdf_geometry_cache);
            let field_reanchored = self.radiance_field.reanchor(scene_bounds);
            if field_reanchored {
                self.reset_radiance_field_coverage_stats();
                self.queue.write_buffer(
                    &self._radiance_field_buffer,
                    0,
                    bytemuck::cast_slice(&vec![RadianceFieldAngularSample::zeroed(); self.radiance_field.cell_count() * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS]),
                );
                self.queue.write_buffer(
                    &self._radiance_field_bounce_buffer,
                    0,
                    bytemuck::cast_slice(&vec![[0.0_f32; 4]; self.radiance_field.cell_count() * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS]),
                );
                self.queue.write_buffer(&self._radiance_field_probe_buffer, 0, bytemuck::cast_slice(&vec![[0.0_f32; 4]; self.radiance_field.cell_count()]));
                self.queue.write_buffer(&self._radiance_field_coverage_buffer, 0, bytemuck::cast_slice(&vec![[0_u32; 4]; self.radiance_field.cell_count()]));
            }
            self.performance_stats.sdf_build_cpu_ms = sdf_build_start.elapsed().as_secs_f32() * 1000.0;
            if geometry_changed {
                self.radiance_field.refresh_temporally();
            }

            if let Some(sdf_update) = sdf_update {
                self.performance_stats.sdf_voxels_updated = sdf_update.voxels_updated;
                self.performance_stats.sdf_geometry_bvh_build_ms = sdf_update.geometry_bvh_build_ms;
                self.performance_stats.sdf_voxel_build_ms = sdf_update.voxel_build_ms;
                self.performance_stats.sdf_clearance_build_ms = sdf_update.clearance_build_ms;
                let sdf_upload_start = Instant::now();
                self.update_sdf_gpu(&sdf_update);
                self.sdf_gpu_updated_this_frame = true;
                self.performance_stats.sdf_upload_ms = sdf_upload_start.elapsed().as_secs_f32() * 1000.0;
            }
            let radiance_field_uniform = self.radiance_field.uniform(
                self.gi_radiance_temporal_accumulation_enabled,
                self.radiance_angular_resolution.segment_count(),
                self.gi_radiance_probe_relocation_enabled,
                self.gi_radiance_frozen,
                self.camera.transform.position,
            );
            self.performance_stats.gi_cells_updated = radiance_field_uniform.schedule_metadata[0];
            self.performance_stats.gi_rays_scheduled = radiance_field_uniform.schedule_metadata[1];
            self.performance_stats.gi_pending_cells = self.radiance_field.pending_cell_count() as u32;
            self.queue.write_buffer(&self.radiance_field_uniform_buffer, 0, bytemuck::bytes_of(&radiance_field_uniform));
            self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(true, self.sky_lighting)));
            self.performance_stats.probe_update_ms = 0.0;
            self.performance_stats.irradiance_upload_ms = 0.0;
            self.performance_stats.visibility_rebuild_ms = 0.0;
            self.performance_stats.probes_updated = 0;
            return;
        }
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(false, self.sky_lighting)));
        if !self.render_settings.irradiance_enabled {
            self.performance_stats.probe_update_ms = 0.0;
            self.performance_stats.irradiance_upload_ms = 0.0;
            self.performance_stats.visibility_rebuild_ms = 0.0;
            self.performance_stats.probes_updated = 0;
            return;
        }
        let visibility_start = Instant::now();
        let geometry_bounds = geometry_bounds(instances);
        let mut visibility_rebuild_ms = 0.0;
        if self.visibility_rebuild_cooldown > 0 {
            self.visibility_rebuild_cooldown -= 1;
        }

        if geometry_bounds != self.visibility_bounds && self.visibility_rebuild_cooldown == 0 {
            self.visibility_grid = VisibilityGrid::from_bounds(&geometry_bounds);
            self.visibility_bounds = geometry_bounds;
            refresh_probe_locations(self.irradiance_cache.as_mut(), volumes, &self.visibility_grid);
            refresh_probe_locations(self.irradiance_direct_cache.as_mut(), volumes, &self.visibility_grid);
            self.irradiance_probe_ages.fill(u32::MAX);
            self.visibility_rebuild_cooldown = VISIBILITY_REBUILD_INTERVAL;
            visibility_rebuild_ms = visibility_start.elapsed().as_secs_f32() * 1000.0;
        }

        if volumes != self.irradiance_volume_layout.as_slice() {
            let old_cache = self.irradiance_cache.clone();
            let old_direct_cache = self.irradiance_direct_cache.clone();
            let old_ages = self.irradiance_probe_ages;
            let mut new_cache = Box::new(irradiance_uniform(
                volumes,
                &self.static_probe_samples,
                &self.dynamic_probe_samples,
                lights,
                self.sky_lighting,
                &self.visibility_grid,
                false,
            ));
            let mut new_direct_cache = Box::new(irradiance_uniform(
                volumes,
                &self.static_probe_samples,
                &self.dynamic_probe_samples,
                lights,
                self.sky_lighting,
                &self.visibility_grid,
                false,
            ));
            let mut new_ages = [u32::MAX; MAX_IRRADIANCE_PROBES];
            preserve_overlapping_probes(&old_cache, &old_ages, &mut new_cache, &mut new_ages);
            preserve_overlapping_probes(&old_direct_cache, &old_ages, &mut new_direct_cache, &mut new_ages);
            self.irradiance_cache = new_cache;
            self.irradiance_direct_cache = new_direct_cache;
            self.irradiance_probe_ages = new_ages;
            self.irradiance_volume_layout = volumes.to_vec();
        }

        let active_volumes = volumes.iter().filter(|volume| volume.enabled).take(MAX_IRRADIANCE_VOLUMES).collect::<Vec<_>>();
        let total_probe_count = active_volumes
            .iter()
            .map(|volume| volume.resolution[0].max(1) as usize * volume.resolution[1].max(1) as usize * volume.resolution[2].max(1) as usize)
            .sum::<usize>()
            .min(MAX_IRRADIANCE_PROBES);
        let probe_start = Instant::now();
        let mut probes_updated = 0_u32;
        if total_probe_count > 0 {
            for age in self.irradiance_probe_ages.iter_mut().take(total_probe_count) {
                *age = age.saturating_add(1);
            }
            let mut updated = [false; MAX_IRRADIANCE_PROBES];
            for _ in 0..self.irradiance_update_budget.max(1) {
                let mut best_index = None;
                let mut best_score = f32::MIN;
                for (index, was_updated) in updated.iter().enumerate().take(total_probe_count) {
                    if *was_updated {
                        continue;
                    }
                    let Some(location) = probe_location_for_index(&active_volumes, index, &self.visibility_grid) else { continue };
                    let probe_position = [location.position.x, location.position.y, location.position.z, 1.0];
                    self.irradiance_cache.probe_positions[index] = probe_position;
                    self.irradiance_direct_cache.probe_positions[index] = probe_position;
                    let position = location.position;
                    let distance_score = 4.0 / (1.0 + self.camera.transform.position.distance_squared(position));
                    let score = self.irradiance_probe_ages[index] as f32 * 0.15 + distance_score;
                    if score > best_score {
                        best_index = Some(index);
                        best_score = score;
                    }
                }
                let Some(index) = best_index else { break };
                let Some(location) = probe_location_for_index(&active_volumes, index, &self.visibility_grid) else { continue };
                let probe_position = [location.position.x, location.position.y, location.position.z, 1.0];
                self.irradiance_cache.probe_positions[index] = probe_position;
                self.irradiance_direct_cache.probe_positions[index] = probe_position;
                let position = location.position;
                let (color, sky_visibility, direction) = probe_irradiance(position, &self.static_probe_samples, &self.dynamic_probe_samples, lights, &self.visibility_grid, self.sky_lighting);
                let blend = if self.irradiance_probe_ages[index] == u32::MAX { 1.0 } else { 0.25 };
                let previous_color = Vec3::new(self.irradiance_direct_cache.probes[index][0], self.irradiance_direct_cache.probes[index][1], self.irradiance_direct_cache.probes[index][2]);
                let blended_color = previous_color.lerp(color, blend);
                let previous_visibility = self.irradiance_direct_cache.probes[index][3];
                let blended_visibility = previous_visibility + (sky_visibility - previous_visibility) * blend;
                self.irradiance_direct_cache.probes[index] = [blended_color.x, blended_color.y, blended_color.z, blended_visibility];
                let previous_direction = Vec4::from_array(self.irradiance_direct_cache.directions[index]);
                let blended_direction = previous_direction.lerp(direction, blend);
                self.irradiance_direct_cache.directions[index] = blended_direction.to_array();
                let (second_bounce, second_bounce_direction) = probe_second_diffuse_bounce(position, &self.static_probe_samples, &self.dynamic_probe_samples, &self.irradiance_direct_cache, &self.visibility_grid);
                let final_color = blended_color + second_bounce;
                let final_direction = (blended_direction.truncate() * blended_direction.w + second_bounce_direction.truncate() * second_bounce_direction.w).normalize_or_zero();
                let final_direction_weight = (blended_direction.w + second_bounce_direction.w).clamp(0.0, 1.0);
                self.irradiance_cache.probes[index] = [final_color.x, final_color.y, final_color.z, blended_visibility];
                self.irradiance_cache.directions[index] = final_direction.extend(final_direction_weight).to_array();
                self.irradiance_probe_ages[index] = 0;
                updated[index] = true;
                probes_updated += 1;
            }
        }
        let upload_start = Instant::now();
        self.queue.write_buffer(&self.irradiance_buffer, 0, bytemuck::bytes_of(self.irradiance_cache.as_ref()));
        self.performance_stats = RendererPerformanceStats {
            frame_cpu_ms: self.performance_stats.frame_cpu_ms,
            sdf_build_cpu_ms: self.performance_stats.sdf_build_cpu_ms,
            sdf_geometry_bvh_build_ms: self.performance_stats.sdf_geometry_bvh_build_ms,
            sdf_voxel_build_ms: self.performance_stats.sdf_voxel_build_ms,
            sdf_clearance_build_ms: self.performance_stats.sdf_clearance_build_ms,
            sdf_upload_ms: self.performance_stats.sdf_upload_ms,
            sdf_voxels_updated: self.performance_stats.sdf_voxels_updated,
            gi_region_coverage_percent: self.performance_stats.gi_region_coverage_percent,
            gi_regions_covered_twice_percent: self.performance_stats.gi_regions_covered_twice_percent,
            gi_cells_with_coverage_percent: self.performance_stats.gi_cells_with_coverage_percent,
            gi_valid_cells_percent: self.performance_stats.gi_valid_cells_percent,
            gi_regions_attempted_percent: self.performance_stats.gi_regions_attempted_percent,
            gi_coverage_available: self.performance_stats.gi_coverage_available,
            gi_pending_cells: self.performance_stats.gi_pending_cells,
            gi_cells_updated: self.performance_stats.gi_cells_updated,
            gi_rays_scheduled: self.performance_stats.gi_rays_scheduled,
            gi_angular_regions: self.performance_stats.gi_angular_regions,
            gpu_shadow_ms: self.performance_stats.gpu_shadow_ms,
            gpu_scene_ms: self.performance_stats.gpu_scene_ms,
            gpu_sdf_gi_ms: self.performance_stats.gpu_sdf_gi_ms,
            gpu_sdf_update_ms: self.performance_stats.gpu_sdf_update_ms,
            gpu_gizmo_ms: self.performance_stats.gpu_gizmo_ms,
            gpu_ui_ms: self.performance_stats.gpu_ui_ms,
            gpu_total_ms: self.performance_stats.gpu_total_ms,
            scheduled_shadow_lights: self.performance_stats.scheduled_shadow_lights,
            dirty_shadow_lights: self.performance_stats.dirty_shadow_lights,
            triangle_bvh_segments: self.performance_stats.triangle_bvh_segments,
            triangle_bvh_bounds_tests: self.performance_stats.triangle_bvh_bounds_tests,
            triangle_bvh_triangle_tests: self.performance_stats.triangle_bvh_triangle_tests,
            triangle_bvh_hits: self.performance_stats.triangle_bvh_hits,
            triangle_bvh_build_ms: self.performance_stats.triangle_bvh_build_ms,
            triangle_bvh_refit_ms: self.performance_stats.triangle_bvh_refit_ms,
            triangle_bvh_node_count: self.performance_stats.triangle_bvh_node_count,
            triangle_bvh_triangle_count: self.performance_stats.triangle_bvh_triangle_count,
            light_update_ms: self.performance_stats.light_update_ms,
            instance_update_ms: self.performance_stats.instance_update_ms,
            probe_update_ms: probe_start.elapsed().as_secs_f32() * 1000.0,
            irradiance_upload_ms: upload_start.elapsed().as_secs_f32() * 1000.0,
            visibility_rebuild_ms,
            probes_updated,
            probe_budget: self.irradiance_update_budget as u32,
            static_sample_count: self.static_probe_samples.len() as u32,
            dynamic_sample_count: self.dynamic_probe_samples.len() as u32,
            region_count: self.visibility_grid.regions.iter().copied().max().unwrap_or(0) as u32,
        };
    }

    pub fn set_irradiance_update_budget(&mut self, probes_per_frame: usize) {
        self.irradiance_update_budget = probes_per_frame.max(1);
    }

    pub fn performance_stats(&self) -> RendererPerformanceStats {
        self.performance_stats
    }

    pub fn gpu_timing_available(&self) -> bool {
        self.gpu_timestamps.is_some()
    }

    fn reset_radiance_field_coverage_stats(&mut self) {
        self.performance_stats.gi_region_coverage_percent = 0.0;
        self.performance_stats.gi_regions_covered_twice_percent = 0.0;
        self.performance_stats.gi_cells_with_coverage_percent = 0.0;
        self.performance_stats.gi_valid_cells_percent = 0.0;
        self.performance_stats.gi_regions_attempted_percent = 0.0;

        self.performance_stats.gi_coverage_available = false;
    }

    pub fn set_triangle_bvh_diagnostics_enabled(&mut self, enabled: bool) {
        if self.triangle_bvh_diagnostics_enabled == enabled {
            return;
        }
        self.triangle_bvh_diagnostics_enabled = enabled;
        self.triangle_bvh_settings.counts[2] = u32::from(enabled);
        self.queue.write_buffer(&self.triangle_bvh_settings_buffer, 0, bytemuck::bytes_of(&self.triangle_bvh_settings));
        if !enabled {
            self.performance_stats.triangle_bvh_segments = 0;
            self.performance_stats.triangle_bvh_bounds_tests = 0;
            self.performance_stats.triangle_bvh_triangle_tests = 0;
            self.performance_stats.triangle_bvh_hits = 0;
        }
    }

    pub fn set_triangle_bvh_sah_enabled(&mut self, enabled: bool) {
        let build_strategy = if enabled { TriangleBvhBuildStrategy::Sah } else { TriangleBvhBuildStrategy::Median };

        if self.triangle_bvh_build_strategy == build_strategy {
            return;
        }
        self.triangle_bvh_build_strategy = build_strategy;

        self.rebuild_triangle_bvh();
    }

    pub fn read_triangle_bvh_diagnostics(&mut self) -> Result<(), String> {
        if !self.triangle_bvh_diagnostics_enabled {
            return Ok(());
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("triangle BVH diagnostics readback") });
        encoder.copy_buffer_to_buffer(
            &self.triangle_bvh_diagnostics_buffer,
            0,
            &self.triangle_bvh_diagnostics_readback_buffer,
            0,
            mem::size_of::<TriangleBvhCounterBuffer>() as u64,
        );
        self.queue.submit([encoder.finish()]);
        let slice = self.triangle_bvh_diagnostics_readback_buffer.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
        let counters = bytemuck::from_bytes::<TriangleBvhCounterBuffer>(&mapped);
        self.performance_stats.triangle_bvh_segments = counters.counters[0];
        self.performance_stats.triangle_bvh_bounds_tests = counters.counters[1];
        self.performance_stats.triangle_bvh_triangle_tests = counters.counters[2];
        self.performance_stats.triangle_bvh_hits = counters.counters[3];
        drop(mapped);
        self.triangle_bvh_diagnostics_readback_buffer.unmap();

        Ok(())
    }

    pub fn radiance_field_grid(&self) -> RendererRadianceFieldGrid {
        self.radiance_field.grid()
    }

    pub fn viewport_size(&self) -> [u32; 2] {
        [self.config.width, self.config.height]
    }

    pub fn sdf_grid(&self) -> RendererSdfGrid {
        RendererSdfGrid { minimum: self.sdf_clipmap.minimum.to_array(), maximum: self.sdf_clipmap.maximum.to_array(), resolution: self.sdf_clipmap.resolution, voxel_size: self.sdf_clipmap.voxel_size.to_array() }
    }

    pub fn inspect_radiance_probe(&self, coordinate: [u32; 3]) -> Result<Option<RendererRadianceProbeDebug>, String> {
        let resolution = self.radiance_field.resolution;

        if coordinate.iter().zip(resolution).any(|(coordinate, count)| *coordinate >= count) {
            return Ok(None);
        }
        let cell_index = self.radiance_field.index(coordinate) as u64;
        let segment_count = self.radiance_angular_resolution.segment_count() as usize;
        let samples_size = mem::size_of::<RadianceFieldAngularSample>() as u64 * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS as u64;
        let probe_offset = samples_size;
        let coverage_offset = probe_offset + mem::size_of::<[f32; 4]>() as u64;
        let readback_size = coverage_offset + mem::size_of::<[u32; 4]>() as u64;
        let readback = self
            .device
            .create_buffer(&wgpu::BufferDescriptor { label: Some("radiance probe MCP readback"), size: readback_size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("radiance probe MCP readback") });
        encoder.copy_buffer_to_buffer(&self._radiance_field_buffer, cell_index * samples_size, &readback, 0, samples_size);
        encoder.copy_buffer_to_buffer(
            &self._radiance_field_probe_buffer,
            cell_index * mem::size_of::<[f32; 4]>() as u64,
            &readback,
            probe_offset,
            mem::size_of::<[f32; 4]>() as u64,
        );
        encoder.copy_buffer_to_buffer(
            &self._radiance_field_coverage_buffer,
            cell_index * mem::size_of::<[u32; 4]>() as u64,
            &readback,
            coverage_offset,
            mem::size_of::<[u32; 4]>() as u64,
        );
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
        let samples = bytemuck::cast_slice::<u8, RadianceFieldAngularSample>(&mapped[..samples_size as usize]);
        let probe_position = bytemuck::from_bytes::<[f32; 4]>(&mapped[probe_offset as usize..coverage_offset as usize]);
        let coverage = bytemuck::from_bytes::<[u32; 4]>(&mapped[coverage_offset as usize..readback_size as usize]);
        let (bands, sectors) = self.radiance_angular_resolution.bands_and_sectors();
        let angular_segments = samples
            .iter()
            .take(segment_count)
            .enumerate()
            .map(|(index, sample)| {
                let band = index as u32 / sectors;
                let sector = index as u32 % sectors;
                let height = 1.0 - 2.0 * (band as f32 + 0.5) / bands as f32;
                let azimuth = std::f32::consts::TAU * (sector as f32 + 0.5) / sectors as f32;
                let radius = (1.0 - height * height).max(0.0).sqrt();
                RendererRadianceAngularSegment {
                    sampled: sample.sampling[0] > 0,
                    direction: [radius * azimuth.cos(), height, radius * azimuth.sin()],
                    radiance: [sample.radiance[0], sample.radiance[1], sample.radiance[2]],
                    sample_count: sample.sampling[0],
                    attempt_count: sample.sampling[1],
                    age_frames: if sample.sampling[0] > 0 { self.radiance_field.frame_index.wrapping_sub(sample.sampling[2]) } else { u32::MAX },
                }
            })
            .collect();
        let center = self.radiance_field.minimum + (Vec3::from_array(coordinate.map(|value| value as f32)) + Vec3::splat(0.5)) * self.radiance_field.cell_size;
        let result = RendererRadianceProbeDebug { coordinate, cell_center: center.to_array(), probe_position: [probe_position[0], probe_position[1], probe_position[2]], relocation_valid: coverage[3] > 0, relocation_succeeded: probe_position[3] > 0.0, covered_once: coverage[0], covered_twice: coverage[1], attempted_regions: coverage[2], angular_segments };
        drop(mapped);
        readback.unmap();

        Ok(Some(result))
    }

    pub fn inspect_sdf_voxel(&self, coordinate: [u32; 3]) -> Result<Option<RendererSdfVoxelDebug>, String> {
        if coordinate.iter().zip(self.sdf_clipmap.resolution).any(|(coordinate, count)| *coordinate >= count) {
            return Ok(None);
        }
        let readback = self
            .device
            .create_buffer(&wgpu::BufferDescriptor { label: Some("SDF voxel MCP readback"), size: 768, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let origin = wgpu::Origin3d { x: coordinate[0], y: coordinate[1], z: coordinate[2] };
        let extent = wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
        let layout = |offset| wgpu::TexelCopyBufferLayout { offset, bytes_per_row: Some(256), rows_per_image: Some(1) };
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("SDF voxel MCP readback") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_texture, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: layout(0) },
            extent,
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_emission_texture, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: layout(256) },
            extent,
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_clearance_texture, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: layout(512) },
            extent,
        );
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
        let sdf = bytemuck::from_bytes::<[f32; 4]>(&mapped[0..16]);
        let emission = bytemuck::from_bytes::<[f32; 4]>(&mapped[256..272]);
        let clearance = bytemuck::from_bytes::<f32>(&mapped[512..516]);
        let world_position = self.sdf_clipmap.minimum + (Vec3::from_array(coordinate.map(|value| value as f32)) + Vec3::splat(0.5)) * self.sdf_clipmap.voxel_size;
        let result = RendererSdfVoxelDebug { coordinate, world_position: world_position.to_array(), signed_distance: sdf[0], albedo: [sdf[1], sdf[2], sdf[3]], emission: [emission[0], emission[1], emission[2]], clearance: *clearance };
        drop(mapped);
        readback.unmap();

        Ok(Some(result))
    }

    pub fn pick_viewport_pixel(&self, camera: &Camera, x: u32, y: u32, width: u32, height: u32) -> Option<RendererViewportPick> {
        let width = if width == 0 { self.config.width } else { width };
        let height = if height == 0 { self.config.height } else { height };

        if width == 0 || height == 0 || x >= width || y >= height {
            return None;
        }
        let normalized = Vec3::new(2.0 * (x as f32 + 0.5) / width as f32 - 1.0, 1.0 - 2.0 * (y as f32 + 0.5) / height as f32, -1.0);
        let aspect = width as f32 / height as f32;
        let tangent = (camera.field_of_view_y.to_radians() * 0.5).tan();
        let local_direction = Vec3::new(normalized.x * aspect * tangent, normalized.y * tangent, -1.0).normalize();
        let ray_origin = camera.transform.position;
        let ray_direction = (camera.transform.rotation * local_direction).normalize();
        let hit = closest_surface_triangle_hit(ray_origin, ray_origin + ray_direction * camera.far_clip, &self.surface_inspection_meshes)?;
        let position = hit.position;
        let sdf_minimum = self.sdf_clipmap.minimum;
        let sdf_maximum = self.sdf_clipmap.maximum;
        let sdf_voxel = if position.cmpge(sdf_minimum).all() && position.cmplt(sdf_maximum).all() {
            Some(((position - sdf_minimum) / self.sdf_clipmap.voxel_size).floor().as_uvec3().to_array())
        } else {
            None
        };
        let radiance_minimum = self.radiance_field.minimum;
        let radiance_maximum = self.radiance_field.maximum;
        let radiance_cell = if position.cmpge(radiance_minimum).all() && position.cmplt(radiance_maximum).all() {
            Some(self.radiance_field.cell_coordinate(position))
        } else {
            None
        };

        Some(RendererViewportPick { world_position: position.to_array(), world_normal: hit.normal.to_array(), distance: hit.distance, instance_index: hit.instance_index, triangle_index: hit.triangle_index, barycentric: hit.barycentric.to_array(), sdf_voxel, radiance_cell })
    }

    pub fn shadow_requests(&self) -> &[ShadowRequest] {
        &self.shadow_scheduler.requests
    }

    pub fn shadow_resource_allocations(&self) -> &[ShadowResourceAllocation] {
        &self.shadow_resources.allocations
    }

    pub fn shadow_resource_allocation(&self, light_index: usize) -> Option<&ShadowResourceAllocation> {
        self.shadow_resources.allocation_for(light_index)
    }

    pub fn update_gpu_timestamps(&mut self) -> Result<(), String> {
        let Some(timestamps) = &self.gpu_timestamps else {
            return Ok(());
        };
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        let slice = timestamps.readback_buffer.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
        let values = bytemuck::cast_slice::<u8, u64>(&mapped);
        let to_ms = |start: usize, end: usize| {
            if values[end] < values[start] { 0.0 } else { (values[end] - values[start]) as f32 * timestamps.timestamp_period / 1_000_000.0 }
        };
        let scene_ms = to_ms(6, 7);
        let sdf_gi_ms = if self.sdf_gi_enabled && self.render_settings.irradiance_enabled && !self.gi_radiance_frozen { to_ms(4, 5) } else { 0.0 };
        let sdf_update_ms = if self.sdf_gpu_updated_this_frame { to_ms(12, 13) } else { 0.0 };
        let gizmo_ms = if self.gizmo_vertex_count > 0 { to_ms(8, 9) } else { 0.0 };
        let ui_ms = if self.ui_vertex_count > 0 { to_ms(10, 11) } else { 0.0 };
        let mut shadow_ms = 0.0;

        if self.render_settings.shadows_enabled {
            if self.lights.iter().any(|light| matches!(light, Light::Directional(_))) {
                shadow_ms += to_ms(2, 3);
            }

            if self.lights.iter().any(|light| matches!(light, Light::Spot(_))) {
                shadow_ms += to_ms(0, 1);
            }
        }
        drop(mapped);
        timestamps.readback_buffer.unmap();
        self.performance_stats.gpu_shadow_ms = shadow_ms;
        self.performance_stats.gpu_scene_ms = scene_ms;
        self.performance_stats.gpu_gizmo_ms = gizmo_ms;
        self.performance_stats.gpu_ui_ms = ui_ms;
        self.performance_stats.gpu_sdf_gi_ms = sdf_gi_ms;
        self.performance_stats.gpu_sdf_update_ms = sdf_update_ms;
        self.performance_stats.gpu_total_ms = shadow_ms + scene_ms + sdf_gi_ms + sdf_update_ms + gizmo_ms + ui_ms;
        self.sdf_gpu_updated_this_frame = false;

        Ok(())
    }

    pub fn set_frame_cpu_ms(&mut self, frame_cpu_ms: f32) {
        self.performance_stats.frame_cpu_ms = frame_cpu_ms;
    }

    pub fn update_sky_lighting(&mut self, sky_lighting: RendererSkyLighting) {
        if self.sky_lighting == sky_lighting {
            return;
        }
        self.sky_lighting = sky_lighting;
        self.irradiance_probe_ages.fill(u32::MAX);
        self.radiance_field.refresh_temporally();

        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, sky_lighting)));
    }

    pub fn update_render_debug_mode(&mut self, mode: RenderDebugMode) {
        if self.render_debug_mode == mode {
            return;
        }
        self.render_debug_mode = mode;
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, mode, self.render_settings, self.shadow_bounds);

        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
    }

    pub fn update_render_settings(&mut self, settings: RenderSettings) {
        if self.render_settings == settings {
            return;
        }
        let cascade_count_changed = self.render_settings.shadow_cascade_count != settings.shadow_cascade_count;
        let shadows_reenabled = !self.render_settings.shadows_enabled && settings.shadows_enabled;
        let gi_reenabled = !self.render_settings.irradiance_enabled && settings.irradiance_enabled && self.sdf_gi_enabled;
        self.render_settings = settings;
        if gi_reenabled {
            self.radiance_field.invalidate();
        }

        if settings.shadows_enabled {
            if shadows_reenabled {
                self.shadow_scheduler.mark_all_dirty();
            }
            self.shadow_scheduler.rebuild(&self.lights, &self.camera, settings.shadow_budget);
            self.rebuild_shadow_resources();
            self.update_shadow_scheduler_stats();
            if cascade_count_changed || shadows_reenabled {
                self.shadow_matrices = self
                    .lights
                    .iter()
                    .find_map(|light| match light {
                        Light::Directional(light) => Some(cascade_shadow_view_projections(light, &self.camera, self.config.width, self.config.height, settings.shadow_cascade_count)),
                        _ => None,
                    })
                    .unwrap_or([Mat4::IDENTITY; SHADOW_CASCADE_COUNT]);
                self.shadow_cascade_splits = cascade_splits_for_count(&self.camera, settings.shadow_cascade_count);
                for (cascade_index, matrix) in self.shadow_matrices.iter().enumerate() {
                    self.queue
                        .write_buffer(&self.shadow_uniform_buffers[cascade_index], 0, bytemuck::bytes_of(&ShadowUniform { view_projection: matrix.to_cols_array_2d() }));
                }
            }
        }
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, self.render_debug_mode, settings, self.shadow_bounds);

        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
    }

    pub fn update_ui(&mut self, vertices: &[UiVertex]) {
        if vertices.is_empty() {
            self.ui_vertex_count = 0;
            return;
        }

        if vertices.len() > self.ui_vertex_capacity {
            self.ui_vertex_capacity = vertices.len().next_power_of_two();
            self.ui_vertex_buffer = self
                .device
                .create_buffer(&wgpu::BufferDescriptor { label: Some("UI vertices"), size: (self.ui_vertex_capacity * mem::size_of::<UiVertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        self.queue.write_buffer(&self.ui_vertex_buffer, 0, bytemuck::cast_slice(vertices));

        self.ui_vertex_count = vertices.len() as u32;
    }

    pub fn update_gizmos(&mut self, vertices: &[RendererGizmoVertex]) {
        if vertices.is_empty() {
            self.gizmo_vertex_count = 0;
            return;
        }

        if vertices.len() > self.gizmo_vertex_capacity {
            self.gizmo_vertex_capacity = vertices.len().next_power_of_two();
            self.gizmo_vertex_buffer = self
                .device
                .create_buffer(&wgpu::BufferDescriptor { label: Some("gizmo vertices"), size: (self.gizmo_vertex_capacity * mem::size_of::<RendererGizmoVertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        self.queue.write_buffer(&self.gizmo_vertex_buffer, 0, bytemuck::cast_slice(vertices));

        self.gizmo_vertex_count = vertices.len() as u32;
    }

    pub fn set_gi_radiance_frozen(&mut self, frozen: bool) {
        self.gi_radiance_frozen = frozen;
    }

    pub fn set_gi_radiance_temporal_accumulation_enabled(&mut self, enabled: bool) {
        self.gi_radiance_temporal_accumulation_enabled = enabled;
    }

    pub fn set_gi_radiance_probe_relocation_enabled(&mut self, enabled: bool) {
        if self.gi_radiance_probe_relocation_enabled == enabled {
            return;
        }
        self.gi_radiance_probe_relocation_enabled = enabled;

        self.radiance_field.invalidate();
    }

    pub fn set_radiance_angular_resolution(&mut self, resolution: RadianceAngularResolution) {
        if self.radiance_angular_resolution == resolution {
            return;
        }
        self.radiance_angular_resolution = resolution;
        self.radiance_angular_segments = [RendererRadianceAngularSegment::default(); RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS];
        self.radiance_angular_segments_cell = [u32::MAX; 3];
        self.gi_trace_diagnostics = [RendererGiRayDiagnostic::default(); RADIANCE_FIELD_RAY_COUNT];
        self.reset_radiance_field_coverage_stats();
        self.queue.write_buffer(
            &self._radiance_field_buffer,
            0,
            bytemuck::cast_slice(&vec![RadianceFieldAngularSample::zeroed(); self.radiance_field.cell_count() * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS]),
        );
        self.queue.write_buffer(
            &self._radiance_field_bounce_buffer,
            0,
            bytemuck::cast_slice(&vec![[0.0_f32; 4]; self.radiance_field.cell_count() * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS]),
        );
        self.queue.write_buffer(&self._radiance_field_coverage_buffer, 0, bytemuck::cast_slice(&vec![[0_u32; 4]; self.radiance_field.cell_count()]));

        self.radiance_field.invalidate();
    }

    pub fn radiance_angular_resolution(&self) -> RadianceAngularResolution {
        self.radiance_angular_resolution
    }

    pub fn radiance_angular_segments(&self) -> [RendererRadianceAngularSegment; RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS] {
        self.radiance_angular_segments
    }

    pub fn radiance_angular_segments_cell(&self) -> [u32; 3] {
        self.radiance_angular_segments_cell
    }

    pub fn set_world_radiance_gizmos_enabled(&mut self, enabled: bool) {
        self.world_radiance_gizmos_enabled = enabled;
    }

    pub fn set_world_radiance_trace_gizmos_enabled(&mut self, enabled: bool) {
        self.world_radiance_trace_gizmos_enabled = enabled;
    }

    pub fn set_selected_radiance_cell(&mut self, cell: [u32; 3]) {
        let resolution = self.radiance_field.resolution;
        let selected_cell = [cell[0].min(resolution[0] - 1), cell[1].min(resolution[1] - 1), cell[2].min(resolution[2] - 1)];

        if selected_cell == self.selected_radiance_cell {
            return;
        }
        self.selected_radiance_cell = selected_cell;
        let selection = [self.selected_radiance_cell[0], self.selected_radiance_cell[1], self.selected_radiance_cell[2], 0];

        self.queue.write_buffer(&self.gi_gizmo_selection_buffer, 0, bytemuck::cast_slice(&selection));
    }

    pub fn set_selected_radiance_trace_options(&mut self, selected_ray: u32, mode: RendererGiTraceDebugMode) {
        let options = [selected_ray.min(RADIANCE_FIELD_RAY_COUNT as u32 - 1), mode as u32, 0, 0];

        self.queue.write_buffer(&self.gi_gizmo_selection_buffer, 16, bytemuck::cast_slice(&options));
    }

    pub fn gi_trace_diagnostics(&self) -> [RendererGiRayDiagnostic; RADIANCE_FIELD_RAY_COUNT] {
        self.gi_trace_diagnostics
    }

    pub fn request_radiance_surface_inspection(&mut self, pixel: [u32; 2]) {
        self.radiance_surface_inspection_request = Some([pixel[0].min(self.config.width.saturating_sub(1)), pixel[1].min(self.config.height.saturating_sub(1))]);

        self.radiance_surface_inspection = RendererRadianceSurfaceInspection::default();
    }

    pub fn radiance_surface_inspection(&self) -> RendererRadianceSurfaceInspection {
        self.radiance_surface_inspection
    }

    pub fn radiance_surface_inspection_pending(&self) -> bool {
        self.radiance_surface_inspection_request.is_some() || self.radiance_surface_inspection_readback_receiver.is_some()
    }

    fn copy_selected_gi_trace_debug(&self, encoder: &mut wgpu::CommandEncoder) {
        let field = &self.radiance_field;
        let coordinate = self.selected_radiance_cell;
        let cell_index = coordinate[0] + coordinate[1] * field.resolution[0] + coordinate[2] * field.resolution[0] * field.resolution[1];
        let copy_size = RADIANCE_FIELD_TRACE_DEBUG_RECORD_SIZE * RADIANCE_FIELD_RAY_COUNT as u64;
        let trace_offset = cell_index as u64 * RADIANCE_FIELD_RAY_COUNT as u64 * RADIANCE_FIELD_TRACE_DEBUG_RECORD_SIZE;
        encoder.copy_buffer_to_buffer(&self._radiance_field_trace_debug_buffer, trace_offset, &self.gi_trace_readback_buffer, 0, copy_size);
        let region_offset = cell_index as u64 * RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS as u64 * mem::size_of::<RadianceFieldAngularSample>() as u64;
        let region_size = RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS as u64 * mem::size_of::<RadianceFieldAngularSample>() as u64;

        encoder.copy_buffer_to_buffer(&self._radiance_field_buffer, region_offset, &self.gi_trace_readback_buffer, copy_size, region_size);
    }

    fn poll_gi_trace_readback(&mut self) -> Result<(), String> {
        if self.gi_trace_readback_receiver.is_none() {
            return Ok(());
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let result = match self.gi_trace_readback_receiver.as_ref().unwrap().try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.gi_trace_readback_receiver = None;
                self.gi_trace_diagnostics = [RendererGiRayDiagnostic::default(); RADIANCE_FIELD_RAY_COUNT];
                return Ok(());
            }
        };
        self.gi_trace_readback_receiver = None;
        if result.is_err() {
            self.gi_trace_diagnostics = [RendererGiRayDiagnostic::default(); RADIANCE_FIELD_RAY_COUNT];
            return Ok(());
        }

        if self.gi_trace_readback_angular_resolution != self.radiance_angular_resolution {
            let mapped = self.gi_trace_readback_buffer.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
            drop(mapped);
            self.gi_trace_readback_buffer.unmap();
            self.gi_trace_diagnostics = [RendererGiRayDiagnostic::default(); RADIANCE_FIELD_RAY_COUNT];
            self.radiance_angular_segments_cell = [u32::MAX; 3];
            return Ok(());
        }
        let slice = self.gi_trace_readback_buffer.slice(..);
        let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
        let debug_size = (RADIANCE_FIELD_TRACE_DEBUG_RECORD_SIZE * RADIANCE_FIELD_RAY_COUNT as u64) as usize;
        let records = bytemuck::cast_slice::<u8, [[f32; 4]; RADIANCE_FIELD_TRACE_DEBUG_VEC4_COUNT]>(&mapped[..debug_size]);
        let angular_samples = bytemuck::cast_slice::<u8, RadianceFieldAngularSample>(&mapped[debug_size..]);
        self.radiance_angular_segments_cell = self.gi_trace_readback_cell;
        let (bands, sectors) = self.radiance_angular_resolution.bands_and_sectors();
        for segment_index in 0..RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS {
            let sample = angular_samples[segment_index];
            let sampled = sample.sampling[0] > 0;
            let band = segment_index as u32 / sectors;
            let sector = segment_index as u32 % sectors;
            let height = 1.0 - 2.0 * (band as f32 + 0.5) / bands as f32;
            let azimuth = std::f32::consts::TAU * (sector as f32 + 0.5) / sectors as f32;
            let radius = (1.0 - height * height).max(0.0).sqrt();
            self.radiance_angular_segments[segment_index] = RendererRadianceAngularSegment {
                sampled,
                direction: [radius * azimuth.cos(), height, radius * azimuth.sin()],
                radiance: [sample.radiance[0], sample.radiance[1], sample.radiance[2]],
                sample_count: sample.sampling[0],
                attempt_count: sample.sampling[1],
                age_frames: if sampled { self.gi_trace_readback_frame.wrapping_sub(sample.sampling[2]) } else { u32::MAX },
            };
        }

        for (index, record) in records.iter().enumerate() {
            let region_index = (record[10][0].max(0.0).round() as usize).min(RADIANCE_FIELD_MAX_ANGULAR_SEGMENTS - 1);
            let stored_radiance = self.radiance_angular_segments[region_index].radiance;
            let radiance = [record[4][0], record[4][1], record[4][2]];
            let last_update_frame = record[7][3].max(0.0).round() as u32;
            self.gi_trace_diagnostics[index] = RendererGiRayDiagnostic {
                valid: record[0][3] > 0.5,
                relocation_data_valid: record[12][3] != 0.0,
                relocation_succeeded: record[12][3] > 0.0,
                selected_cell: self.gi_trace_readback_cell,
                termination_reason: record[3][3].round() as u32,
                travelled_distance: record[2][3],
                hit_position: [record[1][0], record[1][1], record[1][2]],
                hit_normal: [record[2][0], record[2][1], record[2][2]],
                original_cell_center: [record[12][0], record[12][1], record[12][2]],
                ray_origin: [record[0][0], record[0][1], record[0][2]],
                relocation_offset: [record[0][0] - record[12][0], record[0][1] - record[12][1], record[0][2] - record[12][2]],
                radiance,
                direct_radiance: [record[5][0], record[5][1], record[5][2]],
                emission_radiance: [record[6][0], record[6][1], record[6][2]],
                sky_radiance: [record[7][0], record[7][1], record[7][2]],
                stored_radiance,
                radiance_difference: [radiance[0] - stored_radiance[0], radiance[1] - stored_radiance[1], radiance[2] - stored_radiance[2]],
                last_update_age_frames: self.gi_trace_readback_frame.wrapping_sub(last_update_frame),
                ray_direction: [record[3][0], record[3][1], record[3][2]],
                accumulated_sample_count: record[10][1].max(0.0).round() as u32,
                attempted_sample_count: record[10][2].max(0.0).round() as u32,
                blend_factor: record[4][3].clamp(0.0, 1.0),
                angular_segment_index: record[10][0].max(0.0).round() as u32,
                segment_sample_count: record[10][1].max(0.0).round() as u32,
                segment_attempt_count: record[10][2].max(0.0).round() as u32,
            };
        }
        drop(mapped);
        self.gi_trace_readback_buffer.unmap();

        Ok(())
    }

    fn poll_radiance_field_coverage_readback(&mut self) -> Result<(), String> {
        if self.radiance_field_coverage_readback_receiver.is_none() {
            return Ok(());
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let result = match self.radiance_field_coverage_readback_receiver.as_ref().unwrap().try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.radiance_field_coverage_readback_receiver = None;
                return Ok(());
            }
        };
        self.radiance_field_coverage_readback_receiver = None;
        if result.is_err() {
            return Ok(());
        }
        let mapped = self.radiance_field_coverage_readback_buffer.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
        let summaries = bytemuck::cast_slice::<u8, [u32; 4]>(&mapped);
        if self.sdf_gi_enabled && self.render_settings.irradiance_enabled {
            let [covered_once, covered_twice, covered_cells, attempted_regions, valid_cells] = radiance_field_coverage_percentages(summaries, self.radiance_angular_resolution.segment_count());
            self.performance_stats.gi_region_coverage_percent = covered_once;
            self.performance_stats.gi_regions_covered_twice_percent = covered_twice;
            self.performance_stats.gi_cells_with_coverage_percent = covered_cells;
            self.performance_stats.gi_regions_attempted_percent = attempted_regions;
            self.performance_stats.gi_valid_cells_percent = valid_cells;
            self.performance_stats.gi_coverage_available = true;
        }
        drop(mapped);
        self.radiance_field_coverage_readback_buffer.unmap();

        Ok(())
    }

    fn poll_radiance_surface_inspection(&mut self) -> Result<(), String> {
        if self.radiance_surface_inspection_readback_receiver.is_none() {
            return Ok(());
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let result = match self.radiance_surface_inspection_readback_receiver.as_ref().unwrap().try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.radiance_surface_inspection_readback_receiver = None;
                self.radiance_surface_inspection_mesh_snapshot = None;
                self.radiance_surface_inspection = RendererRadianceSurfaceInspection::default();
                return Ok(());
            }
        };
        self.radiance_surface_inspection_readback_receiver = None;
        if result.is_err() {
            self.radiance_surface_inspection_mesh_snapshot = None;
            self.radiance_surface_inspection = RendererRadianceSurfaceInspection::default();
            return Ok(());
        }
        let mapped = self.radiance_surface_inspection_readback_buffer.slice(..).get_mapped_range().map_err(|error| error.to_string())?;
        let values = bytemuck::cast_slice::<u8, [f32; 4]>(&mapped);
        let mut inspection = RendererRadianceSurfaceInspection {
            valid: values[0][3] > 0.5,
            inside_field: values[1][3] > 0.5,
            world_position: [values[0][0], values[0][1], values[0][2]],
            normal: [values[1][0], values[1][1], values[1][2]],
            sdf_start_voxel: [values[2][0].round() as i32, values[2][1].round() as i32, values[2][2].round() as i32],
            sdf_start_in_bounds: values[2][3] > 0.5,
            sdf_start_occupied: values[3][3] > 0.5,
            sdf_origin_offset: [values[3][0], values[3][1], values[3][2]],
            ..RendererRadianceSurfaceInspection::default()
        };
        for candidate_index in 0..RADIANCE_SURFACE_CANDIDATE_COUNT {
            let value_offset = 4 + candidate_index * 9;
            let grid_position = values[value_offset];
            let probe_position = values[value_offset + 1];
            let directional_radiance = values[value_offset + 2];
            let contribution = values[value_offset + 3];
            let side_weight = values[value_offset + 4];
            let first_blocking_voxel = values[value_offset + 5];
            let first_occupied_after_start = values[value_offset + 6];
            let first_occupied_after_start_minimum = values[value_offset + 7];
            let first_occupied_after_start_maximum = values[value_offset + 8];
            inspection.candidates[candidate_index] = RendererRadianceSurfaceCandidate {
                grid_position: [grid_position[0].round().max(0.0) as u32, grid_position[1].round().max(0.0) as u32, grid_position[2].round().max(0.0) as u32],
                probe_position: [probe_position[0], probe_position[1], probe_position[2]],
                probe_valid: grid_position[3] > 0.0,
                spatial_weight: probe_position[3],
                side_weight: side_weight[0],
                directional_radiance: [directional_radiance[0], directional_radiance[1], directional_radiance[2]],
                directional_support: directional_radiance[3],
                contribution_radiance: [contribution[0], contribution[1], contribution[2]],
                final_weight: contribution[3],
                first_blocking_voxel: [first_blocking_voxel[0].round() as i32, first_blocking_voxel[1].round() as i32, first_blocking_voxel[2].round() as i32],
                has_first_blocking_voxel: first_blocking_voxel[3] > 0.5,
                first_occupied_after_start: [first_occupied_after_start[0].round() as i32, first_occupied_after_start[1].round() as i32, first_occupied_after_start[2].round() as i32],
                has_occupied_voxel_after_start: first_occupied_after_start[3] > 0.5,
                first_occupied_after_start_minimum: [first_occupied_after_start_minimum[0], first_occupied_after_start_minimum[1], first_occupied_after_start_minimum[2]],
                first_occupied_after_start_maximum: [first_occupied_after_start_maximum[0], first_occupied_after_start_maximum[1], first_occupied_after_start_maximum[2]],
                ..RendererRadianceSurfaceCandidate::default()
            };
        }
        drop(mapped);
        self.radiance_surface_inspection_readback_buffer.unmap();
        let mesh_snapshot = self.radiance_surface_inspection_mesh_snapshot.take().unwrap_or_default();
        populate_surface_triangle_visibility(&mut inspection, &mesh_snapshot);
        self.radiance_surface_inspection = inspection;

        Ok(())
    }

    pub fn set_sdf_occupancy_gizmos_enabled(&mut self, enabled: bool) {
        self.sdf_occupancy_gizmos_enabled = enabled;
    }

    pub fn capture(&mut self, camera: &Camera, width: u32, height: u32) -> Result<CapturedFrame, String> {
        self.capture_with_debug_mode(camera, width, height, self.render_debug_mode)
    }

    pub fn capture_current_with_debug_mode(&mut self, camera: &Camera, debug_mode: RenderDebugMode) -> Result<CapturedFrame, String> {
        self.capture_with_debug_mode(camera, self.config.width, self.config.height, debug_mode)
    }

    pub fn capture_current(&mut self, camera: &Camera) -> Result<CapturedFrame, String> {
        self.capture(camera, self.config.width, self.config.height)
    }

    pub fn capture_with_debug_mode(&mut self, camera: &Camera, width: u32, height: u32, debug_mode: RenderDebugMode) -> Result<CapturedFrame, String> {
        self.capture_inner(camera, width, height, debug_mode, true)
    }

    pub fn capture_current_read_only(&mut self, camera: &Camera) -> Result<CapturedFrame, String> {
        self.capture_read_only_with_debug_mode(camera, self.config.width, self.config.height, self.render_debug_mode)
    }

    pub fn capture_read_only(&mut self, camera: &Camera, width: u32, height: u32) -> Result<CapturedFrame, String> {
        self.capture_read_only_with_debug_mode(camera, width, height, self.render_debug_mode)
    }

    pub fn capture_current_read_only_with_debug_mode(&mut self, camera: &Camera, debug_mode: RenderDebugMode) -> Result<CapturedFrame, String> {
        self.capture_read_only_with_debug_mode(camera, self.config.width, self.config.height, debug_mode)
    }

    pub fn capture_read_only_with_debug_mode(&mut self, camera: &Camera, width: u32, height: u32, debug_mode: RenderDebugMode) -> Result<CapturedFrame, String> {
        self.capture_inner(camera, width, height, debug_mode, false)
    }

    fn capture_inner(&mut self, camera: &Camera, width: u32, height: u32, debug_mode: RenderDebugMode, update_gi: bool) -> Result<CapturedFrame, String> {
        let width = width.max(1);
        let height = height.max(1);
        let format = self.config.format;

        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(format!("capture does not support surface format {format:?}"));
        }

        let color_texture =
            self.device
                .create_texture(&wgpu::TextureDescriptor { label: Some("capture color target"), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (depth_texture, depth_view) = create_depth_texture(&self.device, width, height);
        let (depth_sample_texture, depth_sample_view) = create_depth_sample_texture(&self.device, width, height);
        let (surface_normal_texture, surface_normal_view) = create_surface_normal_texture(&self.device, width, height);
        let (_surface_id_texture, surface_id_view) = create_surface_id_texture(&self.device, width, height);
        let capture_depth_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("capture depth bind group"),
            layout: &self.depth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth_sample_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self._depth_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&surface_normal_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&surface_id_view) },
                wgpu::BindGroupEntry { binding: 4, resource: self.triangle_bvh_node_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: self.triangle_bvh_triangle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.triangle_bvh_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: self.triangle_bvh_diagnostics_buffer.as_entire_binding() },
            ],
        });
        let capture_uniform = camera_uniform(width, height, camera, &self.lights, debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&capture_uniform));
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));
        let bytes_per_pixel = 4_u32;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_size = u64::from(padded_bytes_per_row) * u64::from(height);
        let readback = self
            .device
            .create_buffer(&wgpu::BufferDescriptor { label: Some("capture readback"), size: readback_size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let normal_unpadded_bytes_per_row = width * 8;
        let normal_padded_bytes_per_row = normal_unpadded_bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let normal_readback = self
            .device
            .create_buffer(&wgpu::BufferDescriptor { label: Some("capture surface normal readback"), size: u64::from(normal_padded_bytes_per_row) * u64::from(height), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let scene_depth_readback = create_depth_readback_buffer(&self.device, "capture scene depth", width, height);
        let has_directional_light = self.lights.iter().any(|light| matches!(light, Light::Directional(_)));
        let has_spot_light = self.lights.iter().any(|light| matches!(light, Light::Spot(_)));
        let point_light_count = self.lights.iter().filter(|light| matches!(light, Light::Point(_))).count().min(MAX_POINT_SHADOWS);
        let directional_depth_readbacks = has_directional_light.then(|| {
            (0..SHADOW_CASCADE_COUNT)
                .map(|cascade| create_depth_readback_buffer(&self.device, &format!("capture directional cascade {cascade}"), SHADOW_MAP_SIZE, SHADOW_MAP_SIZE))
                .collect::<Vec<_>>()
        });
        let spot_size = self.render_settings.shadow_budget.atlas_size.max(SPOT_SHADOW_MAP_SIZE);
        let spot_depth_readback = has_spot_light.then(|| create_depth_readback_buffer(&self.device, "capture spot shadow atlas", spot_size, spot_size));
        let point_depth_readbacks = (point_light_count > 0).then(|| {
            (0..(point_light_count * POINT_SHADOW_FACE_COUNT as usize))
                .map(|face| create_depth_readback_buffer(&self.device, &format!("capture point shadow face {face}"), SPOT_SHADOW_MAP_SIZE, SPOT_SHADOW_MAP_SIZE))
                .collect::<Vec<_>>()
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture encoder") });
        encoder.push_debug_group("GPU capture shadow passes");
        self.render_shadow_pass(&mut encoder);
        self.render_spot_shadow_pass(&mut encoder);
        self.render_point_shadow_pass(&mut encoder);
        encoder.pop_debug_group();
        encoder.push_debug_group("GPU capture scene pass");
        self.render_depth_prepass(&mut encoder, &depth_view, &surface_normal_view, &surface_id_view);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &depth_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &depth_sample_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        if update_gi && self.sdf_gi_enabled && self.render_settings.irradiance_enabled {
            self.render_world_radiance_field_pass(&mut encoder, &capture_depth_bind_group);
        }
        self.render_scene_pass(&mut encoder, &color_view, &depth_view, &capture_depth_bind_group, debug_mode);
        encoder.pop_debug_group();
        if self.gizmo_vertex_count > 0 {
            self.render_gizmo_pass(&mut encoder, &color_view, &depth_view);
        }

        if self.should_draw_gi_gizmos() {
            self.render_gi_gizmo_pass(&mut encoder, &color_view, &depth_view);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &color_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_bytes_per_row), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &depth_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::DepthOnly },
            wgpu::TexelCopyBufferInfo { buffer: &scene_depth_readback, layout: depth_copy_layout(width, height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &surface_normal_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &normal_readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(normal_padded_bytes_per_row), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        if let Some(readbacks) = &directional_depth_readbacks {
            for (cascade, readback) in readbacks.iter().enumerate() {
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo { texture: &self._shadow_texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: cascade as u32 }, aspect: wgpu::TextureAspect::DepthOnly },
                    wgpu::TexelCopyBufferInfo { buffer: readback, layout: depth_copy_layout(SHADOW_MAP_SIZE, SHADOW_MAP_SIZE) },
                    wgpu::Extent3d { width: SHADOW_MAP_SIZE, height: SHADOW_MAP_SIZE, depth_or_array_layers: 1 },
                );
            }
        }

        if let Some(readback) = &spot_depth_readback {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &self._spot_shadow_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::DepthOnly },
                wgpu::TexelCopyBufferInfo { buffer: readback, layout: depth_copy_layout(spot_size, spot_size) },
                wgpu::Extent3d { width: spot_size, height: spot_size, depth_or_array_layers: 1 },
            );
        }

        if let Some(readbacks) = &point_depth_readbacks {
            for (face, readback) in readbacks.iter().enumerate() {
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo { texture: &self._point_shadow_texture, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: face as u32 }, aspect: wgpu::TextureAspect::DepthOnly },
                    wgpu::TexelCopyBufferInfo { buffer: readback, layout: depth_copy_layout(SPOT_SHADOW_MAP_SIZE, SPOT_SHADOW_MAP_SIZE) },
                    wgpu::Extent3d { width: SPOT_SHADOW_MAP_SIZE, height: SPOT_SHADOW_MAP_SIZE, depth_or_array_layers: 1 },
                );
            }
        }
        self.queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
        let mut rgba8 = Vec::with_capacity((width * height * bytes_per_pixel) as usize);
        for row in mapped.chunks_exact(padded_bytes_per_row as usize).take(height as usize) {
            for pixel in row.as_chunks::<4>().0.iter().take(width as usize) {
                if matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
                    rgba8.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                } else {
                    rgba8.extend_from_slice(pixel);
                }
            }
        }
        drop(mapped);
        readback.unmap();

        let normal_slice = normal_readback.slice(..);
        let (normal_sender, normal_receiver) = mpsc::channel();
        normal_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = normal_sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
        normal_receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
        let normal_mapped = normal_slice.get_mapped_range().map_err(|error| error.to_string())?;
        let mut surface_normals = Vec::with_capacity((width * height) as usize);
        for row in normal_mapped.chunks_exact(normal_padded_bytes_per_row as usize).take(height as usize) {
            for pixel in row[..normal_unpadded_bytes_per_row as usize].as_chunks::<2>().0.chunks_exact(4) {
                let normal = Vec3::new(f16_to_f32(u16::from_le_bytes(pixel[0])), f16_to_f32(u16::from_le_bytes(pixel[1])), f16_to_f32(u16::from_le_bytes(pixel[2])));
                surface_normals.push([((normal.x * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8, ((normal.y * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8, ((normal.z * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8, 255]);
            }
        }
        drop(normal_mapped);
        normal_readback.unmap();

        let scene_depth = CapturedDepthBuffer { width, height, layers: vec![read_depth_buffer(&self.device, &scene_depth_readback, width, height)?] };
        let directional_shadow = CapturedDepthBuffer {
            width: SHADOW_MAP_SIZE,
            height: SHADOW_MAP_SIZE,
            layers: directional_depth_readbacks
                .as_ref()
                .map(|buffers| buffers.iter().map(|buffer| read_depth_buffer(&self.device, buffer, SHADOW_MAP_SIZE, SHADOW_MAP_SIZE)).collect::<Result<_, _>>())
                .transpose()?
                .unwrap_or_default(),
        };
        let spot_shadow = CapturedDepthBuffer { width: spot_size, height: spot_size, layers: spot_depth_readback.as_ref().map(|buffer| read_depth_buffer(&self.device, buffer, spot_size, spot_size)).transpose()?.into_iter().collect() };
        let point_shadow = CapturedDepthBuffer {
            width: SPOT_SHADOW_MAP_SIZE,
            height: SPOT_SHADOW_MAP_SIZE,
            layers: point_depth_readbacks
                .as_ref()
                .map(|buffers| buffers.iter().map(|buffer| read_depth_buffer(&self.device, buffer, SPOT_SHADOW_MAP_SIZE, SPOT_SHADOW_MAP_SIZE)).collect::<Result<_, _>>())
                .transpose()?
                .unwrap_or_default(),
        };

        let current_camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&current_camera_uniform));
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));

        Ok(CapturedFrame { width, height, rgba8, surface_normals, scene_depth, directional_shadow, spot_shadow, point_shadow })
    }

    fn update_sdf_gpu(&mut self, update: &SdfClipmapUpdate) {
        let resolution = self.sdf_clipmap.resolution;
        let texture_voxel_count = update.texture_regions.iter().map(|region| region.size.iter().product::<u32>()).sum::<u32>();
        let clearance_voxel_count = update.clearance_regions.iter().map(|region| region.size.iter().product::<u32>()).sum::<u32>();

        if texture_voxel_count == 0 && clearance_voxel_count == 0 {
            return;
        }
        let dirty_regions = update
            .texture_regions
            .iter()
            .chain(&update.clearance_regions)
            .map(|region| SdfGpuUpdateRegion { origin: [region.origin[0], region.origin[1], region.origin[2], 0], size: [region.size[0], region.size[1], region.size[2], 0] })
            .collect::<Vec<_>>();
        let dirty_buffer_size = (dirty_regions.len() * mem::size_of::<SdfGpuUpdateRegion>()) as u64;
        if dirty_buffer_size > self.sdf_update_dirty_capacity {
            self.sdf_update_dirty_capacity = dirty_buffer_size.next_power_of_two();
            self.sdf_update_dirty_buffer = Some(
                self.device
                    .create_buffer(&wgpu::BufferDescriptor { label: Some("incremental SDF dirty regions"), size: self.sdf_update_dirty_capacity, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            );
        }
        let dirty_buffer = self.sdf_update_dirty_buffer.as_ref().expect("dirty region buffer is allocated for a non-empty update");
        self.queue.write_buffer(dirty_buffer, 0, bytemuck::cast_slice(&dirty_regions));
        let settings = SdfGpuUpdateSettings {
            minimum: self.sdf_clipmap.minimum.extend(0.0).to_array(),
            voxel_size: self.sdf_clipmap.voxel_size.extend(0.0).to_array(),
            counts: [resolution[0], resolution[1], resolution[2], self.triangle_bvh_nodes.len() as u32],
            update: [texture_voxel_count, clearance_voxel_count, update.texture_regions.len() as u32, self.triangle_bvh_triangles.len() as u32],
            region_counts: [update.clearance_regions.len() as u32, 0, 0, 0],
        };
        self.queue.write_buffer(&self.sdf_update_settings_buffer, 0, bytemuck::bytes_of(&settings));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("incremental SDF update resources"),
            layout: &self.sdf_update_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.sdf_update_settings_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.triangle_bvh_node_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.triangle_bvh_triangle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: dirty_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.sdf_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.sdf_emission_view) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.sdf_clearance_sdf_input_view) },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&self.sdf_clearance_work_view) },
            ],
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("incremental SDF GPU update") });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_clearance_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_clearance_work_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: resolution[0], height: resolution[1], depth_or_array_layers: resolution[2] },
        );
        let clearance_count = settings.update[1];
        if texture_voxel_count > 0 {
            let timestamp_writes = self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::ComputePassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(12), end_of_pass_write_index: if clearance_count == 0 { Some(13) } else { None } });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("incremental SDF voxel and emission update"), timestamp_writes });
            pass.set_pipeline(&self.sdf_update_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(texture_voxel_count.div_ceil(64), 1, 1);
        }

        if clearance_count > 0 {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo { texture: &self._sdf_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyTextureInfo { texture: &self._sdf_clearance_sdf_input_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::Extent3d { width: resolution[0], height: resolution[1], depth_or_array_layers: resolution[2] },
            );
            let timestamp_writes = self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::ComputePassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: if texture_voxel_count == 0 { Some(12) } else { None }, end_of_pass_write_index: Some(13) });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("incremental SDF clearance update"), timestamp_writes });
            pass.set_pipeline(&self.sdf_clearance_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(clearance_count.div_ceil(64), 1, 1);
        }
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_clearance_work_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &self._sdf_clearance_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: resolution[0], height: resolution[1], depth_or_array_layers: resolution[2] },
        );

        self.queue.submit(Some(encoder.finish()));
    }

    pub fn render(&mut self) -> Result<(), String> {
        self.poll_gi_trace_readback()?;
        self.poll_radiance_field_coverage_readback()?;
        self.poll_radiance_surface_inspection()?;
        self.update_angular_segment_vertices();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                let surface = self.instance.create_surface(self.window.clone()).map_err(|error| error.to_string())?;
                surface.configure(&self.device, &self.config);
                self.surface = surface;
                self.window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface validation failed".into());
            }
        };

        let color_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        if self.index_count == 0 {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("empty scene encoder") });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("empty scene clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: f64::from(self.camera.clear_color.red), g: f64::from(self.camera.clear_color.green), b: f64::from(self.camera.clear_color.blue), a: f64::from(self.camera.clear_color.alpha) }), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }

            if self.ui_vertex_count > 0 {
                self.render_ui_pass(&mut encoder, &color_view);
            }
            self.queue.submit(Some(encoder.finish()));
            self.queue.present(frame);
            if !self.has_presented {
                log::debug!("First frame presented");
                self.has_presented = true;
            }
            return Ok(());
        }
        self.frame_index = self.frame_index.wrapping_add(1);
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));
        let trace_readback_copied = self.sdf_gi_enabled && self.render_settings.irradiance_enabled && self.gi_trace_readback_receiver.is_none();
        let coverage_readback_copied = self.sdf_gi_enabled && self.render_settings.irradiance_enabled && self.radiance_field_coverage_readback_receiver.is_none() && self.frame_index % RADIANCE_FIELD_COVERAGE_READBACK_INTERVAL == 0;
        let surface_inspection_copied = self.sdf_gi_enabled && self.render_settings.irradiance_enabled && self.radiance_surface_inspection_request.is_some() && self.radiance_surface_inspection_readback_receiver.is_none();
        if surface_inspection_copied {
            self.radiance_surface_inspection_mesh_snapshot = Some(self.surface_inspection_meshes.clone());
            let pixel = self.radiance_surface_inspection_request.unwrap_or_default();
            self.queue.write_buffer(
                &self.radiance_surface_inspection_settings_buffer,
                0,
                bytemuck::bytes_of(&RadianceSurfaceInspectionSettings { pixel: [pixel[0], pixel[1], 0, 0] }),
            );
        }

        if self.triangle_bvh_diagnostics_enabled {
            self.queue.write_buffer(&self.triangle_bvh_diagnostics_buffer, 0, bytemuck::bytes_of(&TriangleBvhCounterBuffer::zeroed()));
        }
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene encoder") });
        encoder.push_debug_group("GPU shadow passes");
        self.render_shadow_pass(&mut encoder);
        self.render_spot_shadow_pass(&mut encoder);
        self.render_point_shadow_pass(&mut encoder);
        encoder.pop_debug_group();
        encoder.push_debug_group("GPU scene pass");
        self.render_depth_prepass(&mut encoder, &self.depth_view, &self.surface_normal_view, &self.surface_id_view);
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &self._depth_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &self._depth_sample_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
        );
        if self.sdf_gi_enabled && self.render_settings.irradiance_enabled {
            self.render_world_radiance_field_pass(&mut encoder, &self.depth_bind_group);
            if trace_readback_copied {
                self.copy_selected_gi_trace_debug(&mut encoder);
            }

            if coverage_readback_copied {
                encoder.copy_buffer_to_buffer(
                    &self._radiance_field_coverage_buffer,
                    0,
                    &self.radiance_field_coverage_readback_buffer,
                    0,
                    (self.radiance_field.cell_count() * mem::size_of::<[u32; 4]>()) as u64,
                );
            }

            if surface_inspection_copied {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("GI surface contribution inspection"), timestamp_writes: None });
                pass.set_pipeline(&self.radiance_surface_inspection_pipeline);
                pass.set_bind_group(0, &self.radiance_field_bounce_bind_group, &[]);
                pass.set_bind_group(1, &self.depth_bind_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
                drop(pass);
                encoder.copy_buffer_to_buffer(&self.radiance_surface_inspection_values_buffer, 0, &self.radiance_surface_inspection_readback_buffer, 0, RADIANCE_SURFACE_INSPECTION_SIZE);
            }
        }
        self.render_scene_pass(&mut encoder, &color_view, &self.depth_view, &self.depth_bind_group, self.render_debug_mode);
        encoder.pop_debug_group();
        if self.gizmo_vertex_count > 0 {
            self.render_gizmo_pass(&mut encoder, &color_view, &self.depth_view);
        }

        if self.should_draw_gi_gizmos() {
            self.render_gi_gizmo_pass(&mut encoder, &color_view, &self.depth_view);
        }

        if self.ui_vertex_count > 0 {
            self.render_ui_pass(&mut encoder, &color_view);
        }
        self.resolve_gpu_timestamps(&mut encoder);
        self.queue.submit(Some(encoder.finish()));
        if trace_readback_copied {
            self.gi_trace_readback_cell = self.selected_radiance_cell;
            self.gi_trace_readback_frame = self.radiance_field.frame_index;
            self.gi_trace_readback_angular_resolution = self.radiance_angular_resolution;
            let (sender, receiver) = mpsc::channel();
            self.gi_trace_readback_buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            self.gi_trace_readback_receiver = Some(receiver);
        }

        if coverage_readback_copied {
            let (sender, receiver) = mpsc::channel();
            self.radiance_field_coverage_readback_buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            self.radiance_field_coverage_readback_receiver = Some(receiver);
        }

        if surface_inspection_copied {
            self.radiance_surface_inspection_request = None;
            let (sender, receiver) = mpsc::channel();
            self.radiance_surface_inspection_readback_buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            self.radiance_surface_inspection_readback_receiver = Some(receiver);
        }
        self.queue.present(frame);
        if !self.has_presented {
            log::debug!("First frame presented");
            self.has_presented = true;
        }

        Ok(())
    }

    fn resolve_gpu_timestamps(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(timestamps) = &self.gpu_timestamps else {
            return;
        };
        encoder.resolve_query_set(&timestamps.query_set, 0..GPU_TIMESTAMP_COUNT, &timestamps.resolve_buffer, 0);

        encoder.copy_buffer_to_buffer(&timestamps.resolve_buffer, 0, &timestamps.readback_buffer, 0, u64::from(GPU_TIMESTAMP_COUNT) * 8);
    }

    fn render_depth_prepass(&self, encoder: &mut wgpu::CommandEncoder, depth_view: &wgpu::TextureView, normal_view: &wgpu::TextureView, surface_id_view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene depth prepass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment { view: normal_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } }),
                Some(wgpu::RenderPassColorAttachment { view: surface_id_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.depth_prepass_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for batch in &self.mesh_draw_batches {
            let instance_start = (batch.instance_range.start * mem::size_of::<RendererInstanceData>()) as u64;
            let instance_end = (batch.instance_range.end * mem::size_of::<RendererInstanceData>()) as u64;
            pass.set_vertex_buffer(1, self.instance_buffer.slice(instance_start..instance_end));
            pass.draw_indexed(batch.index_range.clone(), 0, 0..batch.instance_range.len() as u32);
        }
    }

    fn render_scene_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView, depth_bind_group: &wgpu::BindGroup, debug_mode: RenderDebugMode) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: f64::from(self.camera.clear_color.red), g: f64::from(self.camera.clear_color.green), b: f64::from(self.camera.clear_color.blue), a: f64::from(self.camera.clear_color.alpha) }), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::RenderPassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(6), end_of_pass_write_index: Some(7) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, depth_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        if debug_mode == RenderDebugMode::Wireframe {
            pass.set_pipeline(&self.wireframe_pipeline);
            pass.set_index_buffer(self.wireframe_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in &self.mesh_draw_batches {
                let material_texture = self.material_texture_bindings.get(&batch.material_texture_ids).expect("material texture binding was not created");
                pass.set_bind_group(2, &material_texture.bind_group, &[]);
                let instance_start = (batch.instance_range.start * mem::size_of::<RendererInstanceData>()) as u64;
                let instance_end = (batch.instance_range.end * mem::size_of::<RendererInstanceData>()) as u64;
                pass.set_vertex_buffer(1, self.instance_buffer.slice(instance_start..instance_end));
                pass.draw_indexed(batch.wireframe_range.clone(), 0, 0..batch.instance_range.len() as u32);
            }
        } else {
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in &self.mesh_draw_batches {
                let material_texture = self.material_texture_bindings.get(&batch.material_texture_ids).expect("material texture binding was not created");
                pass.set_bind_group(2, &material_texture.bind_group, &[]);
                let instance_start = (batch.instance_range.start * mem::size_of::<RendererInstanceData>()) as u64;
                let instance_end = (batch.instance_range.end * mem::size_of::<RendererInstanceData>()) as u64;
                pass.set_vertex_buffer(1, self.instance_buffer.slice(instance_start..instance_end));
                pass.draw_indexed(batch.index_range.clone(), 0, 0..batch.instance_range.len() as u32);
            }
        }
    }

    fn render_world_radiance_field_pass(&self, encoder: &mut wgpu::CommandEncoder, depth_bind_group: &wgpu::BindGroup) {
        if self.gi_radiance_frozen {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("world radiance field update"),
            timestamp_writes: self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::ComputePassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(4), end_of_pass_write_index: None }),
        });
        pass.set_pipeline(&self.radiance_field_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, depth_bind_group, &[]);
        pass.dispatch_workgroups(RADIANCE_FIELD_DIRTY_CELL_BUDGET.div_ceil(4) as u32, 1, 1);
        drop(pass);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("world radiance field second bounce"),
            timestamp_writes: self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::ComputePassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(5) }),
        });
        pass.set_pipeline(&self.radiance_field_bounce_pipeline);
        pass.set_bind_group(0, &self.radiance_field_bounce_bind_group, &[]);
        pass.set_bind_group(1, depth_bind_group, &[]);
        pass.dispatch_workgroups(RADIANCE_FIELD_DIRTY_CELL_BUDGET.div_ceil(4) as u32, 1, 1);

        drop(pass);
    }

    fn render_shadow_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.render_settings.shadows_enabled || self.lights.is_empty() {
            return;
        }

        for (cascade_index, shadow_view) in self.shadow_views.iter().enumerate() {
            if cascade_index >= self.render_settings.shadow_cascade_count.clamp(1, SHADOW_CASCADE_COUNT as u32) as usize {
                break;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("directional shadow cascade"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: shadow_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: if cascade_index == 0 {
                    self.gpu_timestamps
                        .as_ref()
                        .map(|timestamps| wgpu::RenderPassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(2), end_of_pass_write_index: Some(3) })
                } else {
                    None
                },
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind_groups[cascade_index], &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in &self.mesh_draw_batches {
                let mut visible_start = None;
                for (batch_instance_index, instance_index) in batch.instance_indices.iter().enumerate() {
                    let visible = shadow_bounds_intersect(self.shadow_matrices[cascade_index], self.shadow_caster_bounds[*instance_index]);
                    match (visible_start, visible) {
                        (None, true) => visible_start = Some(batch_instance_index),
                        (Some(start), false) => {
                            draw_shadow_instance_range(&mut pass, &self.instance_buffer, batch, start, batch_instance_index);
                            visible_start = None;
                        }
                        _ => {}
                    }
                }

                if let Some(start) = visible_start {
                    draw_shadow_instance_range(&mut pass, &self.instance_buffer, batch, start, batch.instance_indices.len());
                }
            }
        }
    }

    fn render_spot_shadow_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.render_settings.shadows_enabled {
            return;
        }
        let mut rendered = 0u32;
        for request in self.shadow_scheduler.requests.clone().into_iter().filter(|request| request.resource_kind == ShadowResourceKind::SpotMap && request.dirty) {
            let Some(Light::Spot(light)) = self.lights.get(request.light_index as usize) else { continue };
            let Some(allocation) = self.shadow_resources.allocation_for(request.light_index as usize).copied() else { continue };
            self.queue.write_buffer(
                &self.spot_shadow_uniform_buffer,
                0,
                bytemuck::bytes_of(&SpotShadowUniform { view_projection: spot_shadow_view_projection(light).to_cols_array_2d() }),
            );
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spot shadow atlas pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.spot_shadow_view, depth_ops: Some(wgpu::Operations { load: if rendered == 0 { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: if rendered == 0 {
                    self.gpu_timestamps
                        .as_ref()
                        .map(|timestamps| wgpu::RenderPassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) })
                } else {
                    None
                },
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(allocation.atlas_offset[0] as f32, allocation.atlas_offset[1] as f32, allocation.resolution as f32, allocation.resolution as f32, 0.0, 1.0);
            pass.set_scissor_rect(allocation.atlas_offset[0], allocation.atlas_offset[1], allocation.resolution, allocation.resolution);
            pass.set_pipeline(&self.spot_shadow_pipeline);
            pass.set_bind_group(0, &self.spot_shadow_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for batch in &self.mesh_draw_batches {
                let instance_start = (batch.instance_range.start * mem::size_of::<RendererInstanceData>()) as u64;
                let instance_end = (batch.instance_range.end * mem::size_of::<RendererInstanceData>()) as u64;
                pass.set_vertex_buffer(1, self.instance_buffer.slice(instance_start..instance_end));
                pass.draw_indexed(batch.index_range.clone(), 0, 0..batch.instance_range.len() as u32);
            }
            drop(pass);
            self.shadow_scheduler.mark_clean(request.light_index as usize);
            rendered += 1;
        }

        if rendered > 0 {
            self.update_shadow_scheduler_stats();
        }
    }

    fn render_point_shadow_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.render_settings.shadows_enabled {
            return;
        }

        for request in self.shadow_scheduler.requests.clone().into_iter().filter(|request| request.resource_kind == ShadowResourceKind::PointCube && request.dirty) {
            let Some(Light::Point(light)) = self.lights.get(request.light_index as usize) else { continue };
            let Some(allocation) = self.shadow_resources.allocation_for(request.light_index as usize).copied() else { continue };
            let projections = point_shadow_view_projections(light.position, light.range);
            for (face, projection) in projections.iter().enumerate() {
                let face_layer = allocation.point_face_layer(face as u32).unwrap_or(u32::MAX) as usize;
                let Some(view) = self.point_shadow_views.get(face_layer) else { continue };
                let Some(uniform_buffer) = self.point_shadow_uniform_buffers.get(face_layer) else { continue };
                let Some(bind_group) = self.point_shadow_bind_groups.get(face_layer) else { continue };
                self.queue.write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&SpotShadowUniform { view_projection: projection.to_cols_array_2d() }));
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("point shadow face pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.spot_shadow_pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for batch in &self.mesh_draw_batches {
                    let instance_start = (batch.instance_range.start * mem::size_of::<RendererInstanceData>()) as u64;
                    let instance_end = (batch.instance_range.end * mem::size_of::<RendererInstanceData>()) as u64;
                    pass.set_vertex_buffer(1, self.instance_buffer.slice(instance_start..instance_end));
                    pass.draw_indexed(batch.index_range.clone(), 0, 0..batch.instance_range.len() as u32);
                }
            }
            self.shadow_scheduler.mark_clean(request.light_index as usize);
        }

        self.update_shadow_scheduler_stats();
    }

    fn render_ui_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::RenderPassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(10), end_of_pass_write_index: Some(11) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.ui_pipeline);
        pass.set_vertex_buffer(0, self.ui_vertex_buffer.slice(..));

        pass.draw(0..self.ui_vertex_count, 0..1);
    }

    fn render_gizmo_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gizmo pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: self
                .gpu_timestamps
                .as_ref()
                .map(|timestamps| wgpu::RenderPassTimestampWrites { query_set: &timestamps.query_set, beginning_of_pass_write_index: Some(8), end_of_pass_write_index: Some(9) }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.gizmo_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.gizmo_vertex_buffer.slice(..));

        pass.draw(0..self.gizmo_vertex_count, 0..1);
    }

    fn render_gi_gizmo_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("GI cache gizmo pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: color_view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.gi_gizmo_pipeline);
        pass.set_bind_group(0, &self.gi_gizmo_bind_group, &[]);
        let radiance_field_count = self.radiance_field.cell_count() as u32;

        if self.world_radiance_gizmos_enabled {
            pass.draw(0..8, 0..radiance_field_count);
        }

        if self.world_radiance_trace_gizmos_enabled {
            let trace_start = radiance_field_count;
            pass.draw(0..42, trace_start..trace_start + RADIANCE_FIELD_RAY_COUNT as u32);
            pass.draw(0..2, trace_start + RADIANCE_FIELD_RAY_COUNT as u32..trace_start + RADIANCE_FIELD_RAY_COUNT as u32 + 12);
            if self.angular_segment_vertex_count > 0 {
                pass.set_pipeline(&self.angular_segment_pipeline);
                pass.set_vertex_buffer(0, self.angular_segment_vertex_buffer.slice(..));
                pass.draw(0..self.angular_segment_vertex_count, 0..1);
            }
        }

        if self.sdf_occupancy_gizmos_enabled {
            pass.set_pipeline(&self.sdf_occupancy_gizmo_pipeline);
            pass.draw(0..36, 0..self.sdf_clipmap.resolution.iter().product());
        }
    }

    fn should_draw_gi_gizmos(&self) -> bool {
        let has_debug_gizmos = self.world_radiance_gizmos_enabled || self.world_radiance_trace_gizmos_enabled || self.sdf_occupancy_gizmos_enabled;

        has_debug_gizmos && self.sdf_gi_enabled && self.render_settings.irradiance_enabled
    }

    fn update_angular_segment_vertices(&mut self) {
        if !self.world_radiance_trace_gizmos_enabled || self.radiance_angular_segments_cell != self.selected_radiance_cell {
            self.angular_segment_vertex_count = 0;
            return;
        }
        let cell_size = self.radiance_field.cell_size;
        let coordinate = Vec3::new(self.selected_radiance_cell[0] as f32, self.selected_radiance_cell[1] as f32, self.selected_radiance_cell[2] as f32);
        let center = self.radiance_field.minimum + (coordinate + Vec3::splat(0.5)) * cell_size;
        let radius = (cell_size.min_element() * 0.28).clamp(0.12, 0.7);
        let (bands, sectors) = self.radiance_angular_resolution.bands_and_sectors();
        let segment_count = self.radiance_angular_resolution.segment_count() as usize;
        let mut vertices = Vec::with_capacity(segment_count * 4 * 4 * 6);
        for segment_index in 0..segment_count {
            let segment = self.radiance_angular_segments[segment_index];
            let band = segment_index as u32 / sectors;
            let sector = segment_index as u32 % sectors;
            let color = angular_segment_heat_color(segment);
            for band_step in 0..4 {
                for sector_step in 0..4 {
                    let u0 = band as f32 + band_step as f32 / 4.0;
                    let u1 = band as f32 + (band_step + 1) as f32 / 4.0;
                    let v0 = sector as f32 + sector_step as f32 / 4.0;
                    let v1 = sector as f32 + (sector_step + 1) as f32 / 4.0;
                    let p00 = angular_segment_sphere_position(center, radius, bands, sectors, u0, v0);
                    let p10 = angular_segment_sphere_position(center, radius, bands, sectors, u1, v0);
                    let p01 = angular_segment_sphere_position(center, radius, bands, sectors, u0, v1);
                    let p11 = angular_segment_sphere_position(center, radius, bands, sectors, u1, v1);
                    for position in [p00, p11, p10, p00, p01, p11] {
                        vertices.push(RendererGizmoVertex { position: position.to_array(), color });
                    }
                }
            }
        }

        if vertices.len() > self.angular_segment_vertex_capacity {
            self.angular_segment_vertex_capacity = vertices.len().next_power_of_two();
            self.angular_segment_vertex_buffer = self
                .device
                .create_buffer(&wgpu::BufferDescriptor { label: Some("radiance angular segment vertices"), size: (self.angular_segment_vertex_capacity * mem::size_of::<RendererGizmoVertex>()) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        }
        self.queue.write_buffer(&self.angular_segment_vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        self.angular_segment_vertex_count = vertices.len() as u32;
    }
}

fn angular_segment_sphere_position(center: Vec3, radius: f32, bands: u32, sectors: u32, band_position: f32, sector_position: f32) -> Vec3 {
    let height = 1.0 - 2.0 * band_position / bands as f32;
    let azimuth = std::f32::consts::TAU * sector_position / sectors as f32;
    let radial = (1.0 - height * height).max(0.0).sqrt();

    center + Vec3::new(radial * azimuth.cos(), height, radial * azimuth.sin()) * radius
}

fn angular_segment_heat_color(segment: RendererRadianceAngularSegment) -> [f32; 4] {
    if !segment.sampled {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let freshness = (-(segment.age_frames as f32) / 180.0).exp();

    [freshness, freshness, freshness, 1.0]
}

fn camera_uniform(width: u32, height: u32, camera: &Camera, lights: &[Light], mode: RenderDebugMode, settings: RenderSettings, _shadow_bounds: GeometryBounds) -> CameraUniform {
    let aspect = width as f32 / height as f32;
    let shadow_view_projections = lights
        .iter()
        .find_map(|light| match light {
            Light::Directional(light) => Some(cascade_shadow_view_projections(light, camera, width, height, settings.shadow_cascade_count)),
            _ => None,
        })
        .unwrap_or([Mat4::IDENTITY; SHADOW_CASCADE_COUNT]);
    let mut spot_shadow_view_projections = [[[0.0; 4]; 4]; MAX_SPOT_SHADOWS];
    let mut spot_shadow_rects = [[0.0; 4]; MAX_SPOT_SHADOWS];
    let mut point_shadow_matrices = [[[0.0; 4]; 4]; MAX_POINT_SHADOWS * POINT_SHADOW_FACE_COUNT as usize];
    let mut point_slot = 0usize;
    let atlas_size = settings.shadow_budget.atlas_size.max(SPOT_SHADOW_MAP_SIZE);
    let spot_resolution = settings.shadow_resolution.clamp(256, SHADOW_MAP_SIZE);
    let cells_per_axis = (atlas_size / spot_resolution).max(1);
    let mut light_directions = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_colors = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_positions = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_params = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_kinds = [[0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_shadow_modes = [[0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut directional_shadow_assigned = false;
    let mut spot_slot = 0usize;

    for (index, light) in lights.iter().take(MAX_DIRECTIONAL_LIGHTS).enumerate() {
        match light {
            Light::Directional(light) => {
                light_directions[index] = light.direction.extend(0.0).to_array();
                light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
                light_kinds[index] = [0, 0, 0, 0];
                if !directional_shadow_assigned {
                    light_shadow_modes[index] = [1, 0, 0, 0];
                    directional_shadow_assigned = true;
                }
            }
            Light::Point(light) => {
                light_positions[index] = light.position.extend(light.range).to_array();
                light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
                light_params[index] = [light.range, 0.0, 0.0, 0.0];
                light_kinds[index] = [1, 0, 0, 0];
                if point_slot < MAX_POINT_SHADOWS {
                    let projections = point_shadow_view_projections(light.position, light.range);
                    for (face, projection) in projections.iter().enumerate() {
                        point_shadow_matrices[point_slot * POINT_SHADOW_FACE_COUNT as usize + face] = projection.to_cols_array_2d();
                    }
                    light_shadow_modes[index] = [3, point_slot as u32, 0, 0];
                    point_slot += 1;
                }
            }
            Light::Spot(light) => {
                light_directions[index] = light.direction.extend(0.0).to_array();
                light_positions[index] = light.position.extend(light.range).to_array();
                light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
                light_params[index] = [light.range, light.inner_angle.cos(), light.outer_angle.cos(), 0.0];
                light_kinds[index] = [2, 0, 0, 0];
                if spot_slot < MAX_SPOT_SHADOWS && spot_slot < (cells_per_axis * cells_per_axis) as usize {
                    spot_shadow_view_projections[spot_slot] = spot_shadow_view_projection(light).to_cols_array_2d();
                    let cell = spot_slot as u32 % cells_per_axis;
                    let row = spot_slot as u32 / cells_per_axis;
                    spot_shadow_rects[spot_slot] = [spot_resolution as f32 / atlas_size as f32, spot_resolution as f32 / atlas_size as f32, cell as f32 * spot_resolution as f32 / atlas_size as f32, row as f32 * spot_resolution as f32 / atlas_size as f32];
                    light_shadow_modes[index] = [2, spot_slot as u32, 0, 0];
                    spot_slot += 1;
                }
            }
        }
    }
    CameraUniform {
        view_projection: camera.view_projection(aspect).to_cols_array_2d(),
        inverse_view_projection: camera.view_projection(aspect).inverse().to_cols_array_2d(),
        view: camera.transform.matrix().inverse().to_cols_array_2d(),
        shadow_view_projection: shadow_view_projections.map(|matrix| matrix.to_cols_array_2d()),
        shadow_cascade_splits: cascade_splits_for_count(camera, settings.shadow_cascade_count),
        spot_shadow_view_projections,
        spot_shadow_rects,
        spot_shadow_atlas_size: [atlas_size as f32, atlas_size as f32, 0.0, 0.0],
        point_shadow_view_projections: point_shadow_matrices,
        light_directions,
        light_colors,
        light_positions,
        light_params,
        light_kinds,
        light_shadow_modes,
        light_count: [lights.len().min(MAX_DIRECTIONAL_LIGHTS) as u32, 0, 0, 0],
        camera_position: camera.transform.position.extend(0.0).to_array(),
        debug_mode: [
            match mode {
                RenderDebugMode::LitMaterials => 0,
                RenderDebugMode::UnlitMaterials => 1,
                RenderDebugMode::Wireframe => 2,
                RenderDebugMode::ShadowVisibility => 3,
                RenderDebugMode::GiOnly => 4,
                RenderDebugMode::GiSupport => 5,
            },
            u32::from(settings.shadows_enabled),
            u32::from(settings.irradiance_enabled),
            settings.shadow_cascade_count.clamp(1, SHADOW_CASCADE_COUNT as u32),
        ],
        shadow_settings: [settings.shadow_depth_bias.max(0.0), settings.shadow_normal_bias.max(0.0), settings.shadow_filter_radius.max(0.25), settings.shadow_resolution.clamp(256, SHADOW_MAP_SIZE) as f32],
        viewport_size: [width as f32, height as f32, 0.0, 0.0],
        shadow_flags: [u32::from(settings.contact_shadows_enabled), u32::from(settings.secondary_light_shadowing), u32::from(settings.ambient_occlusion_enabled), u32::from(settings.direct_light_visibility_enabled)],
        ambient_light: [settings.ambient_intensity.max(0.0), 0.0, 0.0, 0.0],
        gi_settings: [0, u32::from(settings.triangle_probe_occlusion_enabled), 0, 0],
    }
}

fn shadow_geometry_changed(previous: &[Light], current: &[Light]) -> bool {
    if previous.len() != current.len() {
        return true;
    }
    previous.iter().zip(current).any(|(previous, current)| match (previous, current) {
        (Light::Directional(previous), Light::Directional(current)) => previous.direction != current.direction,
        (Light::Point(previous), Light::Point(current)) => previous.position != current.position || previous.range != current.range,
        (Light::Spot(previous), Light::Spot(current)) => previous.position != current.position || previous.direction != current.direction || previous.range != current.range || previous.inner_angle != current.inner_angle || previous.outer_angle != current.outer_angle,
        _ => true,
    })
}

fn wireframe_indices(indices: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(indices.len() * 2);

    for triangle in indices.as_chunks::<3>().0 {
        result.extend_from_slice(&[triangle[0], triangle[1], triangle[1], triangle[2], triangle[2], triangle[0]]);
    }

    result
}

fn cascade_splits(camera: &Camera) -> [f32; 4] {
    cascade_splits_for_count(camera, SHADOW_CASCADE_COUNT as u32)
}

fn cascade_splits_for_count(camera: &Camera, cascade_count: u32) -> [f32; 4] {
    let near = camera.near_clip.max(0.01);
    let far = camera.far_clip.max(near + 0.1);
    let lambda = 0.85;
    let cascade_count = cascade_count.clamp(1, SHADOW_CASCADE_COUNT as u32) as usize;
    let mut splits = [near, far, far, far];

    for (index, split) in splits.iter_mut().enumerate().take(cascade_count).skip(1) {
        let fraction = index as f32 / cascade_count as f32;
        let logarithmic = near * (far / near).powf(fraction);
        let uniform = near + (far - near) * fraction;
        *split = uniform * (1.0 - lambda) + logarithmic * lambda;
    }

    splits
}

fn cascade_shadow_view_projections(light: &DirectionalLight, camera: &Camera, width: u32, height: u32, cascade_count: u32) -> [Mat4; SHADOW_CASCADE_COUNT] {
    let splits = cascade_splits_for_count(camera, cascade_count);
    let cascade_count = cascade_count.clamp(1, SHADOW_CASCADE_COUNT as u32) as usize;
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    let forward = camera.transform.rotation * -Vec3::Z;
    let right = camera.transform.rotation * Vec3::X;
    let up = camera.transform.rotation * Vec3::Y;
    let tangent = (camera.field_of_view_y.to_radians() * 0.5).tan();
    let direction = light.direction.normalize_or_zero();
    let light_up = if direction.dot(Vec3::Y).abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    std::array::from_fn(|cascade_index| {
        let active_index = cascade_index.min(cascade_count - 1);
        let near = splits[active_index];
        let far = splits[active_index + 1];
        let near_half_height = near * tangent;
        let far_half_height = far * tangent;
        let near_half_width = near_half_height * aspect;
        let far_half_width = far_half_height * aspect;
        let near_center = camera.transform.position + forward * near;
        let far_center = camera.transform.position + forward * far;
        let corners = [
            near_center - right * near_half_width - up * near_half_height,
            near_center + right * near_half_width - up * near_half_height,
            near_center - right * near_half_width + up * near_half_height,
            near_center + right * near_half_width + up * near_half_height,
            far_center - right * far_half_width - up * far_half_height,
            far_center + right * far_half_width - up * far_half_height,
            far_center - right * far_half_width + up * far_half_height,
            far_center + right * far_half_width + up * far_half_height,
        ];
        let center = corners.iter().copied().fold(Vec3::ZERO, |sum, corner| sum + corner) / corners.len() as f32;
        let radius = corners.iter().map(|corner| corner.distance(center)).fold(0.0, f32::max);
        let light_distance = radius * 2.0 + 10.0;
        let position = center + direction * light_distance;
        let view = Mat4::look_at_rh(position, center, light_up);
        let center_light = view.transform_point3(center);
        let extent = radius + 0.25;
        let texel_size = (extent * 2.0) / SHADOW_MAP_SIZE as f32;
        let snapped = Vec3::new((center_light.x / texel_size).round() * texel_size, (center_light.y / texel_size).round() * texel_size, center_light.z);
        let snapped_center = view.inverse().transform_point3(snapped);
        let snapped_position = snapped_center + direction * light_distance;
        let snapped_view = Mat4::look_at_rh(snapped_position, snapped_center, light_up);
        let near_plane = (light_distance - radius - 2.0).max(0.1);
        let far_plane = light_distance + radius + 2.0;
        let projection = Mat4::orthographic_rh(-extent, extent, -extent, extent, near_plane, far_plane);
        projection * snapped_view
    })
}

fn spot_shadow_view_projection(light: &SpotLight) -> Mat4 {
    let direction = light.direction.normalize_or_zero();
    let target = light.position + direction;
    let up = if direction.dot(Vec3::Y).abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(light.position, target, up);
    let field_of_view = (light.outer_angle * 2.0).clamp(0.1, 3.13);
    let projection = Mat4::perspective_rh(field_of_view, 1.0, 0.05, light.range.max(0.1));

    projection * view
}

pub fn point_shadow_view_projections(position: Vec3, range: f32) -> [Mat4; 6] {
    let faces = [(Vec3::X, Vec3::Y), (-Vec3::X, Vec3::Y), (Vec3::Y, Vec3::Z), (-Vec3::Y, -Vec3::Z), (Vec3::Z, Vec3::Y), (-Vec3::Z, Vec3::Y)];
    let projection = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.05, range.max(0.1));

    faces.map(|(direction, up)| projection * Mat4::look_at_rh(position, position + direction, up))
}

fn material_texture_ids(material: &mesh::Material) -> MaterialTextureIds {
    MaterialTextureIds { base_color: material.base_color_texture.as_ref().map(mesh::Texture::id), normal: material.normal_texture.as_ref().map(mesh::Texture::id), metallic: material.metallic_texture.as_ref().map(mesh::Texture::id), roughness: material.roughness_texture.as_ref().map(mesh::Texture::id) }
}

fn mesh_topology_signature(instances: &[MeshInstance]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    instances.len().hash(&mut hasher);
    for instance in instances {
        instance.mesh.geometry_version().hash(&mut hasher);
        material_texture_ids(&instance.material).hash(&mut hasher);
    }

    hasher.finish()
}

fn unique_mesh_geometry_counts(instances: &[MeshInstance]) -> (usize, usize) {
    let mut meshes = HashMap::new();
    let mut vertex_count = 0;
    let mut index_count = 0;

    for instance in instances {
        let key = (instance.mesh.geometry_version(), material_texture_ids(&instance.material));
        if meshes.insert(key, ()).is_none() {
            vertex_count += instance.mesh.vertices().len();
            index_count += instance.mesh.indices().len();
        }
    }

    (vertex_count, index_count)
}

fn mesh_instance_signature(instance: &MeshInstance) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    instance.mesh.geometry_version().hash(&mut hasher);
    instance.transform.position.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.transform.rotation.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.transform.scale.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.material.base_color.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.material.metallic.to_bits().hash(&mut hasher);
    instance.material.roughness.to_bits().hash(&mut hasher);
    instance.material.emission.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.material.emission_strength.to_bits().hash(&mut hasher);
    material_texture_ids(&instance.material).hash(&mut hasher);
    instance.probe_dynamic.hash(&mut hasher);

    hasher.finish()
}

fn flatten_meshes(instances: &[MeshInstance]) -> Result<(Vec<RendererMeshVertex>, Vec<u32>, Vec<MeshDrawBatch>), String> {
    if instances.iter().any(|instance| instance.mesh.is_empty()) {
        return Err("renderer does not accept empty mesh instances".into());
    }

    let mut mesh_batches = Vec::<MeshDrawBatch>::new();
    let mut batch_indices = HashMap::new();
    for (instance_index, instance) in instances.iter().enumerate() {
        let version = instance.mesh.geometry_version();
        let texture_ids = material_texture_ids(&instance.material);
        let batch_key = (version, texture_ids);
        let batch_index = match batch_indices.get(&batch_key) {
            Some(batch_index) => *batch_index,
            None => {
                let batch_index = mesh_batches.len();
                mesh_batches.push(MeshDrawBatch { index_range: 0..0, wireframe_range: 0..0, instance_range: 0..0, instance_indices: Vec::new(), material_texture_ids: texture_ids });
                batch_indices.insert(batch_key, batch_index);
                batch_index
            }
        };
        mesh_batches[batch_index].instance_indices.push(instance_index);
    }

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut vertex_offset = 0_u32;
    let mut instance_offset = 0_usize;
    for batch in &mut mesh_batches {
        let mesh = &instances[batch.instance_indices[0]].mesh;
        let index_start = indices.len() as u32;
        vertices.extend(mesh.vertices().iter().map(renderer_mesh_vertex));
        indices.extend(mesh.indices().iter().map(|index| index + vertex_offset));
        vertex_offset += mesh.vertices().len() as u32;
        let index_end = indices.len() as u32;
        batch.index_range = index_start..index_end;
        batch.wireframe_range = (index_start * 2)..(index_end * 2);
        batch.instance_range = instance_offset..instance_offset + batch.instance_indices.len();
        instance_offset = batch.instance_range.end;
    }

    Ok((vertices, indices, mesh_batches))
}

fn renderer_mesh_vertex(vertex: &MeshVertex) -> RendererMeshVertex {
    RendererMeshVertex { position: vertex.position, normal: vertex.normal, color: vertex.color, tex_coord: vertex.tex_coord }
}

fn build_instance_data(instances: &[MeshInstance], batches: &[MeshDrawBatch]) -> (Vec<RendererInstanceData>, Vec<usize>) {
    let mut data = Vec::with_capacity(instances.len());
    let mut data_offsets = vec![0; instances.len()];

    for batch in batches {
        for instance_index in &batch.instance_indices {
            data_offsets[*instance_index] = data.len();
            data.push(renderer_instance_data(&instances[*instance_index], *instance_index));
        }
    }

    (data, data_offsets)
}

fn renderer_instance_data(instance: &MeshInstance, instance_index: usize) -> RendererInstanceData {
    let transform = instance.transform;
    let inverse_scale = Vec3::new(
        if transform.scale.x.abs() > 1.0e-8 { transform.scale.x.recip() } else { 0.0 },
        if transform.scale.y.abs() > 1.0e-8 { transform.scale.y.recip() } else { 0.0 },
        if transform.scale.z.abs() > 1.0e-8 { transform.scale.z.recip() } else { 0.0 },
    );
    let normal_matrix = Mat3::from_quat(transform.rotation) * Mat3::from_diagonal(inverse_scale);
    let normal_columns = normal_matrix.to_cols_array_2d();
    let material_base_color = instance.material.base_color.to_array();
    let material_params = [instance.material.metallic.clamp(0.0, 1.0), instance.material.roughness.clamp(0.04, 1.0), 0.5, instance.material.emission_strength.max(0.0)];
    let material_emission = instance.material.emission.to_array();
    let surface_id = instance_index as u32 + 1;

    RendererInstanceData { model: transform.matrix().to_cols_array_2d(), normal: normal_columns.map(|column| [column[0], column[1], column[2], 0.0]), material_base_color, material_params, material_emission, surface_id, padding: [0; 3] }
}

fn draw_shadow_instance_range<'pass>(pass: &mut wgpu::RenderPass<'pass>, instance_buffer: &'pass wgpu::Buffer, batch: &MeshDrawBatch, start: usize, end: usize) {
    let first_instance = batch.instance_range.start + start;
    let last_instance = batch.instance_range.start + end;
    let instance_start = (first_instance * mem::size_of::<RendererInstanceData>()) as u64;
    let instance_end = (last_instance * mem::size_of::<RendererInstanceData>()) as u64;
    pass.set_vertex_buffer(1, instance_buffer.slice(instance_start..instance_end));

    pass.draw_indexed(batch.index_range.clone(), 0, 0..(end - start) as u32);
}

fn create_material_texture_bindings(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, instances: &[MeshInstance]) -> HashMap<MaterialTextureIds, MaterialTextureBinding> {
    let mut bindings = HashMap::new();
    let default_ids = MaterialTextureIds { base_color: None, normal: None, metallic: None, roughness: None };
    bindings.insert(default_ids, create_material_texture_binding(device, queue, layout, sampler, "default PBR material".into(), None, None, None, None));
    for instance in instances {
        let texture_ids = material_texture_ids(&instance.material);
        bindings.entry(texture_ids).or_insert_with(|| {
            create_material_texture_binding(
                device,
                queue,
                layout,
                sampler,
                "PBR material".into(),
                instance.material.base_color_texture.as_ref(),
                instance.material.normal_texture.as_ref(),
                instance.material.metallic_texture.as_ref(),
                instance.material.roughness_texture.as_ref(),
            )
        });
    }

    bindings
}

fn create_material_texture_binding(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, label: String, base_color: Option<&mesh::Texture>, normal: Option<&mesh::Texture>, metallic: Option<&mesh::Texture>, roughness: Option<&mesh::Texture>) -> MaterialTextureBinding {
    let mut textures = Vec::with_capacity(4);
    let mut views = Vec::with_capacity(4);
    let channels = [(base_color, "base color", [255, 255, 255, 255], wgpu::TextureFormat::Rgba8UnormSrgb), (normal, "normal", [128, 128, 255, 255], wgpu::TextureFormat::Rgba8Unorm), (metallic, "metallic", [255, 255, 255, 255], wgpu::TextureFormat::Rgba8Unorm), (roughness, "roughness", [255, 255, 255, 255], wgpu::TextureFormat::Rgba8Unorm)];

    for (texture, channel, default_pixels, format) in channels {
        let (width, height, rgba8) = texture.map_or((1, 1, &default_pixels[..]), |texture| (texture.width(), texture.height(), texture.rgba8()));
        let texture_label = format!("{label} {channel}");
        let gpu_texture = device.create_texture(&wgpu::TextureDescriptor { label: Some(&texture_label), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &gpu_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba8,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        views.push(gpu_texture.create_view(&wgpu::TextureViewDescriptor::default()));
        textures.push(gpu_texture);
    }
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(&label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[2]) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[3]) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });

    MaterialTextureBinding { _textures: textures, bind_group }
}

fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    (texture, view)
}

fn depth_copy_layout(width: u32, height: u32) -> wgpu::TexelCopyBufferLayout {
    let bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(height) }
}

fn create_depth_readback_buffer(device: &wgpu::Device, label: &str, width: u32, height: u32) -> wgpu::Buffer {
    let layout = depth_copy_layout(width, height);

    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: u64::from(layout.bytes_per_row.unwrap_or_default()) * u64::from(height), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false })
}

fn read_depth_buffer(device: &wgpu::Device, buffer: &wgpu::Buffer, width: u32, height: u32) -> Result<Vec<f32>, String> {
    let slice = buffer.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
    let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
    let bytes_per_row = depth_copy_layout(width, height).bytes_per_row.unwrap_or_default() as usize;
    let mut values = Vec::with_capacity((width * height) as usize);

    for row in mapped.chunks_exact(bytes_per_row).take(height as usize) {
        for bytes in row[..(width * 4) as usize].as_chunks::<4>().0 {
            values.push(f32::from_le_bytes(*bytes));
        }
    }
    drop(mapped);
    buffer.unmap();

    Ok(values)
}

fn f16_to_f32(value: u16) -> f32 {
    let sign = if value & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = (value >> 10) & 0x1f;
    let fraction = value & 0x03ff;

    match exponent {
        0 if fraction == 0 => 0.0,
        0 => sign * (f32::from(fraction) / 1024.0) * 2.0_f32.powi(-14),
        0x1f if fraction == 0 => sign * f32::INFINITY,
        0x1f => f32::NAN,
        _ => sign * (1.0 + f32::from(fraction) / 1024.0) * 2.0_f32.powi(i32::from(exponent) - 15),
    }
}

fn create_depth_sample_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture =
        device.create_texture(&wgpu::TextureDescriptor { label: Some("depth sample texture"), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH_FORMAT, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    (texture, view)
}

fn create_surface_normal_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene surface normal target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    (texture, view)
}

fn create_surface_id_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene surface id target"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    (texture, view)
}

fn create_directional_shadow_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView, [wgpu::TextureView; SHADOW_CASCADE_COUNT]) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("directional cascaded shadow map"),
        size: wgpu::Extent3d { width: SHADOW_MAP_SIZE, height: SHADOW_MAP_SIZE, depth_or_array_layers: SHADOW_CASCADE_COUNT as u32 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor { label: Some("directional cascade array"), dimension: Some(wgpu::TextureViewDimension::D2Array), array_layer_count: Some(SHADOW_CASCADE_COUNT as u32), ..Default::default() });
    let views = std::array::from_fn(|cascade_index| texture.create_view(&wgpu::TextureViewDescriptor { label: Some("directional shadow cascade"), dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: cascade_index as u32, array_layer_count: Some(1), ..Default::default() }));

    (texture, array_view, views)
}

fn create_shadow_texture(device: &wgpu::Device, atlas_size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("directional shadow map"),
        size: wgpu::Extent3d { width: atlas_size.max(SPOT_SHADOW_MAP_SIZE), height: atlas_size.max(SPOT_SHADOW_MAP_SIZE), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    (texture, view)
}

fn create_point_shadow_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView, Vec<wgpu::TextureView>) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("point shadow depth array"),
        size: wgpu::Extent3d { width: SPOT_SHADOW_MAP_SIZE, height: SPOT_SHADOW_MAP_SIZE, depth_or_array_layers: (MAX_POINT_SHADOWS as u32) * POINT_SHADOW_FACE_COUNT },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor { label: Some("point shadow depth array"), dimension: Some(wgpu::TextureViewDimension::D2Array), array_layer_count: Some((MAX_POINT_SHADOWS as u32) * POINT_SHADOW_FACE_COUNT), ..Default::default() });
    let views = (0..MAX_POINT_SHADOWS * POINT_SHADOW_FACE_COUNT as usize)
        .map(|layer| texture.create_view(&wgpu::TextureViewDescriptor { label: Some("point shadow face"), dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: layer as u32, array_layer_count: Some(1), ..Default::default() }))
        .collect();

    (texture, array_view, views)
}

fn irradiance_uniform(volumes: &[RendererIrradianceVolume], static_samples: &[ProbeSurfaceSample], dynamic_samples: &[ProbeSurfaceSample], lights: &[Light], sky_lighting: RendererSkyLighting, visibility_grid: &VisibilityGrid, evaluate_probes: bool) -> IrradianceUniform {
    let mut uniform = IrradianceUniform {
        volumes: [IrradianceVolumeUniform { minimum: [0.0; 4], maximum: [0.0; 4], resolution: [1, 1, 1, 0], probe_offset: 0, _padding: [0; 7] }; MAX_IRRADIANCE_VOLUMES],
        probes: [[0.0; 4]; MAX_IRRADIANCE_PROBES],
        directions: [[0.0; 4]; MAX_IRRADIANCE_PROBES],
        probe_positions: [[0.0; 4]; MAX_IRRADIANCE_PROBES],
        volume_count: [0, 0, 0, 0],
    };
    let mut probe_offset = 0usize;

    for (volume_index, volume) in volumes.iter().filter(|volume| volume.enabled).take(MAX_IRRADIANCE_VOLUMES).enumerate() {
        let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
        let probe_count = resolution[0] as usize * resolution[1] as usize * resolution[2] as usize;
        if probe_offset + probe_count > MAX_IRRADIANCE_PROBES {
            break;
        }
        uniform.volumes[volume_index] = IrradianceVolumeUniform { minimum: [volume.minimum[0], volume.minimum[1], volume.minimum[2], 0.0], maximum: [volume.maximum[0], volume.maximum[1], volume.maximum[2], 0.0], resolution: [resolution[0], resolution[1], resolution[2], 0], probe_offset: probe_offset as u32, _padding: [0; 7] };
        for z in 0..resolution[2] {
            for y in 0..resolution[1] {
                for x in 0..resolution[0] {
                    let index = probe_offset + z as usize * resolution[0] as usize * resolution[1] as usize + y as usize * resolution[0] as usize + x as usize;
                    if let Some(location) = visibility_grid.probe_location(probe_position(*volume, [x, y, z])) {
                        uniform.probe_positions[index] = [location.position.x, location.position.y, location.position.z, 1.0];
                        if evaluate_probes {
                            let (color, sky_visibility, direction) = probe_irradiance(location.position, static_samples, dynamic_samples, lights, visibility_grid, sky_lighting);
                            uniform.probes[index] = [color.x, color.y, color.z, sky_visibility];
                            uniform.directions[index] = [direction.x, direction.y, direction.z, direction.w];
                        } else {
                            uniform.probes[index] = [0.015, 0.015, 0.015, 0.0];
                        }
                    } else {
                        uniform.probes[index] = [0.015, 0.015, 0.015, 0.0];
                    }
                }
            }
        }
        probe_offset += probe_count;
        uniform.volume_count[0] = (volume_index + 1) as u32;
    }

    uniform
}

fn refresh_probe_locations(uniform: &mut IrradianceUniform, volumes: &[RendererIrradianceVolume], visibility_grid: &VisibilityGrid) {
    uniform.probe_positions.fill([0.0; 4]);
    uniform.probes.fill([0.015, 0.015, 0.015, 0.0]);
    uniform.directions.fill([0.0; 4]);
    let mut probe_offset = 0usize;

    for volume in volumes.iter().filter(|volume| volume.enabled).take(MAX_IRRADIANCE_VOLUMES) {
        let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
        let probe_count = resolution[0] as usize * resolution[1] as usize * resolution[2] as usize;
        if probe_offset + probe_count > MAX_IRRADIANCE_PROBES {
            break;
        }

        for z in 0..resolution[2] {
            for y in 0..resolution[1] {
                for x in 0..resolution[0] {
                    let index = probe_offset + z as usize * resolution[0] as usize * resolution[1] as usize + y as usize * resolution[0] as usize + x as usize;
                    if let Some(location) = visibility_grid.probe_location(probe_position(*volume, [x, y, z])) {
                        uniform.probe_positions[index] = [location.position.x, location.position.y, location.position.z, 1.0];
                    }
                }
            }
        }
        probe_offset += probe_count;
    }
}

fn probe_position(volume: RendererIrradianceVolume, coordinate: [u32; 3]) -> Vec3 {
    let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
    let denominator = [resolution[0].saturating_sub(1).max(1), resolution[1].saturating_sub(1).max(1), resolution[2].saturating_sub(1).max(1)];
    let fraction = Vec3::new(coordinate[0] as f32 / denominator[0] as f32, coordinate[1] as f32 / denominator[1] as f32, coordinate[2] as f32 / denominator[2] as f32);

    Vec3::from_array(volume.minimum) + (Vec3::from_array(volume.maximum) - Vec3::from_array(volume.minimum)) * fraction
}

fn preserve_overlapping_probes(old_cache: &IrradianceUniform, old_ages: &[u32; MAX_IRRADIANCE_PROBES], new_cache: &mut IrradianceUniform, new_ages: &mut [u32; MAX_IRRADIANCE_PROBES]) {
    let mut old_probe_indices = HashMap::new();

    for (index, position) in old_cache.probe_positions.iter().enumerate() {
        if position[3] > 0.0 {
            old_probe_indices.insert(probe_position_key(*position), index);
        }
    }

    for (new_index, position) in new_cache.probe_positions.iter().enumerate() {
        if position[3] <= 0.0 {
            continue;
        }
        let Some(old_index) = old_probe_indices.get(&probe_position_key(*position)).copied() else { continue };
        new_cache.probes[new_index] = old_cache.probes[old_index];
        new_cache.directions[new_index] = old_cache.directions[old_index];
        new_ages[new_index] = old_ages[old_index];
    }
}

fn probe_position_key(position: [f32; 4]) -> (i32, i32, i32, i32) {
    (
        (position[0] * 1000.0).round() as i32,
        (position[1] * 1000.0).round() as i32,
        (position[2] * 1000.0).round() as i32,
        position[3].round() as i32,
    )
}

fn probe_location_for_index(volumes: &[&RendererIrradianceVolume], index: usize, visibility_grid: &VisibilityGrid) -> Option<ProbeLocation> {
    let mut local_index = index;

    for volume in volumes {
        let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
        let probe_count = resolution[0] as usize * resolution[1] as usize * resolution[2] as usize;
        if local_index < probe_count {
            let x = local_index % resolution[0] as usize;
            let y = local_index / resolution[0] as usize % resolution[1] as usize;
            let z = local_index / (resolution[0] as usize * resolution[1] as usize);
            return visibility_grid.probe_location(probe_position(**volume, [x as u32, y as u32, z as u32]));
        }
        local_index -= probe_count;
    }

    None
}

#[derive(Clone, Copy, PartialEq)]
struct GeometryBounds {
    minimum: Vec3,
    maximum: Vec3,
}

impl SdfGeometryBvh {
    fn new(geometries: &[SdfGeometry]) -> Self {
        let mut bvh = Self { geometries: geometries.to_vec(), geometry_indices: (0..geometries.len()).collect(), nodes: Vec::new() };

        if !geometries.is_empty() {
            bvh.build_node(0, geometries.len());
        }

        bvh
    }

    fn build_node(&mut self, start: usize, end: usize) -> usize {
        let first_geometry = self.geometries[self.geometry_indices[start]];
        let mut bounds = first_geometry.bounds;
        let mut centroid_minimum = (first_geometry.bounds.minimum + first_geometry.bounds.maximum) * 0.5;
        let mut centroid_maximum = centroid_minimum;

        for geometry_index in &self.geometry_indices[start + 1..end] {
            let geometry = self.geometries[*geometry_index];
            bounds.minimum = bounds.minimum.min(geometry.bounds.minimum);
            bounds.maximum = bounds.maximum.max(geometry.bounds.maximum);
            let centroid = (geometry.bounds.minimum + geometry.bounds.maximum) * 0.5;
            centroid_minimum = centroid_minimum.min(centroid);
            centroid_maximum = centroid_maximum.max(centroid);
        }
        let node_index = self.nodes.len();
        self.nodes.push(SdfBvhNode { bounds, left: 0, right: 0, start, count: end - start });
        if end - start <= 8 {
            return node_index;
        }
        let centroid_extent = centroid_maximum - centroid_minimum;
        let axis = if centroid_extent.x >= centroid_extent.y && centroid_extent.x >= centroid_extent.z {
            0
        } else if centroid_extent.y >= centroid_extent.z {
            1
        } else {
            2
        };
        self.geometry_indices[start..end].sort_unstable_by(|left, right| {
            let left_bounds = self.geometries[*left].bounds;
            let right_bounds = self.geometries[*right].bounds;
            let left_center = (left_bounds.minimum + left_bounds.maximum)[axis];
            let right_center = (right_bounds.minimum + right_bounds.maximum)[axis];
            left_center.partial_cmp(&right_center).unwrap_or(std::cmp::Ordering::Equal)
        });
        let middle = start + (end - start) / 2;
        let left = self.build_node(start, middle);
        let right = self.build_node(middle, end);
        self.nodes[node_index].left = left;
        self.nodes[node_index].right = right;
        self.nodes[node_index].count = 0;

        node_index
    }

    fn nearest(&self, position: Vec3, voxel_padding: Vec3) -> (f32, Vec3, Vec3) {
        if self.nodes.is_empty() {
            return (SDF_MAX_DISTANCE, Vec3::ZERO, Vec3::ZERO);
        }
        let mut nearest_distance = SDF_MAX_DISTANCE;
        let mut nearest_albedo = Vec3::ZERO;
        let mut nearest_emission = Vec3::ZERO;
        let mut stack = [usize::MAX; 64];
        let mut stack_size = 1;
        stack[0] = 0;
        while stack_size > 0 {
            stack_size -= 1;
            let node = self.nodes[stack[stack_size]];
            if signed_distance_to_padded_bounds(position, node.bounds, voxel_padding) > nearest_distance {
                continue;
            }

            if node.count > 0 {
                for ordered_index in node.start..node.start + node.count {
                    let geometry = self.geometries[self.geometry_indices[ordered_index]];
                    let distance = signed_distance_to_padded_bounds(position, geometry.bounds, voxel_padding);
                    if distance < nearest_distance {
                        nearest_distance = distance;
                        nearest_albedo = geometry.albedo;
                        nearest_emission = geometry.emission;
                    }
                }
                continue;
            }
            let left_distance = signed_distance_to_padded_bounds(position, self.nodes[node.left].bounds, voxel_padding);
            let right_distance = signed_distance_to_padded_bounds(position, self.nodes[node.right].bounds, voxel_padding);
            if left_distance <= right_distance {
                if right_distance <= nearest_distance {
                    stack[stack_size] = node.right;
                    stack_size += 1;
                }
                if left_distance <= nearest_distance {
                    stack[stack_size] = node.left;
                    stack_size += 1;
                }
            } else {
                if left_distance <= nearest_distance {
                    stack[stack_size] = node.left;
                    stack_size += 1;
                }
                if right_distance <= nearest_distance {
                    stack[stack_size] = node.right;
                    stack_size += 1;
                }
            }
        }

        (nearest_distance, nearest_albedo, nearest_emission)
    }
}

impl SdfClipmap {
    fn new(scene_bounds: GeometryBounds, geometry_bounds: &[SdfGeometry]) -> Self {
        let (resolution, extent) = padded_grid_shape(scene_bounds, SDF_CLIPMAP_PADDING, SDF_TARGET_CELL_SIZE);
        let voxel_size = Vec3::splat(SDF_TARGET_CELL_SIZE);
        let minimum = static_grid_minimum(bounds_center(scene_bounds), extent);
        let maximum = minimum + extent;
        let mut clipmap = Self { resolution, minimum, maximum, voxel_size, values: Vec::new(), emission_values: Vec::new(), geometry_bounds: Vec::new(), geometry_bvh: SdfGeometryBvh::new(&[]) };
        clipmap.rebuild(geometry_bounds);

        clipmap
    }

    #[cfg(test)]
    fn update(&mut self, scene_bounds: GeometryBounds, geometry_bounds: &[SdfGeometry]) -> Option<SdfClipmapUpdate> {
        self.update_with_cpu_fields(scene_bounds, geometry_bounds, true)
    }

    fn update_gpu(&mut self, scene_bounds: GeometryBounds, geometry_bounds: &[SdfGeometry]) -> Option<SdfClipmapUpdate> {
        self.update_with_cpu_fields(scene_bounds, geometry_bounds, false)
    }

    fn update_with_cpu_fields(&mut self, scene_bounds: GeometryBounds, geometry_bounds: &[SdfGeometry], compute_cpu_fields: bool) -> Option<SdfClipmapUpdate> {
        let extent = self.maximum - self.minimum;
        let minimum = if scene_bounds.minimum.cmpge(self.minimum).all() && scene_bounds.maximum.cmple(self.maximum).all() {
            self.minimum
        } else {
            static_grid_minimum(bounds_center(scene_bounds), extent)
        };
        let scene_within_grid = scene_bounds.minimum.cmpge(self.minimum).all() && scene_bounds.maximum.cmple(self.maximum).all();
        let bounds_changed = !scene_within_grid && self.minimum != minimum;

        if self.geometry_bounds == geometry_bounds && !bounds_changed {
            return None;
        }

        if bounds_changed || self.geometry_bounds.len() != geometry_bounds.len() {
            self.minimum = minimum;
            self.maximum = minimum + extent;
            let build_timing = if compute_cpu_fields {
                self.rebuild(geometry_bounds)
            } else {
                self.geometry_bounds.clear();
                self.geometry_bounds.extend_from_slice(geometry_bounds);
                self.geometry_bvh = SdfGeometryBvh::new(&[]);
                SdfClipmapBuildTiming::default()
            };
            let full_region = SdfTextureRegion { origin: [0; 3], size: self.resolution };
            return Some(SdfClipmapUpdate { texture_regions: vec![full_region], clearance_regions: vec![full_region], voxels_updated: self.resolution.iter().product(), geometry_bvh_build_ms: build_timing.geometry_bvh_build_ms, voxel_build_ms: build_timing.voxel_build_ms, clearance_build_ms: build_timing.clearance_build_ms });
        }
        let changed_bounds = self
            .geometry_bounds
            .iter()
            .zip(geometry_bounds)
            .filter(|(old, new)| old != new)
            .flat_map(|(old, new)| [old.bounds, new.bounds])
            .collect::<Vec<_>>();
        if changed_bounds.is_empty() {
            return None;
        }
        let texture_regions = changed_bounds
            .iter()
            .filter_map(|bounds| self.sdf_bounds_region(*bounds, Vec3::splat(SDF_MAX_DISTANCE) + self.voxel_size * 0.5))
            .collect::<Vec<_>>();
        let texture_regions = merge_sdf_texture_regions(texture_regions);
        let clearance_regions = changed_bounds
            .iter()
            .filter_map(|bounds| self.sdf_bounds_region(*bounds, self.voxel_size * 0.5))
            .map(|region| expand_sdf_texture_region(region, SDF_CLEARANCE_RADIUS_VOXELS as u32, self.resolution))
            .collect::<Vec<_>>();
        let clearance_regions = merge_sdf_texture_regions(clearance_regions);
        self.geometry_bounds.clear();
        self.geometry_bounds.extend_from_slice(geometry_bounds);
        self.geometry_bvh = if compute_cpu_fields { SdfGeometryBvh::new(geometry_bounds) } else { SdfGeometryBvh::new(&[]) };
        let geometry_bvh_build_ms = 0.0;
        let (voxel_build_ms, clearance_build_ms) = if compute_cpu_fields {
            let voxel_build_start = Instant::now();
            self.rebuild_voxel_regions(&texture_regions);
            let voxel_build_ms = voxel_build_start.elapsed().as_secs_f32() * 1000.0;
            let clearance_build_start = Instant::now();
            self.rebuild_clearance_regions(&clearance_regions);
            let clearance_build_ms = clearance_build_start.elapsed().as_secs_f32() * 1000.0;
            (voxel_build_ms, clearance_build_ms)
        } else {
            (0.0, 0.0)
        };
        let voxels_updated = texture_regions.iter().map(|region| region.size.iter().product::<u32>()).sum();

        return Some(SdfClipmapUpdate { texture_regions, clearance_regions, voxels_updated, geometry_bvh_build_ms, voxel_build_ms, clearance_build_ms });
    }

    fn uniform(&self, enabled: bool, sky_lighting: RendererSkyLighting) -> SdfClipmapUniform {
        SdfClipmapUniform {
            minimum: self.minimum.extend(0.0).to_array(),
            maximum: self.maximum.extend(0.0).to_array(),
            resolution: [self.resolution[0], self.resolution[1], self.resolution[2], 0],
            settings: [u32::from(enabled), 0, 0, 0],
            sky_radiance: [sky_lighting.color.red * sky_lighting.intensity, sky_lighting.color.green * sky_lighting.intensity, sky_lighting.color.blue * sky_lighting.intensity, 0.0],
        }
    }

    fn release_cpu_fields(&mut self) {
        self.values.clear();
        self.values.shrink_to_fit();
        self.emission_values.clear();
        self.emission_values.shrink_to_fit();

        self.geometry_bvh = SdfGeometryBvh::new(&[]);
    }

    fn rebuild(&mut self, geometry_bounds: &[SdfGeometry]) -> SdfClipmapBuildTiming {
        // CPU reference fields retain proxy bounds; runtime GPU updates query source triangles.
        self.geometry_bounds.clear();
        self.geometry_bounds.extend_from_slice(geometry_bounds);
        let geometry_bvh_build_start = Instant::now();
        self.geometry_bvh = SdfGeometryBvh::new(geometry_bounds);
        let geometry_bvh_build_ms = geometry_bvh_build_start.elapsed().as_secs_f32() * 1000.0;
        let voxel_count = self.resolution.iter().product::<u32>() as usize;
        self.values = vec![[SDF_MAX_DISTANCE, 0.0, 0.0, 0.0]; voxel_count];
        self.emission_values = vec![[0.0; 4]; voxel_count];
        let voxel_build_start = Instant::now();
        self.rebuild_all_voxels();
        let voxel_build_ms = voxel_build_start.elapsed().as_secs_f32() * 1000.0;
        let clearance_build_start = Instant::now();
        self.rebuild_conservative_clearance();
        let clearance_build_ms = clearance_build_start.elapsed().as_secs_f32() * 1000.0;

        SdfClipmapBuildTiming { geometry_bvh_build_ms, voxel_build_ms, clearance_build_ms }
    }

    fn sdf_bounds_region(&self, bounds: GeometryBounds, margin: Vec3) -> Option<SdfTextureRegion> {
        let minimum = ((bounds.minimum - margin - self.minimum) / self.voxel_size - Vec3::splat(0.5)).floor().max(Vec3::ZERO);
        let maximum = ((bounds.maximum + margin - self.minimum) / self.voxel_size - Vec3::splat(0.5))
            .ceil()
            .min(Vec3::new((self.resolution[0] - 1) as f32, (self.resolution[1] - 1) as f32, (self.resolution[2] - 1) as f32));
        if !minimum.cmple(maximum).all() {
            return None;
        }

        Some(SdfTextureRegion { origin: minimum.as_uvec3().to_array(), size: (maximum - minimum).as_uvec3().to_array().map(|size| size + 1) })
    }

    fn rebuild_voxel_regions(&mut self, regions: &[SdfTextureRegion]) {
        let mut dirty_indices = sdf_region_indices(regions, self.resolution);
        dirty_indices.sort_unstable();
        dirty_indices.dedup();
        if dirty_indices.is_empty() {
            return;
        }
        let worker_count = std::thread::available_parallelism().map(usize::from).unwrap_or(1).min(dirty_indices.len());
        let indices_per_worker = dirty_indices.len().div_ceil(worker_count);
        let mut updates = vec![(0_usize, [0.0_f32; 4], [0.0_f32; 3]); dirty_indices.len()];
        let minimum = self.minimum;
        let voxel_size = self.voxel_size;
        let voxel_padding = voxel_size * 0.5;
        let resolution = self.resolution;
        let geometry_bvh = &self.geometry_bvh;
        std::thread::scope(|scope| {
            for (worker_index, updates) in updates.chunks_mut(indices_per_worker).enumerate() {
                let indices = &dirty_indices[worker_index * indices_per_worker..worker_index * indices_per_worker + updates.len()];
                scope.spawn(move || {
                    for ((index, value, emission_value), voxel_index) in updates.iter_mut().zip(indices) {
                        let voxel_index = *voxel_index as usize;
                        let x = voxel_index % resolution[0] as usize;
                        let y = (voxel_index / resolution[0] as usize) % resolution[1] as usize;
                        let z = voxel_index / (resolution[0] * resolution[1]) as usize;
                        let position = minimum + (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) * voxel_size;
                        let (distance, albedo, emission) = geometry_bvh.nearest(position, voxel_padding);
                        *index = voxel_index;
                        *value = [distance.clamp(-SDF_MAX_DISTANCE, SDF_MAX_DISTANCE), albedo.x, albedo.y, albedo.z];
                        *emission_value = emission.to_array();
                    }
                });
            }
        });
        for (index, value, emission) in updates {
            self.values[index] = value;
            self.emission_values[index][..3].copy_from_slice(&emission);
        }
    }

    fn rebuild_clearance_regions(&mut self, regions: &[SdfTextureRegion]) {
        let mut dirty_indices = sdf_region_indices(regions, self.resolution);
        dirty_indices.sort_unstable();
        dirty_indices.dedup();
        if dirty_indices.is_empty() {
            return;
        }
        let worker_count = std::thread::available_parallelism().map(usize::from).unwrap_or(1).min(dirty_indices.len());
        let indices_per_worker = dirty_indices.len().div_ceil(worker_count);
        let mut updates = vec![(0_usize, 0.0_f32); dirty_indices.len()];
        let half_voxel_diagonal = self.voxel_size.length() * 0.5;
        let minimum_voxel_size = self.voxel_size.min_element();
        let values = &self.values;
        let resolution = self.resolution;
        std::thread::scope(|scope| {
            for (worker_index, updates) in updates.chunks_mut(indices_per_worker).enumerate() {
                let indices = &dirty_indices[worker_index * indices_per_worker..worker_index * indices_per_worker + updates.len()];
                scope.spawn(move || {
                    for ((index, safe_distance), voxel_index) in updates.iter_mut().zip(indices) {
                        let voxel_index = *voxel_index as usize;
                        let x = voxel_index % resolution[0] as usize;
                        let y = (voxel_index / resolution[0] as usize) % resolution[1] as usize;
                        let z = voxel_index / (resolution[0] * resolution[1]) as usize;
                        let mut distance = SDF_CLEARANCE_RADIUS_VOXELS;
                        'neighborhood: for offset_z in -(SDF_CLEARANCE_RADIUS_VOXELS as i32)..=SDF_CLEARANCE_RADIUS_VOXELS as i32 {
                            for offset_y in -(SDF_CLEARANCE_RADIUS_VOXELS as i32)..=SDF_CLEARANCE_RADIUS_VOXELS as i32 {
                                for offset_x in -(SDF_CLEARANCE_RADIUS_VOXELS as i32)..=SDF_CLEARANCE_RADIUS_VOXELS as i32 {
                                    let neighbor_x = x as i32 + offset_x;
                                    let neighbor_y = y as i32 + offset_y;
                                    let neighbor_z = z as i32 + offset_z;
                                    if neighbor_x < 0 || neighbor_y < 0 || neighbor_z < 0 || neighbor_x >= resolution[0] as i32 || neighbor_y >= resolution[1] as i32 || neighbor_z >= resolution[2] as i32 {
                                        continue;
                                    }
                                    let neighbor_index = neighbor_x as usize + neighbor_y as usize * resolution[0] as usize + neighbor_z as usize * resolution[0] as usize * resolution[1] as usize;
                                    if values[neighbor_index][0] <= 0.0 {
                                        distance = distance.min(offset_x.unsigned_abs().max(offset_y.unsigned_abs()).max(offset_z.unsigned_abs()) as u16);
                                        if distance == 0 {
                                            break 'neighborhood;
                                        }
                                    }
                                }
                            }
                        }
                        *index = voxel_index;
                        *safe_distance = (distance as f32 * minimum_voxel_size - half_voxel_diagonal).max(0.0);
                    }
                });
            }
        });
        for (index, safe_distance) in updates {
            self.emission_values[index][3] = safe_distance;
        }
    }

    fn rebuild_conservative_clearance(&mut self) {
        let resolution = self.resolution;
        let voxel_count = resolution.iter().product::<u32>() as usize;
        let mut clearance = vec![u16::MAX; voxel_count];
        let index = |x: u32, y: u32, z: u32| (x + y * resolution[0] + z * resolution[0] * resolution[1]) as usize;
        let physical_index = |x: u32, y: u32, z: u32| index(x, y, z);

        for z in 0..resolution[2] {
            for y in 0..resolution[1] {
                for x in 0..resolution[0] {
                    if self.values[physical_index(x, y, z)][0] <= 0.0 {
                        clearance[index(x, y, z)] = 0;
                    }
                }
            }
        }

        for z in 0..resolution[2] {
            for y in 0..resolution[1] {
                for x in 0..resolution[0] {
                    let value_index = index(x, y, z);
                    let mut distance = clearance[value_index];
                    for dz in -1i32..=0 {
                        for dy in -1i32..=1 {
                            for dx in -1i32..=1 {
                                if dz == 0 && (dy > 0 || (dy == 0 && dx >= 0)) {
                                    continue;
                                }
                                let nx = x as i32 + dx;
                                let ny = y as i32 + dy;
                                let nz = z as i32 + dz;
                                if nx >= 0 && ny >= 0 && nz >= 0 && nx < resolution[0] as i32 && ny < resolution[1] as i32 && nz < resolution[2] as i32 {
                                    distance = distance.min(clearance[index(nx as u32, ny as u32, nz as u32)].saturating_add(1));
                                }
                            }
                        }
                    }
                    clearance[value_index] = distance;
                }
            }
        }

        for z in (0..resolution[2]).rev() {
            for y in (0..resolution[1]).rev() {
                for x in (0..resolution[0]).rev() {
                    let value_index = index(x, y, z);
                    let mut distance = clearance[value_index];
                    for dz in 0i32..=1 {
                        for dy in -1i32..=1 {
                            for dx in -1i32..=1 {
                                if dz == 0 && (dy < 0 || (dy == 0 && dx <= 0)) {
                                    continue;
                                }
                                let nx = x as i32 + dx;
                                let ny = y as i32 + dy;
                                let nz = z as i32 + dz;
                                if nx >= 0 && ny >= 0 && nz >= 0 && nx < resolution[0] as i32 && ny < resolution[1] as i32 && nz < resolution[2] as i32 {
                                    distance = distance.min(clearance[index(nx as u32, ny as u32, nz as u32)].saturating_add(1));
                                }
                            }
                        }
                    }
                    clearance[value_index] = distance;
                }
            }
        }
        let half_voxel_diagonal = self.voxel_size.length() * 0.5;
        let minimum_voxel_size = self.voxel_size.min_element();
        for z in 0..resolution[2] {
            for y in 0..resolution[1] {
                for x in 0..resolution[0] {
                    let distance = clearance[index(x, y, z)].min(SDF_CLEARANCE_RADIUS_VOXELS);
                    let safe_distance = (distance as f32 * minimum_voxel_size - half_voxel_diagonal).max(0.0);
                    self.emission_values[physical_index(x, y, z)][3] = safe_distance;
                }
            }
        }
    }

    fn rebuild_all_voxels(&mut self) {
        let resolution = self.resolution;
        let row_width = resolution[0] as usize;
        let row_count = resolution[1] as usize * resolution[2] as usize;
        let worker_count = std::thread::available_parallelism().map(usize::from).unwrap_or(1).min(row_count);
        let rows_per_worker = row_count.div_ceil(worker_count);
        let values_per_worker = rows_per_worker * row_width;
        let minimum = self.minimum;
        let voxel_size = self.voxel_size;
        let geometry_bvh = &self.geometry_bvh;
        std::thread::scope(|scope| {
            for (chunk_index, (values, emissions)) in self.values.chunks_mut(values_per_worker).zip(self.emission_values.chunks_mut(values_per_worker)).enumerate() {
                let first_row = chunk_index * rows_per_worker;
                scope.spawn(move || {
                    for local_index in 0..values.len() {
                        let row = first_row + local_index / row_width;
                        let x = local_index % row_width;
                        let y = row % resolution[1] as usize;
                        let z = row / resolution[1] as usize;
                        let position = minimum + Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) * voxel_size;
                        let (distance, albedo, emission) = geometry_bvh.nearest(position, voxel_size * 0.5);
                        values[local_index] = [distance.clamp(-SDF_MAX_DISTANCE, SDF_MAX_DISTANCE), albedo.x, albedo.y, albedo.z];
                        emissions[local_index] = [emission.x, emission.y, emission.z, 0.0];
                    }
                });
            }
        });
    }
}

impl WorldRadianceField {
    fn grid(&self) -> RendererRadianceFieldGrid {
        RendererRadianceFieldGrid { minimum: self.minimum.to_array(), maximum: self.maximum.to_array(), resolution: self.resolution }
    }

    fn cell_count(&self) -> usize {
        self.resolution.iter().product::<u32>() as usize
    }

    fn index(&self, coordinate: [u32; 3]) -> usize {
        coordinate[0] as usize + coordinate[1] as usize * self.resolution[0] as usize + coordinate[2] as usize * self.resolution[0] as usize * self.resolution[1] as usize
    }

    fn cell_coordinate(&self, position: Vec3) -> [u32; 3] {
        let coordinate = ((position - self.minimum) / self.cell_size)
            .floor()
            .clamp(Vec3::ZERO, Vec3::new(self.resolution[0] as f32 - 1.0, self.resolution[1] as f32 - 1.0, self.resolution[2] as f32 - 1.0));

        [coordinate.x as u32, coordinate.y as u32, coordinate.z as u32]
    }

    fn new(scene_bounds: GeometryBounds, cell_size: f32) -> Self {
        let (resolution, extent, effective_cell_size) = radiance_field_grid_shape(scene_bounds, cell_size);
        let cell_size = Vec3::splat(effective_cell_size);
        let minimum = static_grid_minimum(bounds_center(scene_bounds), extent);
        Self {
            resolution,
            minimum,
            maximum: minimum + extent,
            cell_size,
            frame_index: 0,
            needs_full_refresh: true,
            queued_reset_cells: VecDeque::new(),
            queued_reset_cell_mask: vec![false; resolution.iter().product::<u32>() as usize],
            queued_priority_cells: VecDeque::new(),
            queued_priority_cell_mask: vec![false; resolution.iter().product::<u32>() as usize],
            queued_camera_cells: VecDeque::new(),
            queued_camera_cell_mask: vec![false; resolution.iter().product::<u32>() as usize],
            queued_dirty_cells: VecDeque::new(),
            queued_dirty_cell_mask: vec![false; resolution.iter().product::<u32>() as usize],
            refinement_cursor: 0,
            camera_focus_cell: None,
        }
    }

    fn reanchor(&mut self, scene_bounds: GeometryBounds) -> bool {
        if scene_bounds.minimum.cmpge(self.minimum).all() && scene_bounds.maximum.cmple(self.maximum).all() {
            return false;
        }
        let extent = self.cell_size * Vec3::new(self.resolution[0] as f32, self.resolution[1] as f32, self.resolution[2] as f32);
        let cell_size = self.cell_size;
        let minimum = static_grid_minimum(bounds_center(scene_bounds), extent);
        if self.minimum == minimum && self.maximum == minimum + extent {
            return false;
        }
        self.minimum = minimum;
        self.maximum = minimum + extent;
        self.cell_size = cell_size;
        self.invalidate();

        true
    }

    fn uniform(&mut self, temporal_accumulation_enabled: bool, angular_region_count: u32, probe_relocation_enabled: bool, frozen: bool, camera_position: Vec3) -> RadianceFieldUniform {
        if self.needs_full_refresh {
            self.clear_pending_cells();
            for index in self.cells_nearest_to(camera_position) {
                self.enqueue_reset_cell(index);
            }
            self.needs_full_refresh = false;
        }
        self.frame_index = self.frame_index.wrapping_add(1);
        self.update_camera_focus(camera_position);
        let scheduled_cells = if frozen { Vec::new() } else { self.schedule_cells(temporal_accumulation_enabled, angular_region_count as usize) };
        let scheduled_rays = scheduled_cells.iter().map(|cell| ((cell >> RADIANCE_FIELD_RAY_COUNT_SHIFT) & 0x7f) as usize).sum::<usize>();
        let mut packed_scheduled_cells = [[u32::MAX; 4]; 8];
        for (index, cell) in scheduled_cells.iter().enumerate() {
            packed_scheduled_cells[index / 4][index % 4] = *cell;
        }
        RadianceFieldUniform {
            minimum: self.minimum.extend(0.0).to_array(),
            maximum: self.maximum.extend(0.0).to_array(),
            resolution: [self.resolution[0], self.resolution[1], self.resolution[2], 0],
            settings: [self.frame_index, RADIANCE_FIELD_MAX_RAYS_PER_CELL as u32, angular_region_count, (u32::from(temporal_accumulation_enabled) << 2) | (u32::from(probe_relocation_enabled) << 3)],
            scheduled_cells: packed_scheduled_cells,
            schedule_metadata: [scheduled_cells.len() as u32, scheduled_rays as u32, 0, 0],
        }
    }

    fn invalidate(&mut self) {
        self.needs_full_refresh = true;

        self.clear_pending_cells();
    }

    fn refresh_temporally(&mut self) {
        if self.queued_reset_cells.is_empty() && self.queued_dirty_cells.is_empty() {
            for index in 0..self.cell_count() as u32 {
                self.enqueue_dirty_cell(index);
            }
        }
    }

    fn refresh_for_lighting_change(&mut self, priority: Option<RadianceFieldPriority>) {
        if let Some(priority) = priority {
            let radius = priority.radius + self.cell_size.length() * 0.5;
            let radius_vector = Vec3::splat(radius);
            let minimum_coordinate = ((priority.position - radius_vector - self.minimum) / self.cell_size).floor().as_ivec3().to_array();
            let maximum_coordinate = ((priority.position + radius_vector - self.minimum) / self.cell_size).ceil().as_ivec3().to_array();
            let maximum_grid_coordinate = [self.resolution[0] as i32 - 1, self.resolution[1] as i32 - 1, self.resolution[2] as i32 - 1];
            let minimum_coordinate: [i32; 3] = std::array::from_fn(|axis| minimum_coordinate[axis].clamp(0, maximum_grid_coordinate[axis]));
            let maximum_coordinate: [i32; 3] = std::array::from_fn(|axis| maximum_coordinate[axis].clamp(0, maximum_grid_coordinate[axis]));
            let mut affected_cells = Vec::new();
            for z in minimum_coordinate[2]..=maximum_coordinate[2] {
                for y in minimum_coordinate[1]..=maximum_coordinate[1] {
                    for x in minimum_coordinate[0]..=maximum_coordinate[0] {
                        let coordinate = [x as u32, y as u32, z as u32];
                        let index = coordinate[0] + coordinate[1] * self.resolution[0] + coordinate[2] * self.resolution[0] * self.resolution[1];
                        let center = self.minimum + (Vec3::new(x as f32, y as f32, z as f32) + Vec3::splat(0.5)) * self.cell_size;
                        let distance_squared = center.distance_squared(priority.position);
                        if distance_squared <= radius * radius {
                            affected_cells.push((distance_squared, index));
                        }
                    }
                }
            }
            affected_cells.sort_by(|left, right| left.0.total_cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            for (_, index) in affected_cells {
                self.enqueue_priority_cell(index);
            }
            return;
        }

        // Directional lights affect the whole field. Keep one sweep pending at a time;
        // camera and local-light queues still take priority while it is processed.
        if self.queued_reset_cells.is_empty() && self.queued_dirty_cells.is_empty() {
            for index in 0..self.cell_count() as u32 {
                self.enqueue_dirty_cell(index);
            }
        }
    }

    fn cells_nearest_to(&self, position: Vec3) -> Vec<u32> {
        let mut cells = (0..self.cell_count() as u32).collect::<Vec<_>>();
        cells.sort_by(|left, right| {
            let left_coordinate = radiance_field_coordinate(*left, self.resolution);
            let right_coordinate = radiance_field_coordinate(*right, self.resolution);
            let left_position = self.minimum + (Vec3::new(left_coordinate[0] as f32, left_coordinate[1] as f32, left_coordinate[2] as f32) + Vec3::splat(0.5)) * self.cell_size;
            let right_position = self.minimum + (Vec3::new(right_coordinate[0] as f32, right_coordinate[1] as f32, right_coordinate[2] as f32) + Vec3::splat(0.5)) * self.cell_size;
            left_position.distance_squared(position).total_cmp(&right_position.distance_squared(position)).then_with(|| left.cmp(right))
        });

        cells
    }

    fn clear_pending_cells(&mut self) {
        self.queued_reset_cells.clear();
        self.queued_reset_cell_mask.fill(false);
        self.queued_priority_cells.clear();
        self.queued_priority_cell_mask.fill(false);
        self.queued_camera_cells.clear();
        self.queued_camera_cell_mask.fill(false);
        self.queued_dirty_cells.clear();

        self.queued_dirty_cell_mask.fill(false);
    }

    fn pending_cell_count(&self) -> usize {
        (0..self.cell_count())
            .filter(|index| self.queued_reset_cell_mask[*index] || self.queued_priority_cell_mask[*index] || self.queued_camera_cell_mask[*index] || self.queued_dirty_cell_mask[*index])
            .count()
    }

    fn update_camera_focus(&mut self, camera_position: Vec3) {
        let coordinate = ((camera_position - self.minimum) / self.cell_size).floor().as_ivec3().to_array();

        if self.camera_focus_cell == Some(coordinate) {
            return;
        }

        if self.camera_focus_cell.replace(coordinate).is_none() {
            return;
        }
        self.queued_camera_cells.clear();
        self.queued_camera_cell_mask.fill(false);
        for z in -1..=1 {
            for y in -1..=1 {
                for x in -1..=1 {
                    let nearby = [coordinate[0] + x, coordinate[1] + y, coordinate[2] + z];
                    if nearby.iter().any(|component| *component < 0) || nearby[0] >= self.resolution[0] as i32 || nearby[1] >= self.resolution[1] as i32 || nearby[2] >= self.resolution[2] as i32 {
                        continue;
                    }
                    let index = nearby[0] as u32 + nearby[1] as u32 * self.resolution[0] + nearby[2] as u32 * self.resolution[0] * self.resolution[1];
                    self.enqueue_camera_cell(index);
                }
            }
        }
    }

    fn enqueue_reset_cell(&mut self, index: u32) {
        enqueue_cell(&mut self.queued_reset_cells, &mut self.queued_reset_cell_mask, index);
    }

    fn enqueue_priority_cell(&mut self, index: u32) {
        enqueue_cell(&mut self.queued_priority_cells, &mut self.queued_priority_cell_mask, index);
    }

    fn enqueue_camera_cell(&mut self, index: u32) {
        enqueue_cell(&mut self.queued_camera_cells, &mut self.queued_camera_cell_mask, index);
    }

    fn enqueue_dirty_cell(&mut self, index: u32) {
        enqueue_cell(&mut self.queued_dirty_cells, &mut self.queued_dirty_cell_mask, index);
    }

    fn schedule_cells(&mut self, refinement_enabled: bool, angular_region_count: usize) -> Vec<u32> {
        let mut scheduled = Vec::with_capacity(RADIANCE_FIELD_DIRTY_CELL_BUDGET);
        let mut scheduled_mask = vec![false; self.cell_count()];
        let bootstrap_ray_count = angular_region_count.max(RADIANCE_FIELD_BOOTSTRAP_RAYS_PER_CELL).min(RADIANCE_FIELD_MAX_RAYS_PER_CELL);
        let mut ray_work = 0;
        ray_work += schedule_urgent_cells(
            &mut self.queued_priority_cells,
            &mut self.queued_priority_cell_mask,
            &mut self.queued_reset_cell_mask,
            &mut scheduled,
            &mut scheduled_mask,
            2,
            RADIANCE_FIELD_MAX_RAYS_PER_CELL,
            RADIANCE_FIELD_RAY_WORK_BUDGET - ray_work,
        );
        ray_work += schedule_urgent_cells(
            &mut self.queued_camera_cells,
            &mut self.queued_camera_cell_mask,
            &mut self.queued_reset_cell_mask,
            &mut scheduled,
            &mut scheduled_mask,
            1,
            RADIANCE_FIELD_MAX_RAYS_PER_CELL,
            RADIANCE_FIELD_RAY_WORK_BUDGET - ray_work,
        );
        ray_work += schedule_queued_cells(
            &mut self.queued_reset_cells,
            &mut self.queued_reset_cell_mask,
            &mut scheduled,
            &mut scheduled_mask,
            RADIANCE_FIELD_DIRTY_CELL_BUDGET,
            bootstrap_ray_count,
            true,
            RADIANCE_FIELD_RAY_WORK_BUDGET - ray_work,
        );
        if !self.queued_reset_cell_mask.iter().any(|queued| *queued) {
            ray_work += schedule_queued_cells(
                &mut self.queued_dirty_cells,
                &mut self.queued_dirty_cell_mask,
                &mut scheduled,
                &mut scheduled_mask,
                RADIANCE_FIELD_DIRTY_CELL_BUDGET,
                bootstrap_ray_count,
                false,
                RADIANCE_FIELD_RAY_WORK_BUDGET - ray_work,
            );
            if refinement_enabled && ray_work < RADIANCE_FIELD_RAY_WORK_BUDGET {
                let mut visited = 0;
                while scheduled.len() < RADIANCE_FIELD_DIRTY_CELL_BUDGET && ray_work + RADIANCE_FIELD_MAX_RAYS_PER_CELL <= RADIANCE_FIELD_RAY_WORK_BUDGET && visited < self.cell_count() {
                    let index = self.refinement_cursor % self.cell_count() as u32;
                    self.refinement_cursor = (self.refinement_cursor + 1) % self.cell_count() as u32;
                    visited += 1;
                    if scheduled_mask[index as usize] {
                        continue;
                    }
                    scheduled_mask[index as usize] = true;
                    scheduled.push(encode_scheduled_cell(index, RADIANCE_FIELD_MAX_RAYS_PER_CELL, false));
                    ray_work += RADIANCE_FIELD_MAX_RAYS_PER_CELL;
                }
            }
        }

        scheduled
    }
}

fn schedule_queued_cells(queue: &mut VecDeque<u32>, queued_mask: &mut Vec<bool>, scheduled: &mut Vec<u32>, scheduled_mask: &mut Vec<bool>, maximum_count: usize, ray_count: usize, reset_history: bool, remaining_work: usize) -> usize {
    let mut selected_count = 0;
    let mut ray_work = 0;

    while selected_count < maximum_count && scheduled.len() < RADIANCE_FIELD_DIRTY_CELL_BUDGET && ray_work + ray_count <= remaining_work {
        let Some(index) = pop_queued_cell(queue, queued_mask) else { break };
        let slot = index as usize;
        if scheduled_mask[slot] {
            continue;
        }
        scheduled_mask[slot] = true;
        scheduled.push(encode_scheduled_cell(index, ray_count, reset_history));
        selected_count += 1;
        ray_work += ray_count;
    }

    ray_work
}

fn schedule_urgent_cells(queue: &mut VecDeque<u32>, queued_mask: &mut Vec<bool>, reset_mask: &mut [bool], scheduled: &mut Vec<u32>, scheduled_mask: &mut [bool], maximum_count: usize, ray_count: usize, remaining_work: usize) -> usize {
    let mut selected_count = 0;
    let mut ray_work = 0;

    while selected_count < maximum_count && scheduled.len() < RADIANCE_FIELD_DIRTY_CELL_BUDGET && ray_work + ray_count <= remaining_work {
        let Some(index) = pop_queued_cell(queue, queued_mask) else { break };
        let slot = index as usize;
        if scheduled_mask[slot] {
            continue;
        }
        let reset_history = reset_mask[slot];
        if reset_history {
            reset_mask[slot] = false;
        }
        scheduled_mask[slot] = true;
        scheduled.push(encode_scheduled_cell(index, ray_count, reset_history));
        selected_count += 1;
        ray_work += ray_count;
    }

    ray_work
}

fn encode_scheduled_cell(index: u32, ray_count: usize, reset_history: bool) -> u32 {
    (index & RADIANCE_FIELD_CELL_INDEX_MASK) | ((ray_count as u32) << RADIANCE_FIELD_RAY_COUNT_SHIFT) | if reset_history { RADIANCE_FIELD_RESET_CELL_FLAG } else { 0 }
}

fn radiance_field_coverage_percentages(summaries: &[[u32; 4]], angular_region_count: u32) -> [f32; 5] {
    if summaries.is_empty() || angular_region_count == 0 {
        return [0.0; 5];
    }
    let cell_count = summaries.len() as f32;
    let total_regions = cell_count * angular_region_count as f32;
    let covered_once = summaries.iter().map(|summary| summary[0]).sum::<u32>() as f32;
    let covered_twice = summaries.iter().map(|summary| summary[1]).sum::<u32>() as f32;
    let covered_cells = summaries.iter().filter(|summary| summary[0] > 0).count() as f32;
    let attempted_regions = summaries.iter().map(|summary| summary[2]).sum::<u32>() as f32;
    let valid_cells = summaries.iter().filter(|summary| summary[3] > 0).count() as f32;

    [(covered_once * 100.0 / total_regions).clamp(0.0, 100.0), (covered_twice * 100.0 / total_regions).clamp(0.0, 100.0), (covered_cells * 100.0 / cell_count).clamp(0.0, 100.0), (attempted_regions * 100.0 / total_regions).clamp(0.0, 100.0), (valid_cells * 100.0 / cell_count).clamp(0.0, 100.0)]
}

fn radiance_field_coordinate(index: u32, resolution: [u32; 3]) -> [u32; 3] {
    [index % resolution[0], (index / resolution[0]) % resolution[1], index / (resolution[0] * resolution[1])]
}

fn enqueue_cell(queue: &mut VecDeque<u32>, queued: &mut Vec<bool>, index: u32) {
    let Some(is_queued) = queued.get_mut(index as usize) else { return };

    if !*is_queued {
        *is_queued = true;
        queue.push_back(index);
    }
}

fn pop_queued_cell(queue: &mut VecDeque<u32>, queued: &mut Vec<bool>) -> Option<u32> {
    while let Some(index) = queue.pop_front() {
        let is_queued = queued.get_mut(index as usize)?;
        if *is_queued {
            *is_queued = false;
            return Some(index);
        }
    }

    None
}

fn changed_light_priority(previous: &[Light], current: &[Light]) -> Option<RadianceFieldPriority> {
    let mut minimum = Vec3::splat(f32::MAX);
    let mut maximum = Vec3::splat(f32::MIN);
    let mut has_local_light = false;

    for index in 0..previous.len().max(current.len()) {
        let old_light = previous.get(index).copied();
        let new_light = current.get(index).copied();
        if old_light == new_light {
            continue;
        }

        for light in [old_light, new_light].into_iter().flatten() {
            let (position, range) = match light {
                Light::Directional(_) => return None,
                Light::Point(light) => (light.position, light.range),
                Light::Spot(light) => (light.position, light.range),
            };
            let radius = Vec3::splat(range.max(0.0));
            minimum = minimum.min(position - radius);
            maximum = maximum.max(position + radius);
            has_local_light = true;
        }
    }

    if !has_local_light {
        return None;
    }
    let center = (minimum + maximum) * 0.5;

    Some(RadianceFieldPriority { position: center, radius: (maximum - minimum).length() * 0.5 })
}

fn expand_sdf_texture_region(region: SdfTextureRegion, amount: u32, resolution: [u32; 3]) -> SdfTextureRegion {
    let end = [region.origin[0] + region.size[0], region.origin[1] + region.size[1], region.origin[2] + region.size[2]];
    let origin = [region.origin[0].saturating_sub(amount), region.origin[1].saturating_sub(amount), region.origin[2].saturating_sub(amount)];
    let expanded_end = [end[0].saturating_add(amount).min(resolution[0]), end[1].saturating_add(amount).min(resolution[1]), end[2].saturating_add(amount).min(resolution[2])];

    SdfTextureRegion { origin, size: [expanded_end[0] - origin[0], expanded_end[1] - origin[1], expanded_end[2] - origin[2]] }
}

fn sdf_region_indices(regions: &[SdfTextureRegion], resolution: [u32; 3]) -> Vec<u32> {
    let voxel_count = regions.iter().map(|region| region.size.iter().product::<u32>() as usize).sum();
    let mut indices = Vec::with_capacity(voxel_count);

    for region in regions {
        for z in region.origin[2]..region.origin[2] + region.size[2] {
            for y in region.origin[1]..region.origin[1] + region.size[1] {
                let row_start = region.origin[0] + y * resolution[0] + z * resolution[0] * resolution[1];
                indices.extend(row_start..row_start + region.size[0]);
            }
        }
    }

    indices
}

fn merge_sdf_texture_regions(mut regions: Vec<SdfTextureRegion>) -> Vec<SdfTextureRegion> {
    let mut index = 0;

    while index < regions.len() {
        let mut merge_index = index + 1;
        while merge_index < regions.len() {
            let first = regions[index];
            let second = regions[merge_index];
            let first_end = [first.origin[0] + first.size[0], first.origin[1] + first.size[1], first.origin[2] + first.size[2]];
            let second_end = [second.origin[0] + second.size[0], second.origin[1] + second.size[1], second.origin[2] + second.size[2]];
            let overlaps = (0..3).all(|axis| first.origin[axis] <= second_end[axis] && second.origin[axis] <= first_end[axis]);
            if overlaps {
                let origin = [first.origin[0].min(second.origin[0]), first.origin[1].min(second.origin[1]), first.origin[2].min(second.origin[2])];
                let end = [first_end[0].max(second_end[0]), first_end[1].max(second_end[1]), first_end[2].max(second_end[2])];
                regions[index] = SdfTextureRegion { origin, size: [end[0] - origin[0], end[1] - origin[1], end[2] - origin[2]] };
                regions.swap_remove(merge_index);
            } else {
                merge_index += 1;
            }
        }
        index += 1;
    }

    regions
}

fn signed_distance_to_bounds(position: Vec3, bounds: GeometryBounds) -> f32 {
    let center = (bounds.minimum + bounds.maximum) * 0.5;
    let half_extent = (bounds.maximum - bounds.minimum) * 0.5;
    let offset = (position - center).abs() - half_extent;

    offset.max(Vec3::ZERO).length() + offset.max_element().min(0.0)
}

fn signed_distance_to_padded_bounds(position: Vec3, bounds: GeometryBounds, padding: Vec3) -> f32 {
    signed_distance_to_bounds(position, GeometryBounds { minimum: bounds.minimum - padding, maximum: bounds.maximum + padding })
}

#[derive(Clone, Copy)]
struct ProbeLocation {
    position: Vec3,
}

struct VisibilityGrid {
    minimum: Vec3,
    maximum: Vec3,
    cell_size: Vec3,
    resolution: [u32; 3],
    occupied: Vec<bool>,
    distance: Vec<f32>,
    regions: Vec<u16>,
    region_sizes: Vec<u32>,
}

impl VisibilityGrid {
    fn from_bounds(bounds: &[GeometryBounds]) -> Self {
        let mut minimum = Vec3::splat(f32::INFINITY);
        let mut maximum = Vec3::splat(f32::NEG_INFINITY);

        for bound in bounds {
            minimum = minimum.min(bound.minimum);
            maximum = maximum.max(bound.maximum);
        }

        if !minimum.is_finite() || !maximum.is_finite() {
            minimum = Vec3::splat(-1.0);
            maximum = Vec3::splat(1.0);
        }
        let padding = Vec3::splat(0.25);
        minimum -= padding;
        maximum += padding;
        let resolution = [32, 64, 32];
        let cell_size = (maximum - minimum) / Vec3::new(resolution[0] as f32, resolution[1] as f32, resolution[2] as f32);
        let cell_count = (resolution[0] * resolution[1] * resolution[2]) as usize;
        let mut grid = Self { minimum, maximum, cell_size, resolution, occupied: vec![false; cell_count], distance: vec![f32::INFINITY; cell_count], regions: vec![0; cell_count], region_sizes: vec![0] };
        for bound in bounds {
            let minimum_cell = grid.cell_coordinate(bound.minimum);
            let maximum_cell = grid.cell_coordinate(bound.maximum);
            for z in minimum_cell[2]..=maximum_cell[2] {
                for y in minimum_cell[1]..=maximum_cell[1] {
                    for x in minimum_cell[0]..=maximum_cell[0] {
                        let index = grid.index([x, y, z]);
                        grid.occupied[index] = true;
                    }
                }
            }
        }
        let mut queue = VecDeque::new();
        for (index, occupied) in grid.occupied.iter().copied().enumerate() {
            if occupied {
                grid.distance[index] = 0.0;
                queue.push_back(index);
            }
        }

        while let Some(index) = queue.pop_front() {
            let coordinate = grid.coordinate(index);
            let candidate = grid.distance[index] + 1.0;
            let neighbors = grid.neighbors(coordinate).collect::<Vec<_>>();
            for neighbor in neighbors {
                let neighbor_index = grid.index(neighbor);
                if candidate < grid.distance[neighbor_index] {
                    grid.distance[neighbor_index] = candidate;
                    queue.push_back(neighbor_index);
                }
            }
        }
        let mut next_region = 1_u16;
        for start in 0..cell_count {
            if grid.occupied[start] || grid.regions[start] != 0 {
                continue;
            }
            grid.regions[start] = next_region;
            let mut region_queue = VecDeque::from([start]);
            while let Some(index) = region_queue.pop_front() {
                let neighbors = grid.neighbors(grid.coordinate(index)).collect::<Vec<_>>();
                for neighbor in neighbors {
                    let neighbor_index = grid.index(neighbor);
                    if !grid.occupied[neighbor_index] && grid.regions[neighbor_index] == 0 {
                        grid.regions[neighbor_index] = next_region;
                        region_queue.push_back(neighbor_index);
                    }
                }
            }
            next_region = next_region.saturating_add(1).max(1);
        }
        grid.region_sizes = vec![0; next_region as usize];
        for region in &grid.regions {
            if *region != 0 {
                grid.region_sizes[*region as usize] += 1;
            }
        }

        grid
    }

    fn cell_coordinate(&self, position: Vec3) -> [u32; 3] {
        let coordinate = ((position - self.minimum) / self.cell_size)
            .floor()
            .clamp(Vec3::ZERO, Vec3::new(self.resolution[0] as f32 - 1.0, self.resolution[1] as f32 - 1.0, self.resolution[2] as f32 - 1.0));

        [coordinate.x as u32, coordinate.y as u32, coordinate.z as u32]
    }

    fn index(&self, coordinate: [u32; 3]) -> usize {
        coordinate[0] as usize + coordinate[1] as usize * self.resolution[0] as usize + coordinate[2] as usize * self.resolution[0] as usize * self.resolution[1] as usize
    }

    fn coordinate(&self, index: usize) -> [u32; 3] {
        let width = self.resolution[0] as usize;
        let layer = width * self.resolution[1] as usize;

        [(index % width) as u32, ((index / width) % self.resolution[1] as usize) as u32, (index / layer) as u32]
    }

    fn neighbors(&self, coordinate: [u32; 3]) -> impl Iterator<Item = [u32; 3]> {
        let mut neighbors = Vec::with_capacity(6);

        if coordinate[0] > 0 {
            neighbors.push([coordinate[0] - 1, coordinate[1], coordinate[2]]);
        }
        if coordinate[0] + 1 < self.resolution[0] {
            neighbors.push([coordinate[0] + 1, coordinate[1], coordinate[2]]);
        }
        if coordinate[1] > 0 {
            neighbors.push([coordinate[0], coordinate[1] - 1, coordinate[2]]);
        }
        if coordinate[1] + 1 < self.resolution[1] {
            neighbors.push([coordinate[0], coordinate[1] + 1, coordinate[2]]);
        }
        if coordinate[2] > 0 {
            neighbors.push([coordinate[0], coordinate[1], coordinate[2] - 1]);
        }
        if coordinate[2] + 1 < self.resolution[2] {
            neighbors.push([coordinate[0], coordinate[1], coordinate[2] + 1]);
        }

        neighbors.into_iter()
    }

    fn distance_at(&self, position: Vec3) -> f32 {
        if position.x < self.minimum.x || position.y < self.minimum.y || position.z < self.minimum.z || position.x >= self.maximum.x || position.y >= self.maximum.y || position.z >= self.maximum.z {
            return f32::INFINITY;
        }

        self.distance[self.index(self.cell_coordinate(position))] * self.cell_size.min_element()
    }

    fn region_at(&self, position: Vec3) -> u16 {
        if position.x < self.minimum.x || position.y < self.minimum.y || position.z < self.minimum.z || position.x >= self.maximum.x || position.y >= self.maximum.y || position.z >= self.maximum.z {
            return 0;
        }

        self.regions[self.index(self.cell_coordinate(position))]
    }

    fn region_size(&self, region: u16) -> u32 {
        self.region_sizes.get(region as usize).copied().unwrap_or(0)
    }

    fn surface_region_at(&self, position: Vec3, normal: Vec3) -> u16 {
        let direction = normal.normalize_or_zero();
        let offset_distance = direction.abs().dot(self.cell_size).max(self.cell_size.min_element());

        self.region_at(position + direction * offset_distance)
    }

    fn probe_location(&self, position: Vec3) -> Option<ProbeLocation> {
        let origin_region = self.region_at(position);
        let minimum_distance = self.cell_size.min_element() * 0.75;

        if origin_region != 0 && self.distance_at(position) > minimum_distance {
            return Some(ProbeLocation { position });
        }
        let offsets = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        let mut best_position = None;
        let mut best_score = f32::NEG_INFINITY;
        for radius in 1..=3 {
            for offset in offsets {
                let candidate = position + offset * self.cell_size * radius as f32;
                let candidate_region = self.region_at(candidate);
                if candidate_region == 0 || (origin_region != 0 && candidate_region != origin_region) {
                    continue;
                }
                let distance = self.distance_at(candidate);
                let region_preference = if origin_region == 0 { self.region_size(candidate_region) as f32 } else { 0.0 };
                let score = region_preference + distance;
                if distance > minimum_distance && score > best_score {
                    best_position = Some(candidate);
                    best_score = score;
                }
            }

            if best_position.is_some() {
                break;
            }
        }

        best_position.map(|position| ProbeLocation { position })
    }

    fn ray_reaches_sky(&self, origin: Vec3, direction: Vec3) -> bool {
        let minimum_step = self.cell_size.min_element().max(0.01) * 0.5;
        let mut distance_travelled = minimum_step;

        for _ in 0..64 {
            let position = origin + direction * distance_travelled;
            if position.x < self.minimum.x || position.y < self.minimum.y || position.z < self.minimum.z || position.x >= self.maximum.x || position.y >= self.maximum.y || position.z >= self.maximum.z {
                return true;
            }
            let distance = self.distance_at(position);
            if distance <= minimum_step * 1.5 {
                return false;
            }
            distance_travelled += distance.max(minimum_step);
        }

        false
    }

    fn ray_reaches_target(&self, origin: Vec3, target: Vec3, target_normal: Vec3) -> bool {
        let target_normal = target_normal.normalize_or_zero();
        let source_side = target_normal.dot(origin - target).signum();
        let endpoint = target + target_normal * source_side * self.cell_size.max_element() * 0.75;
        let ray = endpoint - origin;
        let length = ray.length();

        if length <= 0.01 {
            return true;
        }
        let direction = ray / length;
        let minimum_step = self.cell_size.min_element().max(0.01) * 0.5;
        let target_tolerance = (self.cell_size.min_element() * 1.5).max(minimum_step * 2.0);
        let mut distance_travelled = minimum_step;
        while distance_travelled < length - minimum_step {
            let position = origin + direction * distance_travelled;
            let distance = self.distance_at(position);
            if distance <= minimum_step * 1.5 {
                if length - distance_travelled <= target_tolerance {
                    return true;
                }
                return false;
            }
            distance_travelled += distance.max(minimum_step);
        }

        true
    }
}

fn build_probe_samples(instances: &[MeshInstance], include_static: bool) -> (Vec<ProbeSurfaceSample>, Vec<ProbeSurfaceSample>) {
    let mut static_samples = Vec::new();
    let mut dynamic_samples = Vec::new();

    for instance in instances {
        if !instance.probe_dynamic && !include_static {
            continue;
        }
        let target = if instance.probe_dynamic { &mut dynamic_samples } else { &mut static_samples };
        target.extend(sample_mesh_surface(instance));
    }

    (static_samples, dynamic_samples)
}

fn sample_mesh_surface(instance: &MeshInstance) -> Vec<ProbeSurfaceSample> {
    let vertices = instance.mesh.vertices();
    let mut triangles = Vec::new();
    let mut total_area = 0.0;

    for indices in instance.mesh.indices().chunks_exact(3) {
        let vertex_indices = [indices[0] as usize, indices[1] as usize, indices[2] as usize];
        let Some(vertex_a) = vertices.get(vertex_indices[0]).copied() else { continue };
        let Some(vertex_b) = vertices.get(vertex_indices[1]).copied() else { continue };
        let Some(vertex_c) = vertices.get(vertex_indices[2]).copied() else { continue };
        let triangle_vertices = [vertex_a, vertex_b, vertex_c];
        let positions = triangle_vertices.map(|vertex| instance.transform.transform_point(Vec3::from_array(vertex.position)));
        let area = (positions[1] - positions[0]).cross(positions[2] - positions[0]).length() * 0.5;
        if !area.is_finite() || area <= 0.000001 {
            continue;
        }
        total_area += area;
        triangles.push(ProbeSampleTriangle { vertices: triangle_vertices, positions, normals: triangle_vertices.map(|vertex| instance.transform.transform_vector(Vec3::from_array(vertex.normal)).normalize_or_zero()), cumulative_area: total_area });
    }

    if triangles.is_empty() || !total_area.is_finite() {
        let weight = 1.0 / vertices.len().max(1) as f32;
        return vertices
            .iter()
            .map(|vertex| ProbeSurfaceSample {
                position: instance.transform.transform_point(Vec3::from_array(vertex.position)),
                normal: instance.transform.transform_vector(Vec3::from_array(vertex.normal)).normalize_or_zero(),
                color: Vec3::new(vertex.color[0], vertex.color[1], vertex.color[2]) * Vec3::new(instance.material.base_color.red, instance.material.base_color.green, instance.material.base_color.blue),
                emission: Vec3::new(instance.material.emission.red, instance.material.emission.green, instance.material.emission.blue),
                emission_strength: instance.material.emission_strength,
                weight,
            })
            .collect();
    }

    let sample_count = ((total_area / PROBE_SURFACE_SAMPLE_TARGET_AREA).ceil() as usize).clamp(PROBE_SURFACE_SAMPLE_MIN_COUNT, PROBE_SURFACE_SAMPLE_MAX_COUNT);
    let sample_weight = 1.0 / sample_count as f32;
    let material_color = Vec3::new(instance.material.base_color.red, instance.material.base_color.green, instance.material.base_color.blue);
    let emission = Vec3::new(instance.material.emission.red, instance.material.emission.green, instance.material.emission.blue);
    (0..sample_count)
        .map(|sample_index| {
            let area_coordinate = (sample_index as f32 + 0.5) / sample_count as f32 * total_area;
            let triangle_index = triangles.partition_point(|triangle| triangle.cumulative_area < area_coordinate).min(triangles.len() - 1);
            let triangle = triangles[triangle_index];
            let previous_area = if triangle_index == 0 { 0.0 } else { triangles[triangle_index - 1].cumulative_area };
            let triangle_fraction = ((area_coordinate - previous_area) / (triangle.cumulative_area - previous_area)).clamp(0.0, 1.0);
            let root = triangle_fraction.sqrt();
            let split = radical_inverse(sample_index as u32 + 1, 2);
            let barycentric = [1.0 - root, root * (1.0 - split), root * split];
            let position = triangle.positions[0] * barycentric[0] + triangle.positions[1] * barycentric[1] + triangle.positions[2] * barycentric[2];
            let normal = (triangle.normals[0] * barycentric[0] + triangle.normals[1] * barycentric[1] + triangle.normals[2] * barycentric[2]).normalize_or_zero();
            let color = (Vec3::new(triangle.vertices[0].color[0], triangle.vertices[0].color[1], triangle.vertices[0].color[2]) * barycentric[0]
                + Vec3::new(triangle.vertices[1].color[0], triangle.vertices[1].color[1], triangle.vertices[1].color[2]) * barycentric[1]
                + Vec3::new(triangle.vertices[2].color[0], triangle.vertices[2].color[1], triangle.vertices[2].color[2]) * barycentric[2])
                * material_color;
            ProbeSurfaceSample { position, normal, color, emission, emission_strength: instance.material.emission_strength, weight: sample_weight }
        })
        .collect()
}

fn radical_inverse(mut value: u32, base: u32) -> f32 {
    let mut inverse_base = 1.0 / base as f32;
    let mut result = 0.0;

    while value > 0 {
        result += (value % base) as f32 * inverse_base;
        value /= base;
        inverse_base /= base as f32;
    }

    result
}

fn build_dynamic_probe_samples(instances: &[MeshInstance]) -> Vec<ProbeSurfaceSample> {
    build_probe_samples(instances, false).1
}

fn static_probe_signature(instances: &[MeshInstance]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    for instance in instances.iter().filter(|instance| !instance.probe_dynamic) {
        instance.mesh.vertices().len().hash(&mut hasher);
        instance.mesh.indices().len().hash(&mut hasher);
        for value in instance.transform.matrix().to_cols_array() {
            value.to_bits().hash(&mut hasher);
        }
        instance.material.base_color.to_array().map(f32::to_bits).hash(&mut hasher);
        instance.material.emission.to_array().map(f32::to_bits).hash(&mut hasher);
        instance.material.emission_strength.to_bits().hash(&mut hasher);
    }

    hasher.finish()
}

fn geometry_bounds(instances: &[MeshInstance]) -> Vec<GeometryBounds> {
    instances
        .iter()
        .filter_map(|instance| {
            let mut minimum = Vec3::splat(f32::INFINITY);
            let mut maximum = Vec3::splat(f32::NEG_INFINITY);
            for vertex in instance.mesh.vertices() {
                let position = instance.transform.transform_point(Vec3::from_array(vertex.position));
                minimum = minimum.min(position);
                maximum = maximum.max(position);
            }

            if minimum.is_finite() && maximum.is_finite() { Some(GeometryBounds { minimum, maximum }) } else { None }
        })
        .collect()
}

fn mesh_local_bounds(instances: &[MeshInstance]) -> Vec<GeometryBounds> {
    instances
        .iter()
        .map(|instance| {
            let mut minimum = Vec3::splat(f32::INFINITY);
            let mut maximum = Vec3::splat(f32::NEG_INFINITY);
            for vertex in instance.mesh.vertices() {
                let position = Vec3::from_array(vertex.position);
                minimum = minimum.min(position);
                maximum = maximum.max(position);
            }
            GeometryBounds { minimum, maximum }
        })
        .collect()
}

fn transformed_mesh_bounds(instances: &[MeshInstance], local_bounds: &[GeometryBounds]) -> Vec<GeometryBounds> {
    instances
        .iter()
        .zip(local_bounds)
        .map(|(instance, bounds)| {
            let local_corners = [
                Vec3::new(bounds.minimum.x, bounds.minimum.y, bounds.minimum.z),
                Vec3::new(bounds.maximum.x, bounds.minimum.y, bounds.minimum.z),
                Vec3::new(bounds.minimum.x, bounds.maximum.y, bounds.minimum.z),
                Vec3::new(bounds.maximum.x, bounds.maximum.y, bounds.minimum.z),
                Vec3::new(bounds.minimum.x, bounds.minimum.y, bounds.maximum.z),
                Vec3::new(bounds.maximum.x, bounds.minimum.y, bounds.maximum.z),
                Vec3::new(bounds.minimum.x, bounds.maximum.y, bounds.maximum.z),
                Vec3::new(bounds.maximum.x, bounds.maximum.y, bounds.maximum.z),
            ];
            let mut minimum = Vec3::splat(f32::INFINITY);
            let mut maximum = Vec3::splat(f32::NEG_INFINITY);
            for corner in local_corners {
                let position = instance.transform.transform_point(corner);
                minimum = minimum.min(position);
                maximum = maximum.max(position);
            }
            GeometryBounds { minimum, maximum }
        })
        .collect()
}

fn sdf_geometries(instances: &[MeshInstance]) -> Vec<SdfGeometry> {
    instances
        .iter()
        .filter_map(|instance| {
            let mut minimum = Vec3::splat(f32::INFINITY);
            let mut maximum = Vec3::splat(f32::NEG_INFINITY);
            let mut vertex_color = Vec3::ZERO;
            for vertex in instance.mesh.vertices() {
                let position = instance.transform.transform_point(Vec3::from_array(vertex.position));
                minimum = minimum.min(position);
                maximum = maximum.max(position);
                vertex_color += Vec3::from_array([vertex.color[0], vertex.color[1], vertex.color[2]]);
            }

            if !minimum.is_finite() || !maximum.is_finite() || instance.mesh.vertices().is_empty() {
                return None;
            }
            vertex_color /= instance.mesh.vertices().len() as f32;
            let material_color = Vec3::new(instance.material.base_color.red, instance.material.base_color.green, instance.material.base_color.blue);
            let emission_color = Vec3::new(instance.material.emission.red, instance.material.emission.green, instance.material.emission.blue) * instance.material.emission_strength;
            Some(SdfGeometry { bounds: GeometryBounds { minimum, maximum }, albedo: vertex_color * material_color, emission: emission_color, surface_signature: mesh_instance_signature(instance) })
        })
        .collect()
}

fn surface_inspection_meshes(instances: &[MeshInstance]) -> Vec<SurfaceInspectionMesh> {
    instances
        .iter()
        .enumerate()
        .filter_map(|(instance_index, instance)| {
            let mut minimum = Vec3::splat(f32::INFINITY);
            let mut maximum = Vec3::splat(f32::NEG_INFINITY);
            for vertex in instance.mesh.vertices() {
                let position = instance.transform.transform_point(Vec3::from_array(vertex.position));
                minimum = minimum.min(position);
                maximum = maximum.max(position);
            }

            if !minimum.is_finite() || !maximum.is_finite() {
                return None;
            }
            let triangles = instance
                .mesh
                .indices()
                .chunks_exact(3)
                .enumerate()
                .map(|(triangle_index, indices)| {
                    let vertices = [indices[0], indices[1], indices[2]].map(|index| instance.transform.transform_point(Vec3::from_array(instance.mesh.vertices()[index as usize].position)));
                    let vertex_color = [indices[0], indices[1], indices[2]]
                        .map(|index| {
                            let color = instance.mesh.vertices()[index as usize].color;
                            Vec3::new(color[0], color[1], color[2])
                        })
                        .into_iter()
                        .sum::<Vec3>()
                        / 3.0;
                    SurfaceInspectionTriangle {
                        vertices,
                        triangle_index: triangle_index as u32,
                        albedo: vertex_color * Vec3::new(instance.material.base_color.red, instance.material.base_color.green, instance.material.base_color.blue),
                        emission: Vec3::new(instance.material.emission.red, instance.material.emission.green, instance.material.emission.blue) * instance.material.emission_strength,
                    }
                })
                .collect();
            Some(SurfaceInspectionMesh { instance_index: instance_index as u32, bounds: GeometryBounds { minimum, maximum }, triangles })
        })
        .collect()
}

fn storage_buffer_or_dummy<T: Pod + Zeroable>(device: &wgpu::Device, label: &str, values: &[T]) -> wgpu::Buffer {
    let dummy = T::zeroed();
    let contents = if values.is_empty() { bytemuck::bytes_of(&dummy) } else { bytemuck::cast_slice(values) };

    device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST })
}

fn build_triangle_bvh(meshes: &[SurfaceInspectionMesh], strategy: TriangleBvhBuildStrategy) -> (Vec<TriangleBvhNode>, Vec<TriangleBvhTriangle>) {
    let items = meshes
        .iter()
        .flat_map(|mesh| mesh.triangles.iter().map(move |triangle| (mesh.instance_index, triangle)))
        .map(|(instance_index, triangle)| {
            let minimum = triangle.vertices.iter().copied().fold(Vec3::splat(f32::INFINITY), Vec3::min);
            let maximum = triangle.vertices.iter().copied().fold(Vec3::splat(f32::NEG_INFINITY), Vec3::max);
            TriangleBvhBuildItem {
                triangle: TriangleBvhTriangle { vertices: triangle.vertices.map(|vertex| [vertex.x, vertex.y, vertex.z, 0.0]), source: [instance_index, triangle.triangle_index, 0, 0], albedo: triangle.albedo.extend(0.0).to_array(), emission: triangle.emission.extend(0.0).to_array() },
                bounds: GeometryBounds { minimum, maximum },
                centroid: (minimum + maximum) * 0.5,
            }
        })
        .collect::<Vec<_>>();
    let mut nodes = Vec::new();
    let mut triangles = Vec::new();

    if !items.is_empty() {
        build_triangle_bvh_node(items, &mut nodes, &mut triangles, strategy, 0);
    }

    (nodes, triangles)
}

fn refit_triangle_bvh_data(nodes: &mut [TriangleBvhNode], triangles: &mut [TriangleBvhTriangle], meshes: &[SurfaceInspectionMesh]) -> bool {
    let meshes = meshes.iter().map(|mesh| (mesh.instance_index, mesh)).collect::<HashMap<_, _>>();

    for triangle in triangles.iter_mut() {
        let Some(mesh) = meshes.get(&triangle.source[0]) else {
            return false;
        };
        let Some(source_triangle) = mesh.triangles.get(triangle.source[1] as usize) else {
            return false;
        };
        triangle.vertices = source_triangle.vertices.map(|vertex| [vertex.x, vertex.y, vertex.z, 0.0]);
        triangle.albedo = source_triangle.albedo.extend(0.0).to_array();
        triangle.emission = source_triangle.emission.extend(0.0).to_array();
    }

    for node_index in (0..nodes.len()).rev() {
        let metadata = nodes[node_index].metadata;
        let (minimum, maximum) = if metadata[2] > 0 {
            let mut minimum = Vec3::splat(f32::INFINITY);
            let mut maximum = Vec3::splat(f32::NEG_INFINITY);
            for triangle_index in metadata[1]..metadata[1] + metadata[2] {
                let triangle = triangles[triangle_index as usize];
                for vertex in triangle.vertices {
                    let position = Vec3::new(vertex[0], vertex[1], vertex[2]);
                    minimum = minimum.min(position);
                    maximum = maximum.max(position);
                }
            }
            (minimum, maximum)
        } else {
            let left = nodes[node_index + 1];
            let right = nodes[metadata[0] as usize];
            (
                Vec3::new(left.minimum[0], left.minimum[1], left.minimum[2]).min(Vec3::new(right.minimum[0], right.minimum[1], right.minimum[2])),
                Vec3::new(left.maximum[0], left.maximum[1], left.maximum[2]).max(Vec3::new(right.maximum[0], right.maximum[1], right.maximum[2])),
            )
        };
        nodes[node_index].minimum = [minimum.x, minimum.y, minimum.z, 0.0];
        nodes[node_index].maximum = [maximum.x, maximum.y, maximum.z, 0.0];
    }

    true
}

fn build_triangle_bvh_node(mut items: Vec<TriangleBvhBuildItem>, nodes: &mut Vec<TriangleBvhNode>, triangles: &mut Vec<TriangleBvhTriangle>, strategy: TriangleBvhBuildStrategy, depth: u32) -> u32 {
    let node_index = nodes.len() as u32;
    let minimum = items.iter().fold(Vec3::splat(f32::INFINITY), |minimum, item| minimum.min(item.bounds.minimum));
    let maximum = items.iter().fold(Vec3::splat(f32::NEG_INFINITY), |maximum, item| maximum.max(item.bounds.maximum));
    nodes.push(TriangleBvhNode::zeroed());
    let mut metadata = [0; 4];

    if items.len() <= 4 {
        metadata[1] = triangles.len() as u32;
        metadata[2] = items.len() as u32;
        triangles.extend(items.into_iter().map(|item| item.triangle));
    } else {
        let split = if strategy == TriangleBvhBuildStrategy::Sah && depth < 32 { triangle_bvh_sah_split(&mut items) } else { None }.unwrap_or_else(|| triangle_bvh_median_split(&mut items));
        let right_items = items.split_off(split);
        build_triangle_bvh_node(items, nodes, triangles, strategy, depth + 1);
        metadata[0] = build_triangle_bvh_node(right_items, nodes, triangles, strategy, depth + 1);
    }
    metadata[3] = nodes.len() as u32;
    nodes[node_index as usize] = TriangleBvhNode { minimum: [minimum.x, minimum.y, minimum.z, 0.0], maximum: [maximum.x, maximum.y, maximum.z, 0.0], metadata };

    node_index
}

fn triangle_bvh_median_split(items: &mut [TriangleBvhBuildItem]) -> usize {
    let centroid_minimum = items.iter().fold(Vec3::splat(f32::INFINITY), |minimum, item| minimum.min(item.centroid));
    let centroid_maximum = items.iter().fold(Vec3::splat(f32::NEG_INFINITY), |maximum, item| maximum.max(item.centroid));
    let centroid_extent = centroid_maximum - centroid_minimum;
    let axis = if centroid_extent.x >= centroid_extent.y && centroid_extent.x >= centroid_extent.z {
        0
    } else if centroid_extent.y >= centroid_extent.z {
        1
    } else {
        2
    };
    items.sort_by(|left, right| left.centroid[axis].total_cmp(&right.centroid[axis]));

    items.len() / 2
}

fn triangle_bvh_sah_split(items: &mut [TriangleBvhBuildItem]) -> Option<usize> {
    const BIN_COUNT: usize = 16;
    let centroid_minimum = items.iter().fold(Vec3::splat(f32::INFINITY), |minimum, item| minimum.min(item.centroid));
    let centroid_maximum = items.iter().fold(Vec3::splat(f32::NEG_INFINITY), |maximum, item| maximum.max(item.centroid));
    let mut best_split = None;
    let mut best_cost = f32::INFINITY;

    for axis in 0..3 {
        let minimum = centroid_minimum[axis];
        let extent = centroid_maximum[axis] - minimum;
        if extent <= 0.000001 {
            continue;
        }
        let mut bin_counts = [0_usize; BIN_COUNT];
        let mut bin_bounds = [GeometryBounds { minimum: Vec3::splat(f32::INFINITY), maximum: Vec3::splat(f32::NEG_INFINITY) }; BIN_COUNT];
        for item in items.iter() {
            let bin = (((item.centroid[axis] - minimum) / extent * BIN_COUNT as f32) as usize).min(BIN_COUNT - 1);
            bin_counts[bin] += 1;
            bin_bounds[bin].minimum = bin_bounds[bin].minimum.min(item.bounds.minimum);
            bin_bounds[bin].maximum = bin_bounds[bin].maximum.max(item.bounds.maximum);
        }
        let mut left_counts = [0_usize; BIN_COUNT - 1];
        let mut right_counts = [0_usize; BIN_COUNT - 1];
        let mut left_bounds = [GeometryBounds { minimum: Vec3::splat(f32::INFINITY), maximum: Vec3::splat(f32::NEG_INFINITY) }; BIN_COUNT - 1];
        let mut right_bounds = left_bounds;
        let mut count = 0;
        let mut bounds = GeometryBounds { minimum: Vec3::splat(f32::INFINITY), maximum: Vec3::splat(f32::NEG_INFINITY) };
        for bin in 0..BIN_COUNT - 1 {
            count += bin_counts[bin];
            if bin_counts[bin] > 0 {
                bounds.minimum = bounds.minimum.min(bin_bounds[bin].minimum);
                bounds.maximum = bounds.maximum.max(bin_bounds[bin].maximum);
            }
            left_counts[bin] = count;
            left_bounds[bin] = bounds;
        }
        count = 0;
        bounds = GeometryBounds { minimum: Vec3::splat(f32::INFINITY), maximum: Vec3::splat(f32::NEG_INFINITY) };
        for bin in (1..BIN_COUNT).rev() {
            count += bin_counts[bin];
            if bin_counts[bin] > 0 {
                bounds.minimum = bounds.minimum.min(bin_bounds[bin].minimum);
                bounds.maximum = bounds.maximum.max(bin_bounds[bin].maximum);
            }
            right_counts[bin - 1] = count;
            right_bounds[bin - 1] = bounds;
        }

        for split_bin in 0..BIN_COUNT - 1 {
            if left_counts[split_bin] == 0 || right_counts[split_bin] == 0 {
                continue;
            }
            let cost = bounds_surface_area(left_bounds[split_bin]) * left_counts[split_bin] as f32 + bounds_surface_area(right_bounds[split_bin]) * right_counts[split_bin] as f32;
            if cost < best_cost {
                best_cost = cost;
                best_split = Some((axis, split_bin, minimum, extent));
            }
        }
    }
    let (axis, split_bin, minimum, extent) = best_split?;
    let mut split = 0;
    for index in 0..items.len() {
        let bin = (((items[index].centroid[axis] - minimum) / extent * BIN_COUNT as f32) as usize).min(BIN_COUNT - 1);
        if bin <= split_bin {
            items.swap(index, split);
            split += 1;
        }
    }

    (split > 0 && split < items.len()).then_some(split)
}

fn bounds_surface_area(bounds: GeometryBounds) -> f32 {
    let extent = (bounds.maximum - bounds.minimum).max(Vec3::ZERO);

    2.0 * (extent.x * extent.y + extent.y * extent.z + extent.z * extent.x)
}

fn populate_surface_triangle_visibility(inspection: &mut RendererRadianceSurfaceInspection, meshes: &[SurfaceInspectionMesh]) {
    if !inspection.valid || !inspection.inside_field {
        return;
    }
    let start = Vec3::from_array(inspection.world_position) + Vec3::from_array(inspection.sdf_origin_offset);
    for candidate in &mut inspection.candidates {
        if !candidate.probe_valid {
            continue;
        }
        candidate.triangle_occlusion_tested = true;
        if let Some(hit) = closest_surface_triangle_hit(start, Vec3::from_array(candidate.probe_position), meshes) {
            candidate.triangle_occluded = true;
            candidate.triangle_instance_index = hit.instance_index;
            candidate.triangle_index = hit.triangle_index;
            candidate.triangle_hit_distance = hit.distance;
            candidate.triangle_hit_position = hit.position.to_array();
            candidate.triangle_hit_normal = hit.normal.to_array();
            candidate.triangle_hit_barycentric = hit.barycentric.to_array();
            candidate.triangle_hit_vertices = hit.vertices.map(|vertex| vertex.to_array());
        }
    }
}

fn closest_surface_triangle_hit(start: Vec3, end: Vec3, meshes: &[SurfaceInspectionMesh]) -> Option<SurfaceInspectionTriangleHit> {
    let segment = end - start;
    let segment_length = segment.length();

    if segment_length <= 0.001 {
        return None;
    }
    let direction = segment / segment_length;
    let endpoint_epsilon = 0.001_f32.max(segment_length * 0.00001);
    let mut closest_hit = None;
    for mesh in meshes {
        if !segment_intersects_bounds(start, direction, segment_length, mesh.bounds) {
            continue;
        }

        for triangle in &mesh.triangles {
            let Some((distance, barycentric)) = segment_triangle_intersection(start, direction, segment_length, endpoint_epsilon, triangle.vertices) else {
                continue;
            };
            if closest_hit.as_ref().is_some_and(|hit: &SurfaceInspectionTriangleHit| hit.distance <= distance) {
                continue;
            }
            let [first, second, third] = triangle.vertices;
            closest_hit = Some(SurfaceInspectionTriangleHit { instance_index: mesh.instance_index, triangle_index: triangle.triangle_index, distance, position: start + direction * distance, normal: (second - first).cross(third - first).normalize_or_zero(), barycentric, vertices: triangle.vertices });
        }
    }

    closest_hit
}

fn segment_intersects_bounds(start: Vec3, direction: Vec3, segment_length: f32, bounds: GeometryBounds) -> bool {
    let mut minimum_distance: f32 = 0.0;
    let mut maximum_distance = segment_length;

    for axis in 0..3 {
        let origin = start[axis];
        let direction = direction[axis];
        if direction.abs() <= 0.000001 {
            if origin < bounds.minimum[axis] || origin > bounds.maximum[axis] {
                return false;
            }
            continue;
        }
        let first_distance = (bounds.minimum[axis] - origin) / direction;
        let second_distance = (bounds.maximum[axis] - origin) / direction;
        minimum_distance = minimum_distance.max(first_distance.min(second_distance));
        maximum_distance = maximum_distance.min(first_distance.max(second_distance));
        if maximum_distance < minimum_distance {
            return false;
        }
    }

    true
}

fn segment_triangle_intersection(start: Vec3, direction: Vec3, segment_length: f32, endpoint_epsilon: f32, vertices: [Vec3; 3]) -> Option<(f32, Vec3)> {
    let [first, second, third] = vertices;
    let edge_one = second - first;
    let edge_two = third - first;
    let perpendicular = direction.cross(edge_two);
    let determinant = edge_one.dot(perpendicular);

    if determinant.abs() <= 0.0000001 {
        return None;
    }
    let inverse_determinant = determinant.recip();
    let offset = start - first;
    let barycentric_u = offset.dot(perpendicular) * inverse_determinant;
    if !(0.0..=1.0).contains(&barycentric_u) {
        return None;
    }
    let cross = offset.cross(edge_one);
    let barycentric_v = direction.dot(cross) * inverse_determinant;
    if barycentric_v < 0.0 || barycentric_u + barycentric_v > 1.0 {
        return None;
    }
    let distance = edge_two.dot(cross) * inverse_determinant;
    if distance <= 0.001 || distance >= segment_length - endpoint_epsilon {
        return None;
    }

    Some((distance, Vec3::new(1.0 - barycentric_u - barycentric_v, barycentric_u, barycentric_v)))
}

fn shadow_bounds_intersect(matrix: Mat4, bounds: GeometryBounds) -> bool {
    let corners = [
        Vec3::new(bounds.minimum.x, bounds.minimum.y, bounds.minimum.z),
        Vec3::new(bounds.maximum.x, bounds.minimum.y, bounds.minimum.z),
        Vec3::new(bounds.minimum.x, bounds.maximum.y, bounds.minimum.z),
        Vec3::new(bounds.maximum.x, bounds.maximum.y, bounds.minimum.z),
        Vec3::new(bounds.minimum.x, bounds.minimum.y, bounds.maximum.z),
        Vec3::new(bounds.maximum.x, bounds.minimum.y, bounds.maximum.z),
        Vec3::new(bounds.minimum.x, bounds.maximum.y, bounds.maximum.z),
        Vec3::new(bounds.maximum.x, bounds.maximum.y, bounds.maximum.z),
    ];
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);

    for corner in corners {
        let projected = matrix.transform_point3(corner);
        minimum = minimum.min(projected);
        maximum = maximum.max(projected);
    }

    maximum.x >= -1.05 && minimum.x <= 1.05 && maximum.y >= -1.05 && minimum.y <= 1.05 && maximum.z >= 0.0 && minimum.z <= 1.0
}

fn combined_geometry_bounds(bounds: &[GeometryBounds]) -> GeometryBounds {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);

    for bound in bounds {
        minimum = minimum.min(bound.minimum);
        maximum = maximum.max(bound.maximum);
    }

    if minimum.is_finite() && maximum.is_finite() {
        GeometryBounds { minimum, maximum }
    } else {
        GeometryBounds { minimum: Vec3::splat(-10.0), maximum: Vec3::splat(10.0) }
    }
}

fn bounds_center(bounds: GeometryBounds) -> Vec3 {
    (bounds.minimum + bounds.maximum) * 0.5
}

fn static_grid_minimum(center: Vec3, extent: Vec3) -> Vec3 {
    center - extent * 0.5
}

fn padded_grid_shape(scene_bounds: GeometryBounds, padding: Vec3, cell_size: f32) -> ([u32; 3], Vec3) {
    let bounds_size = (scene_bounds.maximum - scene_bounds.minimum).max(Vec3::ZERO);
    let padded_extent = bounds_size + padding * 2.0;
    let resolution = std::array::from_fn(|axis| (padded_extent[axis] / cell_size).ceil().max(1.0) as u32);
    let extent = Vec3::new(resolution[0] as f32, resolution[1] as f32, resolution[2] as f32) * cell_size;

    (resolution, extent)
}

fn radiance_field_grid_shape(bounds: GeometryBounds, target_cell_size: f32) -> ([u32; 3], Vec3, f32) {
    let mut effective_cell_size = target_cell_size;

    loop {
        let (resolution, extent) = padded_grid_shape(bounds, RADIANCE_FIELD_PADDING, effective_cell_size);
        let cell_count = resolution.iter().product::<u32>();
        if cell_count <= RADIANCE_FIELD_MAX_CELL_COUNT {
            return (resolution, extent, effective_cell_size);
        }
        effective_cell_size *= (cell_count as f32 / RADIANCE_FIELD_MAX_CELL_COUNT as f32).cbrt().max(1.05);
    }
}

fn probe_irradiance(position: Vec3, static_samples: &[ProbeSurfaceSample], dynamic_samples: &[ProbeSurfaceSample], lights: &[Light], visibility_grid: &VisibilityGrid, sky_lighting: RendererSkyLighting) -> (Vec3, f32, Vec4) {
    let mut irradiance = Vec3::splat(0.015);
    let mut direction_sum = Vec3::ZERO;
    let mut direction_weight = 0.0;
    let probe_region = visibility_grid.region_at(position);

    for sample in static_samples.iter().chain(dynamic_samples.iter()) {
        let world_position = sample.position;
        let world_normal = sample.normal;
        let color = sample.color;
        let sample_region = visibility_grid.surface_region_at(world_position, world_normal);
        if probe_region != 0 && sample_region != 0 && probe_region != sample_region {
            continue;
        }
        let distance = position.distance(world_position);
        let sample_attenuation = 1.0 / (1.0 + distance * distance * 0.25);
        if sample_attenuation < 0.01 {
            continue;
        }
        let source_direction = (world_position - position).normalize_or_zero();
        let source_visible = visibility_grid.ray_reaches_target(position, world_position, world_normal);
        if !source_visible {
            continue;
        }

        for light in lights {
            let light_visible = match light {
                Light::Point(light) => visibility_grid.ray_reaches_target(light.position, world_position, world_normal),
                Light::Spot(light) => visibility_grid.ray_reaches_target(light.position, world_position, world_normal),
                Light::Directional(_) => true,
            };
            if !light_visible {
                continue;
            }
            let (direct, source_direction, light_color, light_intensity) = match light {
                Light::Directional(light) => (
                    world_normal.dot(light.direction.normalize_or_zero()).max(0.0),
                    (world_position - position).normalize_or_zero(),
                    light.color,
                    light.intensity,
                ),
                Light::Point(light) => {
                    let to_light = light.position - world_position;
                    let distance_to_light = to_light.length();
                    let attenuation = (1.0 - distance_to_light / light.range.max(0.001)).max(0.0).powi(2);
                    (
                        world_normal.dot(to_light.normalize_or_zero()).max(0.0) * attenuation,
                        (world_position - position).normalize_or_zero(),
                        light.color,
                        light.intensity,
                    )
                }
                Light::Spot(light) => {
                    let to_light = light.position - world_position;
                    let distance_to_light = to_light.length();
                    let light_to_surface = (-to_light).normalize_or_zero();
                    let cone = light_to_surface.dot(light.direction.normalize_or_zero());
                    let cone_factor = ((cone - light.outer_angle.cos()) / (light.inner_angle.cos() - light.outer_angle.cos()).max(0.001)).clamp(0.0, 1.0);
                    let attenuation = (1.0 - distance_to_light / light.range.max(0.001)).max(0.0).powi(2) * cone_factor;
                    (
                        world_normal.dot(to_light.normalize_or_zero()).max(0.0) * attenuation,
                        (world_position - position).normalize_or_zero(),
                        light.color,
                        light.intensity,
                    )
                }
            };
            let reflected = Vec3::new(color.x * light_color.red, color.y * light_color.green, color.z * light_color.blue);
            let contribution = direct * light_intensity * sample_attenuation * PROBE_SURFACE_SAMPLE_SCALE * sample.weight;
            irradiance += reflected * contribution;
            direction_sum += source_direction * contribution;
            direction_weight += contribution;
        }
        let emissive_contribution = sample.emission * sample.emission_strength.max(0.0) * PROBE_SURFACE_SAMPLE_SCALE * sample.weight * sample_attenuation * 2.0;
        irradiance += emissive_contribution;
        let emissive_weight = emissive_contribution.max_element();
        direction_sum += source_direction * emissive_weight;
        direction_weight += emissive_weight;
    }
    let sky_visibility = probe_sky_visibility(position, visibility_grid);
    let sky = Vec3::new(sky_lighting.color.red, sky_lighting.color.green, sky_lighting.color.blue) * sky_lighting.intensity * sky_visibility;
    let direction = direction_sum.normalize_or_zero().extend((direction_weight * 0.35).clamp(0.0, 1.0));

    (irradiance + sky, sky_visibility, direction)
}

fn apply_second_diffuse_bounce(final_cache: &mut IrradianceUniform, direct_cache: &IrradianceUniform, static_samples: &[ProbeSurfaceSample], dynamic_samples: &[ProbeSurfaceSample], visibility_grid: &VisibilityGrid) {
    for index in 0..MAX_IRRADIANCE_PROBES {
        let position = final_cache.probe_positions[index];
        if position[3] < 0.5 {
            continue;
        }
        let position = Vec3::new(position[0], position[1], position[2]);
        let (bounce, bounce_direction) = probe_second_diffuse_bounce(position, static_samples, dynamic_samples, direct_cache, visibility_grid);
        let direct = Vec3::new(direct_cache.probes[index][0], direct_cache.probes[index][1], direct_cache.probes[index][2]);
        let final_color = direct + bounce;
        let direct_direction = Vec4::from_array(direct_cache.directions[index]);
        let direction = (direct_direction.truncate() * direct_direction.w + bounce_direction.truncate() * bounce_direction.w).normalize_or_zero();
        let direction_weight = (direct_direction.w + bounce_direction.w).clamp(0.0, 1.0);
        final_cache.probes[index] = [final_color.x, final_color.y, final_color.z, direct_cache.probes[index][3]];
        final_cache.directions[index] = direction.extend(direction_weight).to_array();
    }
}

fn probe_second_diffuse_bounce(position: Vec3, static_samples: &[ProbeSurfaceSample], dynamic_samples: &[ProbeSurfaceSample], direct_cache: &IrradianceUniform, visibility_grid: &VisibilityGrid) -> (Vec3, Vec4) {
    let mut irradiance = Vec3::ZERO;
    let mut direction_sum = Vec3::ZERO;
    let mut direction_weight = 0.0;
    let probe_region = visibility_grid.region_at(position);

    for sample in static_samples.iter().chain(dynamic_samples.iter()) {
        let sample_region = visibility_grid.surface_region_at(sample.position, sample.normal);
        if probe_region != 0 && sample_region != 0 && probe_region != sample_region {
            continue;
        }
        let distance = position.distance(sample.position);
        let sample_attenuation = 1.0 / (1.0 + distance * distance * 0.25);
        if sample_attenuation < 0.01 {
            continue;
        }

        if !visibility_grid.ray_reaches_target(position, sample.position, sample.normal) {
            continue;
        }
        let source_to_probe = (position - sample.position).normalize_or_zero();
        let source_cosine = sample.normal.dot(source_to_probe).max(0.0);
        if source_cosine <= 0.0 {
            continue;
        }
        let incoming = sample_cached_irradiance(sample.position, sample.normal, direct_cache);
        if incoming.max_element() <= 0.0 {
            continue;
        }
        let reflected = incoming * sample.color;
        let contribution = source_cosine * sample_attenuation * PROBE_SURFACE_SAMPLE_SCALE * sample.weight * std::f32::consts::FRAC_1_PI;
        irradiance += reflected * contribution;
        let contribution_weight = reflected.max_element().max(0.0) * contribution;
        direction_sum += (sample.position - position).normalize_or_zero() * contribution_weight;
        direction_weight += contribution_weight;
    }
    let direction = direction_sum.normalize_or_zero().extend((direction_weight * 0.35).clamp(0.0, 1.0));

    (irradiance, direction)
}

fn sample_cached_irradiance(position: Vec3, normal: Vec3, cache: &IrradianceUniform) -> Vec3 {
    let mut result = Vec3::ZERO;
    let mut blended_weight = 0.0;
    let mut remaining_weight = 1.0;
    let normal = normal.normalize_or_zero();

    for volume_index in 0..cache.volume_count[0].min(MAX_IRRADIANCE_VOLUMES as u32) as usize {
        let volume = cache.volumes[volume_index];
        let minimum = Vec3::new(volume.minimum[0], volume.minimum[1], volume.minimum[2]);
        let maximum = Vec3::new(volume.maximum[0], volume.maximum[1], volume.maximum[2]);
        if position.cmplt(minimum).any() || position.cmpgt(maximum).any() {
            continue;
        }
        let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
        let maximum_coordinate = [resolution[0] - 1, resolution[1] - 1, resolution[2] - 1];
        let size = (maximum - minimum).max(Vec3::splat(0.001));
        let spacing = Vec3::new(size.x / maximum_coordinate[0].max(1) as f32, size.y / maximum_coordinate[1].max(1) as f32, size.z / maximum_coordinate[2].max(1) as f32);
        let receiver_bias_scale = if normal.y.abs() < 0.5 { 0.25 } else { 0.75 };
        let receiver_bias = normal.abs().dot(spacing) * receiver_bias_scale;
        let wall_receiver_lift = if normal.y.abs() < 0.5 { spacing.y * 0.5 } else { 0.0 };
        let sample_position = position + normal * receiver_bias + Vec3::Y * wall_receiver_lift;
        let coordinate = ((sample_position - minimum) / size).clamp(Vec3::ZERO, Vec3::ONE) * Vec3::new(maximum_coordinate[0] as f32, maximum_coordinate[1] as f32, maximum_coordinate[2] as f32);
        let base = [coordinate.x.floor() as u32, coordinate.y.floor() as u32, coordinate.z.floor() as u32];
        let fraction = coordinate - Vec3::new(base[0] as f32, base[1] as f32, base[2] as f32);
        let side_blend_width = spacing.min_element().max(0.001);
        let mut volume_result = Vec3::ZERO;
        let mut volume_direction = Vec3::ZERO;
        let mut volume_direction_weight = 0.0;
        let mut volume_weight = 0.0;
        let mut volume_color_weight = 0.0;
        for z in 0..=1 {
            for y in 0..=1 {
                for x in 0..=1 {
                    let sample_coordinate = [(base[0] + x).min(maximum_coordinate[0]), (base[1] + y).min(maximum_coordinate[1]), (base[2] + z).min(maximum_coordinate[2])];
                    let local_index = sample_coordinate[0] as usize + sample_coordinate[1] as usize * resolution[0] as usize + sample_coordinate[2] as usize * resolution[0] as usize * resolution[1] as usize;
                    let probe_index = volume.probe_offset as usize + local_index;
                    if probe_index >= MAX_IRRADIANCE_PROBES || cache.probe_positions[probe_index][3] < 0.5 {
                        continue;
                    }
                    let probe_position = cache.probe_positions[probe_index];
                    let probe_offset = Vec3::new(probe_position[0], probe_position[1], probe_position[2]) - position;
                    let front_distance = normal.dot(probe_offset);
                    let probe_distance = probe_offset.length();
                    if front_distance <= 0.0 || probe_distance <= 0.0001 {
                        continue;
                    }
                    let face_alignment = front_distance / probe_distance;
                    let side_weight = smoothstep(0.0, side_blend_width, front_distance);
                    let facing_weight = smoothstep(0.0, 0.25, face_alignment);
                    let trilinear_weight = if x == 1 { fraction.x } else { 1.0 - fraction.x } * if y == 1 { fraction.y } else { 1.0 - fraction.y } * if z == 1 { fraction.z } else { 1.0 - fraction.z };
                    let interpolation_weight = trilinear_weight * side_weight;
                    let weight = interpolation_weight * facing_weight;
                    let probe = cache.probes[probe_index];
                    let direction = cache.directions[probe_index];
                    volume_result += Vec3::new(probe[0], probe[1], probe[2]) * weight;
                    volume_direction += Vec3::new(direction[0], direction[1], direction[2]) * direction[3] * weight;
                    volume_direction_weight += direction[3] * weight;
                    volume_weight += interpolation_weight;
                    volume_color_weight += weight;
                }
            }
        }
        let support_weight = smoothstep(0.0, 0.1, volume_weight);
        if support_weight > 0.0 && volume_color_weight > 0.0001 {
            volume_result /= volume_color_weight;
            volume_direction /= volume_color_weight;
            volume_direction_weight /= volume_color_weight;
            if volume_direction_weight > 0.0 {
                let direction_alignment = normal.dot(volume_direction.normalize_or_zero()).max(0.0);
                volume_result *= 1.0 + direction_alignment * volume_direction_weight.min(1.0) * 0.35;
            }
            let blend_weight = remaining_weight * support_weight;
            result += volume_result * blend_weight;
            blended_weight += blend_weight;
            remaining_weight *= 1.0 - support_weight;
        }
    }

    result / blended_weight.max(0.0001)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let amount = ((value - edge0) / (edge1 - edge0).max(f32::EPSILON)).clamp(0.0, 1.0);

    amount * amount * (3.0 - 2.0 * amount)
}

fn probe_sky_visibility(position: Vec3, visibility_grid: &VisibilityGrid) -> f32 {
    let directions = [Vec3::Y, Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z];
    let visible = directions.iter().filter(|direction| visibility_grid.ray_reaches_sky(position, **direction)).count();

    visible as f32 / directions.len() as f32
}

#[cfg(test)]
mod triangle_bvh_tests {
    use super::*;

    fn bvh_segment_hit(start: Vec3, end: Vec3, nodes: &[TriangleBvhNode], triangles: &[TriangleBvhTriangle]) -> bool {
        let segment = end - start;
        let segment_length = segment.length();
        let direction = segment / segment_length;
        let endpoint_epsilon = 0.001_f32.max(segment_length * 0.00001);
        let mut node_index = 0;

        for _ in 0..nodes.len() {
            if node_index >= nodes.len() {
                break;
            }
            let node = nodes[node_index];
            let node_bounds = GeometryBounds { minimum: Vec3::new(node.minimum[0], node.minimum[1], node.minimum[2]), maximum: Vec3::new(node.maximum[0], node.maximum[1], node.maximum[2]) };
            if !segment_intersects_bounds(start, direction, segment_length, node_bounds) {
                node_index = node.metadata[3] as usize;
                continue;
            }

            for triangle_index in node.metadata[1]..node.metadata[1] + node.metadata[2] {
                let triangle = triangles[triangle_index as usize];
                let vertices = triangle.vertices.map(|vertex| Vec3::new(vertex[0], vertex[1], vertex[2]));
                if segment_triangle_intersection(start, direction, segment_length, endpoint_epsilon, vertices).is_some() {
                    return true;
                }
            }
            node_index += 1;
        }

        false
    }

    #[test]
    fn median_and_sah_bvhs_match_brute_force_segment_visibility() {
        let triangles = (0..128)
            .map(|triangle_index| {
                let x = (triangle_index % 16) as f32 - 8.0;
                let y = (triangle_index / 16) as f32 - 4.0;
                let z = (triangle_index * 13 % 19) as f32 * 0.15;
                let origin = Vec3::new(x, y, z);
                SurfaceInspectionTriangle { vertices: [origin, origin + Vec3::X * 0.8, origin + Vec3::Y * 0.8], triangle_index, albedo: Vec3::ONE, emission: Vec3::ZERO }
            })
            .collect::<Vec<_>>();
        let bounds = triangles
            .iter()
            .flat_map(|triangle| triangle.vertices)
            .fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(minimum, maximum), vertex| (minimum.min(vertex), maximum.max(vertex)));
        let meshes = [SurfaceInspectionMesh { instance_index: 0, bounds: GeometryBounds { minimum: bounds.0, maximum: bounds.1 }, triangles }];

        for strategy in [TriangleBvhBuildStrategy::Median, TriangleBvhBuildStrategy::Sah] {
            let (nodes, bvh_triangles) = build_triangle_bvh(&meshes, strategy);
            for ray_index in 0..512 {
                let x = (ray_index * 37 % 240) as f32 * 0.1 - 12.0;
                let y = (ray_index * 71 % 120) as f32 * 0.1 - 6.0;
                let start = Vec3::new(x, y, -1.0);
                let end = Vec3::new(x + 0.013 * (ray_index % 9) as f32, y - 0.017 * (ray_index % 7) as f32, 4.0);
                assert_eq!(
                    bvh_segment_hit(start, end, &nodes, &bvh_triangles),
                    closest_surface_triangle_hit(start, end, &meshes).is_some(),
                    "{strategy:?} ray {ray_index} visibility differed"
                );
            }
        }

        let (mut nodes, mut bvh_triangles) = build_triangle_bvh(&meshes, TriangleBvhBuildStrategy::Sah);
        let mut moved_meshes = meshes.clone();
        let translation = Vec3::new(14.0, -5.0, 2.0);
        moved_meshes[0].bounds.minimum += translation;
        moved_meshes[0].bounds.maximum += translation;
        for triangle in &mut moved_meshes[0].triangles {
            triangle.vertices = triangle.vertices.map(|vertex| vertex + translation);
        }
        assert!(refit_triangle_bvh_data(&mut nodes, &mut bvh_triangles, &moved_meshes));
        for ray_index in 0..512 {
            let x = (ray_index * 37 % 240) as f32 * 0.1 - 12.0;
            let y = (ray_index * 71 % 120) as f32 * 0.1 - 6.0;
            let start = Vec3::new(x, y, -1.0) + translation;
            let end = Vec3::new(x + 0.013 * (ray_index % 9) as f32, y - 0.017 * (ray_index % 7) as f32, 4.0) + translation;
            assert_eq!(
                bvh_segment_hit(start, end, &nodes, &bvh_triangles),
                closest_surface_triangle_hit(start, end, &moved_meshes).is_some(),
                "refitted ray {ray_index} visibility differed"
            );
        }
    }
}

#[cfg(test)]
mod sdf_clearance_tests {
    use super::*;

    fn occupied_at(clipmap: &SdfClipmap, position: Vec3) -> bool {
        let logical = ((position - clipmap.minimum) / clipmap.voxel_size).floor().as_uvec3();
        let resolution = clipmap.resolution;
        let index = (logical.x + logical.y * resolution[0] + logical.z * resolution[0] * resolution[1]) as usize;

        clipmap.values[index][0] <= 0.0
    }

    #[test]
    fn conservative_voxel_field_covers_geometry_inside_static_scene_bounds() {
        let geometries = [
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(-2.0, -0.03, -2.0), maximum: Vec3::new(2.0, 0.03, 2.0) }, albedo: Vec3::ONE, emission: Vec3::ZERO, surface_signature: 0 },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(3.0, 0.0, -2.0), maximum: Vec3::new(3.02, 3.0, 2.0) }, albedo: Vec3::ONE, emission: Vec3::ZERO, surface_signature: 0 },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(4.0, 0.0, 4.0), maximum: Vec3::new(4.1, 0.2, 4.1) }, albedo: Vec3::ONE, emission: Vec3::ZERO, surface_signature: 0 },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(-4.0, -1.0, 0.0), maximum: Vec3::new(-3.99, 1.0, 0.01) }, albedo: Vec3::ONE, emission: Vec3::ZERO, surface_signature: 0 },
        ];
        let scene_bounds = GeometryBounds { minimum: Vec3::new(-8.0, -2.0, -8.0), maximum: Vec3::new(8.0, 4.0, 8.0) };
        let mut clipmap = SdfClipmap::new(scene_bounds, &geometries);
        let geometry_samples = [Vec3::ZERO, Vec3::new(3.0, 1.0, 0.0), Vec3::new(4.0, 0.0, 4.0), Vec3::new(-4.0, 0.0, 0.0)];
        assert!(geometry_samples.iter().all(|position| occupied_at(&clipmap, *position)));

        let empty_position = Vec3::new(8.0, 1.0, 8.0);
        let empty_logical = ((empty_position - clipmap.minimum) / clipmap.voxel_size).floor().as_uvec3();
        let resolution = clipmap.resolution;
        let empty_index = (empty_logical.x + empty_logical.y * resolution[0] + empty_logical.z * resolution[0] * resolution[1]) as usize;
        let empty_center = clipmap.minimum + (empty_logical.as_vec3() + Vec3::splat(0.5)) * clipmap.voxel_size;
        let nearest_geometry_distance = geometries.iter().map(|geometry| signed_distance_to_bounds(empty_center, geometry.bounds)).fold(f32::INFINITY, f32::min);
        let safe_clearance = clipmap.emission_values[empty_index][3];
        assert!(safe_clearance > 0.0);
        assert!(safe_clearance <= nearest_geometry_distance);
        assert!(clipmap.update(scene_bounds, &geometries).is_none(), "camera movement must not shift the static SDF");
        assert!(geometry_samples.iter().all(|position| occupied_at(&clipmap, *position)));
    }

    #[test]
    fn incremental_sdf_update_matches_full_rebuild_after_geometry_motion() {
        let geometries = [
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(-2.0, -0.03, -2.0), maximum: Vec3::new(2.0, 0.03, 2.0) }, albedo: Vec3::ONE, emission: Vec3::ZERO, surface_signature: 0 },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(3.0, 0.0, -2.0), maximum: Vec3::new(3.02, 3.0, 2.0) }, albedo: Vec3::new(0.7, 0.3, 0.1), emission: Vec3::ZERO, surface_signature: 0 },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(4.0, 0.0, 4.0), maximum: Vec3::new(4.1, 0.2, 4.1) }, albedo: Vec3::ONE, emission: Vec3::ZERO, surface_signature: 0 },
        ];
        let moved_geometries = [geometries[0], SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(3.5, 0.0, -2.0), maximum: Vec3::new(3.52, 3.0, 2.0) }, ..geometries[1] }, geometries[2]];
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-8.0), maximum: Vec3::splat(8.0) };
        let mut incremental = SdfClipmap::new(scene_bounds, &geometries);
        let update = incremental.update(scene_bounds, &moved_geometries).expect("moving geometry must update the field");
        let rebuilt = SdfClipmap::new(scene_bounds, &moved_geometries);

        assert!(update.voxels_updated < incremental.resolution.iter().product());
        assert_eq!(incremental.values, rebuilt.values);
        assert_eq!(incremental.emission_values, rebuilt.emission_values);
    }
}

#[cfg(test)]
mod radiance_field_tests {
    use super::*;

    #[test]
    fn coverage_metrics_report_observed_regions_and_cells() {
        let summaries = [[2, 1, 2, 1], [1, 0, 1, 1], [0, 0, 0, 0]];

        let coverage = radiance_field_coverage_percentages(&summaries, 2);

        assert_eq!(coverage, [50.0, 100.0 / 6.0, 200.0 / 3.0, 50.0, 200.0 / 3.0]);
        assert_eq!(radiance_field_coverage_percentages(&[], 2), [0.0; 5]);
    }

    #[test]
    fn scheduled_cells_encode_ray_count_and_reset_flag() {
        let scheduled = encode_scheduled_cell(17, 8, true);

        assert_eq!(scheduled & RADIANCE_FIELD_CELL_INDEX_MASK, 17);
        assert_eq!((scheduled >> RADIANCE_FIELD_RAY_COUNT_SHIFT) & 0x7f, 8);
        assert_ne!(scheduled & RADIANCE_FIELD_RESET_CELL_FLAG, 0);
    }

    #[test]
    fn room_test_outer_walls_fit_inside_single_radiance_and_sdf_fields() {
        let scene_bounds = GeometryBounds { minimum: Vec3::new(-16.0, 0.0, -16.0), maximum: Vec3::new(16.0, 5.2, 16.0) };
        let (_, sdf_extent) = padded_grid_shape(scene_bounds, SDF_CLIPMAP_PADDING, SDF_TARGET_CELL_SIZE);
        let sdf_minimum = static_grid_minimum(bounds_center(scene_bounds), sdf_extent);
        let sdf_maximum = sdf_minimum + sdf_extent;
        let radiance_field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);

        for position in [Vec3::new(-13.15, 0.0, -13.15), Vec3::new(13.15, 5.2, 13.15)] {
            assert!(position.cmpge(sdf_minimum).all() && position.cmplt(sdf_maximum).all());
            assert!(position.cmpge(radiance_field.minimum).all() && position.cmplt(radiance_field.maximum).all());
        }

        assert_eq!(radiance_field.cell_size, Vec3::splat(RADIANCE_FIELD_TARGET_CELL_SIZE));
    }

    #[test]
    fn oversized_radiance_fields_coarsen_spacing_to_stay_within_the_cell_cap() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-400.0), maximum: Vec3::splat(400.0) };

        let field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);

        assert!(field.cell_count() <= RADIANCE_FIELD_MAX_CELL_COUNT as usize);
        assert!(field.cell_size.x > RADIANCE_FIELD_TARGET_CELL_SIZE);
        assert!(field.minimum.cmple(scene_bounds.minimum - RADIANCE_FIELD_PADDING).all());
        assert!(field.maximum.cmpge(scene_bounds.maximum + RADIANCE_FIELD_PADDING).all());
    }

    #[test]
    fn radiance_field_reanchors_without_changing_cell_spacing() {
        let small_bounds = GeometryBounds { minimum: Vec3::new(-16.0, 0.0, -16.0), maximum: Vec3::new(16.0, 5.2, 16.0) };
        let large_bounds = GeometryBounds { minimum: Vec3::new(20.0, 0.0, 0.0), maximum: Vec3::new(30.0, 5.2, 10.0) };
        let (resolution, sdf_extent) = padded_grid_shape(large_bounds, SDF_CLIPMAP_PADDING, SDF_TARGET_CELL_SIZE);
        let sdf_minimum = static_grid_minimum(bounds_center(large_bounds), sdf_extent);
        let sdf_maximum = sdf_minimum + sdf_extent;
        let mut field = WorldRadianceField::new(small_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        field.uniform(false, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);

        assert_eq!(
            resolution,
            std::array::from_fn(|axis| ((large_bounds.maximum[axis] - large_bounds.minimum[axis] + SDF_CLIPMAP_PADDING[axis] * 2.0) / SDF_TARGET_CELL_SIZE).ceil() as u32)
        );
        assert!(sdf_minimum.cmple(large_bounds.minimum - SDF_CLIPMAP_PADDING).all());
        assert!(sdf_maximum.cmpge(large_bounds.maximum + SDF_CLIPMAP_PADDING).all());
        assert!(field.reanchor(large_bounds));
        assert!(field.needs_full_refresh);
        assert!(field.minimum.cmple(large_bounds.minimum - RADIANCE_FIELD_PADDING).all());
        assert!(field.maximum.cmpge(large_bounds.maximum + RADIANCE_FIELD_PADDING).all());
        assert_eq!(field.cell_size, Vec3::splat(RADIANCE_FIELD_TARGET_CELL_SIZE));
        field.uniform(false, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        let expanded_minimum = field.minimum;
        let expanded_maximum = field.maximum;
        assert!(field.reanchor(small_bounds));
        assert!(field.needs_full_refresh);
        assert_ne!(field.minimum, expanded_minimum);
        assert_ne!(field.maximum, expanded_maximum);
        assert_eq!(field.cell_size, Vec3::splat(RADIANCE_FIELD_TARGET_CELL_SIZE));
    }

    #[test]
    fn static_radiance_cells_keep_their_world_bounds_across_updates() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-4.0), maximum: Vec3::splat(4.0) };
        let mut field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        let initial = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        let fixed_minimum = field.minimum;
        let next = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);

        assert_eq!(initial.schedule_metadata[0], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32);
        assert!(initial.scheduled_cells[0][0] & RADIANCE_FIELD_RESET_CELL_FLAG != 0);
        assert_eq!(next.schedule_metadata[0], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32);
        assert_eq!(field.minimum, fixed_minimum);
    }

    #[test]
    fn radiance_refinement_is_budgeted_and_freeze_preserves_pending_work() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-4.0), maximum: Vec3::splat(4.0) };
        let mut field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        let initial = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        assert_eq!(initial.settings[3] & 4, 4);
        assert_eq!(initial.schedule_metadata[0], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32);
        assert_eq!(initial.schedule_metadata[1], RADIANCE_FIELD_RAY_WORK_BUDGET as u32);
        assert!(initial.schedule_metadata[1] <= RADIANCE_FIELD_RAY_WORK_BUDGET as u32);
        assert!(initial.schedule_metadata[1] > RADIANCE_FIELD_RAY_COUNT as u32);

        let continuous_update = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        assert_eq!(continuous_update.schedule_metadata[0], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32);
        assert_eq!(continuous_update.schedule_metadata[1], RADIANCE_FIELD_RAY_WORK_BUDGET as u32);

        while !field.queued_reset_cells.is_empty() {
            field.uniform(false, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        }

        field.refresh_temporally();
        let frozen = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, true, Vec3::ZERO);
        assert_eq!(frozen.schedule_metadata[0], 0);
        let changed = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        assert_eq!(changed.schedule_metadata[0], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32);
        assert_eq!(changed.scheduled_cells[0][0] & RADIANCE_FIELD_RESET_CELL_FLAG, 0);

        let refinement_disabled = field.uniform(false, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        assert_eq!(refinement_disabled.settings[3] & 4, 0);

        let disabled_update = field.uniform(false, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        assert_eq!(disabled_update.schedule_metadata[0], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32);
        assert_eq!(disabled_update.schedule_metadata[1], RADIANCE_FIELD_DIRTY_CELL_BUDGET as u32 * RADIANCE_FIELD_BOOTSTRAP_RAYS_PER_CELL as u32);
    }

    #[test]
    fn camera_cell_changes_prioritize_nearby_probes_within_the_ray_budget() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-4.0), maximum: Vec3::splat(4.0) };
        let mut field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        while !field.queued_reset_cells.is_empty() {
            field.uniform(false, RadianceAngularResolution::default().segment_count(), false, false, Vec3::ZERO);
        }

        let focused = field.uniform(true, RadianceAngularResolution::default().segment_count(), false, false, Vec3::new(1.6, 0.0, 0.0));

        assert_eq!(focused.schedule_metadata[0], (RADIANCE_FIELD_RAY_WORK_BUDGET / RADIANCE_FIELD_MAX_RAYS_PER_CELL) as u32);
        assert!(focused.schedule_metadata[1] <= RADIANCE_FIELD_RAY_WORK_BUDGET as u32);
        assert_eq!((focused.scheduled_cells[0][0] >> RADIANCE_FIELD_RAY_COUNT_SHIFT) & 0x7f, RADIANCE_FIELD_MAX_RAYS_PER_CELL as u32);
        assert_eq!(focused.schedule_metadata[1], RADIANCE_FIELD_RAY_WORK_BUDGET as u32);
    }

    #[test]
    fn directional_light_sweeps_are_coalesced_while_pending() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-4.0), maximum: Vec3::splat(4.0) };
        let mut field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        field.needs_full_refresh = false;

        field.refresh_for_lighting_change(None);
        let pending_cells = field.queued_dirty_cells.len();
        field.refresh_for_lighting_change(None);

        assert_eq!(pending_cells, field.cell_count());
        assert_eq!(field.queued_dirty_cells.len(), pending_cells);
    }

    #[test]
    fn local_light_changes_only_queue_cells_inside_the_affected_region() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-8.0), maximum: Vec3::splat(8.0) };
        let mut field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        field.needs_full_refresh = false;

        field.refresh_for_lighting_change(Some(RadianceFieldPriority { position: Vec3::ZERO, radius: 2.0 }));

        assert!(!field.queued_priority_cells.is_empty());
        assert!(field.queued_priority_cells.len() < field.cell_count());
        assert!(field.queued_dirty_cells.is_empty());
    }

    #[test]
    fn urgent_light_cells_preempt_pending_resets_without_scheduling_twice() {
        let scene_bounds = GeometryBounds { minimum: Vec3::splat(-4.0), maximum: Vec3::splat(4.0) };
        let mut field = WorldRadianceField::new(scene_bounds, RADIANCE_FIELD_TARGET_CELL_SIZE);
        let urgent_index = (field.cell_count() / 2) as u32;
        field.enqueue_reset_cell(urgent_index);
        field.enqueue_reset_cell(0);
        field.enqueue_priority_cell(urgent_index);

        let scheduled = field.schedule_cells(true, RadianceAngularResolution::default().segment_count() as usize);

        assert_eq!(scheduled[0] & RADIANCE_FIELD_CELL_INDEX_MASK, urgent_index);
        assert_ne!(scheduled[0] & RADIANCE_FIELD_RESET_CELL_FLAG, 0);
        assert!(!field.queued_reset_cell_mask[urgent_index as usize]);
    }
}

#[cfg(test)]
mod shadow_scheduler_tests {
    use super::*;
    use math::PointLight;

    #[test]
    fn prioritizes_directional_lights_and_nearby_local_lights() {
        let camera = Camera::default();
        let lights = vec![Light::Point(PointLight { position: Vec3::new(30.0, 0.0, 0.0), color: Color::WHITE, intensity: 1.0, range: 10.0 }), Light::Directional(DirectionalLight::default()), Light::Point(PointLight { position: Vec3::new(2.0, 0.0, 0.0), color: Color::WHITE, intensity: 1.0, range: 10.0 })];
        let mut scheduler = ShadowScheduler::default();
        scheduler.rebuild(
            &lights,
            &camera,
            ShadowBudget { max_dynamic_lights: 2, max_point_lights: 2, max_spot_lights: 0, max_updates_per_frame: 2, atlas_size: 4096 },
        );
        assert_eq!(scheduler.requests.len(), 2);
        assert_eq!(scheduler.requests[0].light_index, 1);
        assert_eq!(scheduler.requests[1].light_index, 2);
    }

    #[test]
    fn clean_requests_are_not_scheduled_again_until_dirty() {
        let camera = Camera::default();
        let lights = [Light::Spot(SpotLight { position: Vec3::new(0.0, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 })];
        let mut scheduler = ShadowScheduler::default();
        scheduler.rebuild(&lights, &camera, ShadowBudget::default());
        assert!(scheduler.is_scheduled(0));
        scheduler.mark_clean(0);
        scheduler.rebuild(&lights, &camera, ShadowBudget::default());
        assert!(!scheduler.is_scheduled(0));
        scheduler.mark_all_dirty();

        assert!(scheduler.is_scheduled(0));
    }

    #[test]
    fn resource_table_assigns_stable_spot_atlas_offsets() {
        let requests = [ShadowRequest { light_index: 0, resource_kind: ShadowResourceKind::SpotMap, resource_slot: 0, policy: ShadowUpdatePolicy::OnChange, priority: 1, dirty: true }, ShadowRequest { light_index: 1, resource_kind: ShadowResourceKind::SpotMap, resource_slot: 1, policy: ShadowUpdatePolicy::OnChange, priority: 1, dirty: true }];
        let mut table = ShadowResourceTable::default();
        table.rebuild(&requests, 1024, 2048);
        assert_eq!(table.allocations[0].light_index, 0);
        assert_eq!(table.allocation_for(1).map(|allocation| allocation.slot), Some(1));
        assert_eq!(table.allocations[0].atlas_offset, [0, 0]);
        assert_eq!(table.allocations[1].atlas_offset, [1024, 0]);
        assert_eq!(table.allocations[1].atlas_uv_scale_offset(2048), [0.5, 0.5, 0.5, 0.0]);
    }

    #[test]
    fn resource_table_rejects_spot_allocations_outside_atlas_capacity() {
        let requests = (0..5)
            .map(|light_index| ShadowRequest { light_index, resource_kind: ShadowResourceKind::SpotMap, resource_slot: light_index, policy: ShadowUpdatePolicy::OnChange, priority: 1, dirty: true })
            .collect::<Vec<_>>();
        let mut table = ShadowResourceTable::default();
        table.rebuild(&requests, 1024, 2048);
        assert_eq!(table.allocations.len(), 4);
        assert!(table.allocation_for(4).is_none());
    }

    #[test]
    fn scheduler_respects_spot_atlas_capacity() {
        let camera = Camera::default();
        let lights = (0..5)
            .map(|index| Light::Spot(SpotLight { position: Vec3::new(index as f32, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 }))
            .collect::<Vec<_>>();
        let mut scheduler = ShadowScheduler::default();
        scheduler.rebuild(
            &lights,
            &camera,
            ShadowBudget { max_dynamic_lights: 8, max_point_lights: 0, max_spot_lights: 8, max_updates_per_frame: 8, atlas_size: SPOT_SHADOW_MAP_SIZE * 2 },
        );
        assert_eq!(scheduler.requests.len(), 4);
        assert!(scheduler.requests.iter().all(|request| request.resource_slot < 4));
    }

    #[test]
    fn camera_uniform_assigns_each_spot_shadow_slot() {
        let lights =
            [Light::Spot(SpotLight { position: Vec3::new(0.0, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 }), Light::Spot(SpotLight { position: Vec3::new(2.0, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 })];
        let settings = RenderSettings { shadow_resolution: 2048, ..RenderSettings::default() };
        let uniform = camera_uniform(1280, 720, &Camera::default(), &lights, RenderDebugMode::default(), settings, GeometryBounds { minimum: Vec3::ZERO, maximum: Vec3::ZERO });
        assert_eq!(uniform.light_shadow_modes[0], [2, 0, 0, 0]);
        assert_eq!(uniform.light_shadow_modes[1], [2, 1, 0, 0]);
        assert!(uniform.spot_shadow_rects[1][0] > 0.0);
    }

    #[test]
    fn point_shadow_allocation_reserves_six_array_layers() {
        let request = ShadowRequest { light_index: 3, resource_kind: ShadowResourceKind::PointCube, resource_slot: 1, policy: ShadowUpdatePolicy::OnChange, priority: 1, dirty: true };
        let mut table = ShadowResourceTable::default();
        table.rebuild(&[request], 1024, 4096);
        let allocation = table.allocation_for(3).expect("point allocation");
        assert_eq!(allocation.atlas_layer, 6);
        assert_eq!(allocation.point_face_layer(0), Some(6));
        assert_eq!(allocation.point_face_layer(5), Some(11));
        assert_eq!(allocation.point_face_layer(6), None);
    }

    #[test]
    fn point_shadow_faces_provide_six_distinct_projections() {
        let projections = point_shadow_view_projections(Vec3::new(1.0, 2.0, 3.0), 20.0);
        assert_eq!(projections.len(), POINT_SHADOW_FACE_COUNT as usize);
        assert!(projections.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
