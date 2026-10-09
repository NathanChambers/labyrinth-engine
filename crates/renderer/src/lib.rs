use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::mem;
use std::ops::Range;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use math::{Camera, Color, DirectionalLight, Light, Mat4, SpotLight, Vec3, Vec4};
use mesh::{MeshInstance, UiVertex};
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
const PROBE_SURFACE_SAMPLE_SCALE: f32 = 1.25;
const VISIBILITY_REBUILD_INTERVAL: u32 = 4;
const GPU_TIMESTAMP_COUNT: u32 = 12;
const SDF_CLIPMAP_RESOLUTION: [u32; 3] = [64, 32, 64];
const SDF_CLIPMAP_EXTENT: Vec3 = Vec3::new(32.0, 16.0, 32.0);
const SDF_MAX_DISTANCE: f32 = 16.0;
const RADIANCE_FIELD_RESOLUTION: [u32; 3] = [8, 4, 8];
const SURFACE_PROBE_STRIDE: u32 = 16;
const SURFACE_PROBE_CAPACITY: u32 = 131_072;
const RADIANCE_NEAR_FIELD_EXTENT: Vec3 = Vec3::new(16.0, 8.0, 16.0);
const RADIANCE_FAR_FIELD_EXTENT: Vec3 = SDF_CLIPMAP_EXTENT;
const RADIANCE_FIELD_DIRECTIONS: usize = 6;
const RADIANCE_FIELD_REFRESH_INTERVAL: u32 = 32;

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
                ShadowResourceKind::DirectionalCascade => ShadowResourceAllocation {
                    light_index: request.light_index,
                    kind: request.resource_kind,
                    slot: request.resource_slot,
                    resolution,
                    atlas_layer: request.resource_slot,
                    atlas_offset: [0, 0],
                },
                ShadowResourceKind::SpotMap => {
                    let columns = (atlas_size / resolution.max(1)).max(1);
                    let cell = request.resource_slot % columns;
                    let row = request.resource_slot / columns;
                    if (row + 1) * resolution > atlas_size {
                        continue;
                    }
                    ShadowResourceAllocation {
                        light_index: request.light_index,
                        kind: request.resource_kind,
                        slot: request.resource_slot,
                        resolution,
                        atlas_layer: 0,
                        atlas_offset: [cell * resolution, row * resolution],
                    }
                }
                ShadowResourceKind::PointCube => ShadowResourceAllocation {
                    light_index: request.light_index,
                    kind: request.resource_kind,
                    slot: request.resource_slot,
                    resolution,
                    atlas_layer: request.resource_slot * POINT_SHADOW_FACE_COUNT,
                    atlas_offset: [0, 0],
                },
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
                self.requests.push(ShadowRequest { light_index: index as u32, resource_kind, resource_slot, policy, priority, dirty: self.dirty_lights[index] });
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
    pub surface_probes_enabled: bool,
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
            surface_probes_enabled: true,
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
    material_base_color: [f32; 4],
    material_params: [f32; 4],
    material_emission: [f32; 4],
    surface_id: u32,
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
struct SurfaceProbeUniform {
    screen_resolution: [u32; 4],
    cache_settings: [u32; 4],
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
    grid_offset: [u32; 4],
    sky_radiance: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RadianceFieldUniform {
    minimum: [f32; 4],
    maximum: [f32; 4],
    resolution: [u32; 4],
    grid_offset: [u32; 4],
    scroll_shift: [i32; 4],
    settings: [u32; 4],
}

#[derive(Clone, Copy, PartialEq)]
struct SdfGeometry {
    bounds: GeometryBounds,
    albedo: Vec3,
    emission: Vec3,
}

struct SdfClipmap {
    minimum: Vec3,
    maximum: Vec3,
    voxel_size: Vec3,
    values: Vec<[f32; 4]>,
    emission_values: Vec<[f32; 4]>,
    geometry_bounds: Vec<SdfGeometry>,
    geometry_bvh: SdfGeometryBvh,
    grid_offset: [u32; 3],
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
    extent: Vec3,
    resolution: [u32; 3],
    minimum: Vec3,
    maximum: Vec3,
    cell_size: Vec3,
    grid_offset: [u32; 3],
    frame_index: u32,
    refresh_frames_remaining: u32,
    needs_full_refresh: bool,
}

#[derive(Clone, Copy)]
struct SdfTextureRegion {
    origin: [u32; 3],
    size: [u32; 3],
}

#[derive(Clone, Copy)]
struct SdfLogicalRegion {
    start: [u32; 3],
    size: [u32; 3],
}

struct SdfClipmapUpdate {
    texture_regions: Vec<SdfTextureRegion>,
    voxels_updated: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererIrradianceVolume {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    pub resolution: [u32; 3],
    pub enabled: bool,
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
    pub sdf_upload_ms: f32,
    pub sdf_voxels_updated: u32,
    pub gi_history_valid_percent: f32,
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
    pub gpu_gizmo_ms: f32,
    pub gpu_ui_ms: f32,
    pub gpu_total_ms: f32,
    pub scheduled_shadow_lights: u32,
    pub dirty_shadow_lights: u32,
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
    surface_probe_update_pipeline: wgpu::ComputePipeline,
    surface_probe_maintenance_pipeline: wgpu::ComputePipeline,
    depth_prepass_pipeline: wgpu::RenderPipeline,
    wireframe_pipeline: wgpu::RenderPipeline,
    gi_gizmo_pipeline: wgpu::RenderPipeline,
    sdf_occupancy_gizmo_pipeline: wgpu::RenderPipeline,
    gi_gizmo_bind_group: wgpu::BindGroup,
    gi_gizmo_selection_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    mesh_topology_signature: u64,
    instance_signatures: Vec<u64>,
    wireframe_index_buffer: wgpu::Buffer,
    wireframe_index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    irradiance_buffer: wgpu::Buffer,
    irradiance_cache: Box<IrradianceUniform>,
    irradiance_volume_layout: Vec<RendererIrradianceVolume>,
    sdf_clipmap: SdfClipmap,
    sdf_geometry_cache: Vec<SdfGeometry>,
    sdf_uniform_buffer: wgpu::Buffer,
    _sdf_texture: wgpu::Texture,
    _sdf_emission_texture: wgpu::Texture,
    near_radiance_field: WorldRadianceField,
    far_radiance_field: WorldRadianceField,
    near_radiance_field_uniform_buffer: wgpu::Buffer,
    far_radiance_field_uniform_buffer: wgpu::Buffer,
    _near_radiance_field_buffer: wgpu::Buffer,
    _far_radiance_field_buffer: wgpu::Buffer,
    _radiance_field_trace_debug_buffer: wgpu::Buffer,
    surface_probe_uniform_buffer: wgpu::Buffer,
    _surface_probe_buffer: wgpu::Buffer,
    sdf_gi_enabled: bool,
    gi_radiance_frozen: bool,
    irradiance_probe_ages: [u32; MAX_IRRADIANCE_PROBES],
    irradiance_update_budget: usize,
    sky_lighting: RendererSkyLighting,
    visibility_grid: VisibilityGrid,
    visibility_bounds: Vec<GeometryBounds>,
    visibility_rebuild_cooldown: u32,
    static_probe_samples: Vec<ProbeSurfaceSample>,
    dynamic_probe_samples: Vec<ProbeSurfaceSample>,
    static_probe_signature: u64,
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
    shadow_draw_ranges: Vec<Range<u32>>,
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
    surface_probe_gizmos_enabled: bool,
    world_radiance_gizmos_enabled: bool,
    world_radiance_trace_gizmos_enabled: bool,
    selected_radiance_cell: [u32; 3],
    selected_radiance_field_far: bool,
    sdf_occupancy_gizmos_enabled: bool,
    render_debug_mode: RenderDebugMode,
    render_settings: RenderSettings,
    gpu_timestamps: Option<GpuTimestampState>,
}

impl Renderer {
    fn update_shadow_scheduler_stats(&mut self) {
        let (scheduled_shadow_lights, dirty_shadow_lights) = self.shadow_scheduler.counts();
        self.performance_stats.scheduled_shadow_lights = scheduled_shadow_lights;
        self.performance_stats.dirty_shadow_lights = dirty_shadow_lights;
    }

    fn rebuild_shadow_resources(&mut self) {
        self.shadow_resources.rebuild(&self.shadow_scheduler.requests, self.render_settings.shadow_resolution, self.render_settings.shadow_budget.atlas_size);
    }

    pub fn new(window: Arc<Window>, graphics_api: GraphicsApi, camera: &Camera, lights: &[Light], instances: &[MeshInstance], irradiance_volumes: &[RendererIrradianceVolume]) -> Result<Self, String> {
        let (vertices, indices) = flatten_instances(instances)?;
        let (shadow_draw_ranges, shadow_caster_bounds) = shadow_draw_data(instances);
        if vertices.is_empty() || indices.is_empty() {
            return Err("renderer requires nonempty mesh instances".into());
        }

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
            let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU timestamp resolve"),
                size: u64::from(GPU_TIMESTAMP_COUNT) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU timestamp readback"),
                size: u64::from(GPU_TIMESTAMP_COUNT) * 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
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
        config.present_mode = if surface_capabilities.present_modes.contains(&wgpu::PresentMode::Immediate) { wgpu::PresentMode::Immediate } else { wgpu::PresentMode::AutoNoVsync };
        config.desired_maximum_frame_latency = 3;
        eprintln!("Renderer present mode: {:?}; GPU timestamps: {}", config.present_mode, gpu_timestamps.is_some());
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
        let shadow_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let shadow_uniform_buffers = std::array::from_fn(|cascade_index| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("shadow cascade uniform"),
                contents: bytemuck::bytes_of(&ShadowUniform { view_projection: shadow_matrices[cascade_index].to_cols_array_2d() }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        let shadow_bind_groups = std::array::from_fn(|cascade_index| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shadow cascade bind group"),
                layout: &shadow_bind_group_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_uniform_buffers[cascade_index].as_entire_binding() }],
            })
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shadow shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shadow_shader.wgsl").into()) });
        let spot_shadow_shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("spot shadow shader"), source: wgpu::ShaderSource::Wgsl(include_str!("spot_shadow_shader.wgsl").into()) });
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("shadow pipeline layout"), bind_group_layouts: &[Some(&shadow_bind_group_layout)], immediate_size: 0 });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Uint32],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Front), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 0, slope_scale: 1.0, clamp: 0.0 },
            }),
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
        let spot_shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spot shadow uniform"),
            contents: bytemuck::bytes_of(&SpotShadowUniform { view_projection: spot_shadow_matrix.to_cols_array_2d() }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let spot_shadow_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spot shadow layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let spot_shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spot shadow bind group"),
            layout: &spot_shadow_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: spot_shadow_uniform_buffer.as_entire_binding() }],
        });
        let point_shadow_uniform_buffers = (0..MAX_POINT_SHADOWS * POINT_SHADOW_FACE_COUNT as usize)
            .map(|face| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("point shadow uniform {face}")),
                    size: mem::size_of::<SpotShadowUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect::<Vec<_>>();
        let point_shadow_bind_groups = point_shadow_uniform_buffers
            .iter()
            .map(|buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("point shadow bind group"),
                    layout: &spot_shadow_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
                })
            })
            .collect::<Vec<_>>();
        let spot_shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spot shadow pipeline layout"),
            bind_group_layouts: &[Some(&spot_shadow_bind_group_layout)],
            immediate_size: 0,
        });
        let spot_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spot shadow pipeline"),
            layout: Some(&spot_shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &spot_shadow_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Uint32],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 0, slope_scale: 0.25, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let camera_uniform = camera_uniform(width, height, camera, lights, RenderDebugMode::default(), RenderSettings::default(), shadow_bounds);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera uniform"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sky_lighting = RendererSkyLighting::default();
        let visibility_bounds = geometry_bounds(instances);
        let visibility_grid = VisibilityGrid::from_bounds(&visibility_bounds);
        let sdf_geometries = sdf_geometries(instances);
        let sdf_clipmap = SdfClipmap::new(camera.transform.position, &sdf_geometries);
        let sdf_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("AABB SDF geometry and albedo clipmap"),
            size: wgpu::Extent3d { width: SDF_CLIPMAP_RESOLUTION[0], height: SDF_CLIPMAP_RESOLUTION[1], depth_or_array_layers: SDF_CLIPMAP_RESOLUTION[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sdf_view = sdf_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let sdf_emission_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("AABB SDF emission clipmap"),
            size: wgpu::Extent3d { width: SDF_CLIPMAP_RESOLUTION[0], height: SDF_CLIPMAP_RESOLUTION[1], depth_or_array_layers: SDF_CLIPMAP_RESOLUTION[2] },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let sdf_emission_view = sdf_emission_texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D3), ..Default::default() });
        let near_radiance_field = WorldRadianceField::new(camera.transform.position, RADIANCE_NEAR_FIELD_EXTENT, RADIANCE_FIELD_RESOLUTION);
        let far_radiance_field = WorldRadianceField::new(camera.transform.position, RADIANCE_FAR_FIELD_EXTENT, RADIANCE_FIELD_RESOLUTION);
        let radiance_field_uniform = RadianceFieldUniform::zeroed();
        let near_radiance_field_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("near world radiance field uniform"),
            contents: bytemuck::bytes_of(&radiance_field_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let far_radiance_field_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("far world radiance field uniform"),
            contents: bytemuck::bytes_of(&radiance_field_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let radiance_field_values = vec![[0.0_f32; 4]; RADIANCE_FIELD_RESOLUTION.iter().product::<u32>() as usize * RADIANCE_FIELD_DIRECTIONS];
        let near_radiance_field_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("near world radiance field directional lobes"),
            contents: bytemuck::cast_slice(&radiance_field_values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let far_radiance_field_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("far world radiance field directional lobes"),
            contents: bytemuck::cast_slice(&radiance_field_values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let radiance_field_trace_debug_values = vec![[[0.0_f32; 4]; 4]; RADIANCE_FIELD_RESOLUTION.iter().product::<u32>() as usize * RADIANCE_FIELD_DIRECTIONS * 2];
        let radiance_field_trace_debug_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("world radiance field trace diagnostics"),
            contents: bytemuck::cast_slice(&radiance_field_trace_debug_values),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let gi_gizmo_selection_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("selected GI radiance cell"),
            contents: bytemuck::cast_slice(&[4_u32, 1, 4, 0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let surface_probe_uniform = SurfaceProbeUniform {
            screen_resolution: [width.div_ceil(SURFACE_PROBE_STRIDE), height.div_ceil(SURFACE_PROBE_STRIDE), width, height],
            cache_settings: [SURFACE_PROBE_STRIDE, SURFACE_PROBE_CAPACITY, 0, 0],
        };
        let surface_probe_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("surface probe update settings"),
            contents: bytemuck::bytes_of(&surface_probe_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let surface_probe_data = vec![[0u32; 16]; SURFACE_PROBE_CAPACITY as usize];
        let surface_probe_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("persistent world surface probes"),
            contents: bytemuck::cast_slice(&surface_probe_data),
            usage: wgpu::BufferUsages::STORAGE,
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &sdf_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&sdf_clipmap.values),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SDF_CLIPMAP_RESOLUTION[0] * 16), rows_per_image: Some(SDF_CLIPMAP_RESOLUTION[1]) },
            wgpu::Extent3d { width: SDF_CLIPMAP_RESOLUTION[0], height: SDF_CLIPMAP_RESOLUTION[1], depth_or_array_layers: SDF_CLIPMAP_RESOLUTION[2] },
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &sdf_emission_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&sdf_clipmap.emission_values),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(SDF_CLIPMAP_RESOLUTION[0] * 16), rows_per_image: Some(SDF_CLIPMAP_RESOLUTION[1]) },
            wgpu::Extent3d { width: SDF_CLIPMAP_RESOLUTION[0], height: SDF_CLIPMAP_RESOLUTION[1], depth_or_array_layers: SDF_CLIPMAP_RESOLUTION[2] },
        );
        let sdf_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF clipmap uniform"),
            contents: bytemuck::bytes_of(&sdf_clipmap.uniform(false, sky_lighting)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let (static_probe_samples, dynamic_probe_samples) = build_probe_samples(instances, true);
        let static_probe_signature = static_probe_signature(instances);
        let initial_irradiance = Box::new(irradiance_uniform(irradiance_volumes, &static_probe_samples, &dynamic_probe_samples, lights, sky_lighting, &visibility_grid, true));
        let irradiance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("irradiance probes"),
            contents: bytemuck::bytes_of(initial_irradiance.as_ref()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let (depth_texture, depth_view) = create_depth_texture(&device, width, height);
        let (depth_sample_texture, depth_sample_view) = create_depth_sample_texture(&device, width, height);
        let (surface_normal_texture, surface_normal_view) = create_surface_normal_texture(&device, width, height);
        let (surface_id_texture, surface_id_view) = create_surface_id_texture(&device, width, height);
        let depth_sampler =
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("scene depth sampler"), mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, ..Default::default() });
        let depth_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene depth layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
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
            ],
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 15,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 20,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
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
                wgpu::BindGroupEntry { binding: 11, resource: near_radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: near_radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 13, resource: far_radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 14, resource: far_radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 15, resource: surface_probe_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 16, resource: surface_probe_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 20, resource: radiance_field_trace_debug_buffer.as_entire_binding() },
            ],
        });
        let gi_gizmo_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GI gizmo read-only layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 14,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 16,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 20,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 21,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let gi_gizmo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GI gizmo read-only resources"),
            layout: &gi_gizmo_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&sdf_view) },
                wgpu::BindGroupEntry { binding: 9, resource: sdf_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: near_radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: near_radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 13, resource: far_radiance_field_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 14, resource: far_radiance_field_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 16, resource: surface_probe_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 20, resource: radiance_field_trace_debug_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 21, resource: gi_gizmo_selection_buffer.as_entire_binding() },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("diffuse shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&depth_bind_group_layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Uint32],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout.clone())], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let radiance_field_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world radiance field compute pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout), Some(&depth_bind_group_layout)],
            immediate_size: 0,
        });
        let radiance_field_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("world radiance field update pipeline"),
            layout: Some(&radiance_field_pipeline_layout),
            module: &shader,
            entry_point: Some("update_world_radiance_field"),
            compilation_options: Default::default(),
            cache: None,
        });
        let surface_probe_update_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("screen seeded surface probe update pipeline"),
            layout: Some(&radiance_field_pipeline_layout),
            module: &shader,
            entry_point: Some("update_surface_probes"),
            compilation_options: Default::default(),
            cache: None,
        });
        let surface_probe_maintenance_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("surface probe aging pipeline"),
            layout: Some(&radiance_field_pipeline_layout),
            module: &shader,
            entry_point: Some("age_surface_probes"),
            compilation_options: Default::default(),
            cache: None,
        });
        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wireframe pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout.clone())], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let depth_shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("depth prepass shader"), source: wgpu::ShaderSource::Wgsl(include_str!("depth_prepass.wgsl").into()) });
        let depth_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("depth prepass pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout)], immediate_size: 0 });
        let depth_prepass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth prepass pipeline"),
            layout: Some(&depth_pipeline_layout),
            vertex: wgpu::VertexState { module: &depth_shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout.clone())], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &depth_shader,
                entry_point: Some("fragment_main"),
                targets: &[
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Uint, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                ],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let ui_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("UI shader"), source: wgpu::ShaderSource::Wgsl(include_str!("ui_shader.wgsl").into()) });
        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("UI pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &ui_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<UiVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &ui_shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let gizmo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gizmo shader"), source: wgpu::ShaderSource::Wgsl(include_str!("gizmo_shader.wgsl").into()) });
        let gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gizmo pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &gizmo_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<RendererGizmoVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &gizmo_shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let gi_gizmo_shader =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("GI cache gizmo shader"), source: wgpu::ShaderSource::Wgsl(include_str!("gi_gizmo.wgsl").into()) });
        let gi_gizmo_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GI cache gizmo pipeline layout"),
            bind_group_layouts: &[Some(&gi_gizmo_bind_group_layout)],
            immediate_size: 0,
        });
        let gi_gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GI cache gizmo pipeline"),
            layout: Some(&gi_gizmo_pipeline_layout),
            vertex: wgpu::VertexState { module: &gi_gizmo_shader, entry_point: Some("vertex_main"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &gi_gizmo_shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sdf_occupancy_gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("SDF occupancy gizmo pipeline"),
            layout: Some(&gi_gizmo_pipeline_layout),
            vertex: wgpu::VertexState { module: &gi_gizmo_shader, entry_point: Some("occupancy_vertex_main"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &gi_gizmo_shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
        let wireframe_indices = wireframe_indices(&indices);
        let wireframe_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wireframe indices"),
            contents: bytemuck::cast_slice(&wireframe_indices),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        });
        let ui_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI vertices"),
            contents: bytemuck::cast_slice(&[UiVertex { position: [0.0, 0.0], color: [0.0; 4] }; 6]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let gizmo_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gizmo vertices"),
            contents: bytemuck::cast_slice(&[RendererGizmoVertex { position: [0.0; 3], color: [0.0; 4] }; 2]),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        Ok(Self {
            window,
            instance,
            surface,
            device,
            queue,
            config,
            pipeline,
            radiance_field_pipeline,
            surface_probe_update_pipeline,
            surface_probe_maintenance_pipeline,
            depth_prepass_pipeline,
            wireframe_pipeline,
            gi_gizmo_pipeline,
            sdf_occupancy_gizmo_pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            mesh_topology_signature: mesh_topology_signature(instances),
            instance_signatures: instances.iter().map(mesh_instance_signature).collect(),
            wireframe_index_buffer,
            wireframe_index_count: wireframe_indices.len() as u32,
            camera_buffer,
            camera_bind_group,
            irradiance_buffer,
            irradiance_cache: initial_irradiance,
            irradiance_volume_layout: irradiance_volumes.to_vec(),
            sdf_clipmap,
            sdf_geometry_cache: sdf_geometries,
            sdf_uniform_buffer,
            _sdf_texture: sdf_texture,
            _sdf_emission_texture: sdf_emission_texture,
            near_radiance_field,
            far_radiance_field,
            near_radiance_field_uniform_buffer,
            far_radiance_field_uniform_buffer,
            _near_radiance_field_buffer: near_radiance_field_buffer,
            _far_radiance_field_buffer: far_radiance_field_buffer,
            _radiance_field_trace_debug_buffer: radiance_field_trace_debug_buffer,
            surface_probe_uniform_buffer,
            _surface_probe_buffer: surface_probe_buffer,
            sdf_gi_enabled: false,
            gi_radiance_frozen: false,
            irradiance_probe_ages: [0; MAX_IRRADIANCE_PROBES],
            irradiance_update_budget: 8,
            sky_lighting,
            visibility_grid,
            visibility_bounds,
            visibility_rebuild_cooldown: 0,
            static_probe_samples,
            dynamic_probe_samples,
            static_probe_signature,
            shadow_bounds,
            performance_stats: RendererPerformanceStats::default(),
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
            shadow_draw_ranges,
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
            surface_probe_gizmos_enabled: false,
            world_radiance_gizmos_enabled: false,
            world_radiance_trace_gizmos_enabled: false,
            selected_radiance_cell: [4, 1, 4],
            selected_radiance_field_far: false,
            sdf_occupancy_gizmos_enabled: false,
            render_debug_mode: RenderDebugMode::default(),
            render_settings: RenderSettings::default(),
            gpu_timestamps,
        })
    }

    pub fn update_instances(&mut self, instances: &[MeshInstance]) -> Result<(), String> {
        let update_start = Instant::now();
        if instances.is_empty() || instances.iter().any(|instance| instance.mesh.is_empty()) {
            return Err("renderer requires nonempty mesh instances".into());
        }
        let topology_signature = mesh_topology_signature(instances);
        let instance_signatures = instances.iter().map(mesh_instance_signature).collect::<Vec<_>>();
        let topology_changed = topology_signature != self.mesh_topology_signature
            || instances.iter().map(|instance| instance.mesh.indices().len()).sum::<usize>() as u32 != self.index_count
            || instances.iter().map(|instance| instance.mesh.vertices().len()).sum::<usize>() * mem::size_of::<RendererMeshVertex>() != self.vertex_buffer.size() as usize;
        let instances_changed = instance_signatures != self.instance_signatures;
        if !topology_changed && !instances_changed {
            self.performance_stats.instance_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
            return Ok(());
        }
        if topology_changed {
            let (vertices, indices) = flatten_instances(instances)?;
            self.vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
            self.index_buffer =
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
            self.index_count = indices.len() as u32;
            self.mesh_topology_signature = topology_signature;
            self.instance_signatures = instance_signatures;
            let wireframe_indices = wireframe_indices(&indices);
            self.wireframe_index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("wireframe indices"),
                contents: bytemuck::cast_slice(&wireframe_indices),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            });
            self.wireframe_index_count = wireframe_indices.len() as u32;
            (self.shadow_draw_ranges, self.shadow_caster_bounds) = shadow_draw_data(instances);
            self.shadow_bounds = combined_geometry_bounds(&geometry_bounds(instances));
            self.visibility_bounds.clear();
            self.visibility_rebuild_cooldown = 0;
        } else {
            let mut vertex_offset = 0_usize;
            for (instance_index, instance) in instances.iter().enumerate() {
                if instance_signatures[instance_index] != self.instance_signatures[instance_index] {
                    let vertices = flatten_instance(instance, instance_index);
                    let byte_offset = (vertex_offset * mem::size_of::<RendererMeshVertex>()) as u64;
                    self.queue.write_buffer(&self.vertex_buffer, byte_offset, bytemuck::cast_slice(&vertices));
                }
                vertex_offset += instance.mesh.vertices().len();
            }
            self.instance_signatures = instance_signatures;
            self.shadow_caster_bounds = geometry_bounds(instances);
            self.shadow_bounds = combined_geometry_bounds(&self.shadow_caster_bounds);
        }
        self.shadow_scheduler.mark_all_dirty();
        let signature = static_probe_signature(instances);
        if signature != self.static_probe_signature {
            let (static_samples, dynamic_samples) = build_probe_samples(instances, true);
            self.static_probe_samples = static_samples;
            self.dynamic_probe_samples = dynamic_samples;
            self.static_probe_signature = signature;
        } else {
            self.dynamic_probe_samples = build_dynamic_probe_samples(instances);
        }
        self.sdf_geometry_cache = sdf_geometries(instances);
        self.performance_stats.instance_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
        Ok(())
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
            ],
        });
        let surface_probe_uniform = SurfaceProbeUniform {
            screen_resolution: [width.div_ceil(SURFACE_PROBE_STRIDE), height.div_ceil(SURFACE_PROBE_STRIDE), width, height],
            cache_settings: [SURFACE_PROBE_STRIDE, SURFACE_PROBE_CAPACITY, 0, self.packed_surface_probe_frame_index()],
        };
        self.queue.write_buffer(&self.surface_probe_uniform_buffer, 0, bytemuck::bytes_of(&surface_probe_uniform));
        let camera_uniform = camera_uniform(width, height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));
    }

    pub fn update_camera(&mut self, camera: &Camera) -> Result<(), String> {
        self.camera = *camera;
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
            self.queue.write_buffer(&self.shadow_uniform_buffers[cascade_index], 0, bytemuck::bytes_of(&ShadowUniform { view_projection: matrix.to_cols_array_2d() }));
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
        let previous_lights = self.lights.clone();
        if previous_lights != lights {
            self.near_radiance_field.refresh_temporally();
            self.far_radiance_field.refresh_temporally();
        }
        self.lights.clear();
        self.lights.extend_from_slice(lights);
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
            self.queue.write_buffer(&self.shadow_uniform_buffers[cascade_index], 0, bytemuck::bytes_of(&ShadowUniform { view_projection: matrix.to_cols_array_2d() }));
        }
        let spot_shadow_matrix = self
            .lights
            .iter()
            .find_map(|light| match light {
                Light::Spot(light) => Some(spot_shadow_view_projection(light)),
                _ => None,
            })
            .unwrap_or(Mat4::IDENTITY);
        self.queue.write_buffer(&self.spot_shadow_uniform_buffer, 0, bytemuck::bytes_of(&SpotShadowUniform { view_projection: spot_shadow_matrix.to_cols_array_2d() }));
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        self.performance_stats.light_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
        Ok(())
    }

    pub fn update_irradiance_volumes(&mut self, volumes: &[RendererIrradianceVolume], instances: &[MeshInstance], lights: &[Light], sdf_gi_enabled: bool) {
        if sdf_gi_enabled != self.sdf_gi_enabled {
            self.sdf_gi_enabled = sdf_gi_enabled;
            self.near_radiance_field.invalidate();
            self.far_radiance_field.invalidate();
        }
        self.performance_stats.sdf_build_cpu_ms = 0.0;
        self.performance_stats.sdf_upload_ms = 0.0;
        self.performance_stats.sdf_voxels_updated = 0;
        self.performance_stats.gi_history_valid_percent = 0.0;
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
            let sdf_update = self.sdf_clipmap.update(self.camera.transform.position, &self.sdf_geometry_cache);
            self.performance_stats.sdf_build_cpu_ms = sdf_build_start.elapsed().as_secs_f32() * 1000.0;
            if geometry_changed {
                self.near_radiance_field.refresh_temporally();
                self.far_radiance_field.refresh_temporally();
            }
            if let Some(sdf_update) = sdf_update {
                self.performance_stats.sdf_voxels_updated = sdf_update.voxels_updated;
                let sdf_upload_start = Instant::now();
                for region in sdf_update.texture_regions {
                    upload_sdf_region(&self.queue, &self._sdf_texture, &self.sdf_clipmap.values, region);
                }
                upload_sdf_region(&self.queue, &self._sdf_emission_texture, &self.sdf_clipmap.emission_values, SdfTextureRegion { origin: [0; 3], size: SDF_CLIPMAP_RESOLUTION });
                self.performance_stats.sdf_upload_ms = sdf_upload_start.elapsed().as_secs_f32() * 1000.0;
            }
            let near_radiance_field_uniform = self.near_radiance_field.uniform(self.camera.transform.position);
            let far_radiance_field_uniform = self.far_radiance_field.uniform(self.camera.transform.position);
            self.queue.write_buffer(&self.near_radiance_field_uniform_buffer, 0, bytemuck::bytes_of(&near_radiance_field_uniform));
            self.queue.write_buffer(&self.far_radiance_field_uniform_buffer, 0, bytemuck::bytes_of(&far_radiance_field_uniform));
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
            self.irradiance_probe_ages.fill(u32::MAX);
            self.visibility_rebuild_cooldown = VISIBILITY_REBUILD_INTERVAL;
            visibility_rebuild_ms = visibility_start.elapsed().as_secs_f32() * 1000.0;
        }
        if volumes != self.irradiance_volume_layout.as_slice() {
            let old_cache = self.irradiance_cache.clone();
            let old_ages = self.irradiance_probe_ages;
            let mut new_cache = Box::new(irradiance_uniform(volumes, &self.static_probe_samples, &self.dynamic_probe_samples, lights, self.sky_lighting, &self.visibility_grid, false));
            let mut new_ages = [u32::MAX; MAX_IRRADIANCE_PROBES];
            preserve_overlapping_probes(&old_cache, &old_ages, &mut new_cache, &mut new_ages);
            self.irradiance_cache = new_cache;
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
                    self.irradiance_cache.probe_positions[index] = [location.position.x, location.position.y, location.position.z, 1.0];
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
                self.irradiance_cache.probe_positions[index] = [location.position.x, location.position.y, location.position.z, 1.0];
                let position = location.position;
                let (color, sky_visibility, direction) = probe_irradiance(position, &self.static_probe_samples, &self.dynamic_probe_samples, lights, &self.visibility_grid, self.sky_lighting);
                let blend = if self.irradiance_probe_ages[index] == u32::MAX { 1.0 } else { 0.25 };
                let previous_color = Vec3::new(self.irradiance_cache.probes[index][0], self.irradiance_cache.probes[index][1], self.irradiance_cache.probes[index][2]);
                let blended_color = previous_color.lerp(color, blend);
                let previous_visibility = self.irradiance_cache.probes[index][3];
                let blended_visibility = previous_visibility + (sky_visibility - previous_visibility) * blend;
                self.irradiance_cache.probes[index] = [blended_color.x, blended_color.y, blended_color.z, blended_visibility];
                let previous_direction = Vec4::from_array(self.irradiance_cache.directions[index]);
                let blended_direction = previous_direction.lerp(direction, blend);
                self.irradiance_cache.directions[index] = blended_direction.to_array();
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
            sdf_upload_ms: self.performance_stats.sdf_upload_ms,
            sdf_voxels_updated: self.performance_stats.sdf_voxels_updated,
            gi_history_valid_percent: self.performance_stats.gi_history_valid_percent,
            gpu_shadow_ms: self.performance_stats.gpu_shadow_ms,
            gpu_scene_ms: self.performance_stats.gpu_scene_ms,
            gpu_sdf_gi_ms: self.performance_stats.gpu_sdf_gi_ms,
            gpu_gizmo_ms: self.performance_stats.gpu_gizmo_ms,
            gpu_ui_ms: self.performance_stats.gpu_ui_ms,
            gpu_total_ms: self.performance_stats.gpu_total_ms,
            scheduled_shadow_lights: self.performance_stats.scheduled_shadow_lights,
            dirty_shadow_lights: self.performance_stats.dirty_shadow_lights,
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
        let sdf_gi_ms = if self.sdf_gi_enabled && self.render_settings.irradiance_enabled { to_ms(4, 5) } else { 0.0 };
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
        self.performance_stats.gpu_total_ms = shadow_ms + scene_ms + sdf_gi_ms + gizmo_ms + ui_ms;
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
        self.near_radiance_field.refresh_temporally();
        self.far_radiance_field.refresh_temporally();
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, sky_lighting)));
    }

    pub fn update_render_debug_mode(&mut self, mode: RenderDebugMode) {
        self.render_debug_mode = mode;
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
    }

    pub fn update_render_settings(&mut self, settings: RenderSettings) {
        if self.render_settings == settings {
            return;
        }
        let cascade_count_changed = self.render_settings.shadow_cascade_count != settings.shadow_cascade_count;
        let gi_reenabled = !self.render_settings.irradiance_enabled && settings.irradiance_enabled && self.sdf_gi_enabled;
        self.render_settings = settings;
        if gi_reenabled {
            self.near_radiance_field.invalidate();
            self.far_radiance_field.invalidate();
        }
        self.shadow_scheduler.rebuild(&self.lights, &self.camera, settings.shadow_budget);
        self.rebuild_shadow_resources();
        self.update_shadow_scheduler_stats();
        if cascade_count_changed {
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
                self.queue.write_buffer(&self.shadow_uniform_buffers[cascade_index], 0, bytemuck::bytes_of(&ShadowUniform { view_projection: matrix.to_cols_array_2d() }));
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
            self.ui_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("UI vertices"),
                size: (self.ui_vertex_capacity * mem::size_of::<UiVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
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
            self.gizmo_vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gizmo vertices"),
                size: (self.gizmo_vertex_capacity * mem::size_of::<RendererGizmoVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue.write_buffer(&self.gizmo_vertex_buffer, 0, bytemuck::cast_slice(vertices));
        self.gizmo_vertex_count = vertices.len() as u32;
    }

    pub fn set_surface_probe_gizmos_enabled(&mut self, enabled: bool) {
        self.surface_probe_gizmos_enabled = enabled;
    }

    pub fn set_gi_radiance_frozen(&mut self, frozen: bool) {
        self.gi_radiance_frozen = frozen;
    }

    fn packed_surface_probe_frame_index(&self) -> u32 {
        (self.frame_index & 0x7fff_ffff) | if self.gi_radiance_frozen { 0x8000_0000 } else { 0 }
    }

    pub fn set_world_radiance_gizmos_enabled(&mut self, enabled: bool) {
        self.world_radiance_gizmos_enabled = enabled;
    }

    pub fn set_world_radiance_trace_gizmos_enabled(&mut self, enabled: bool) {
        self.world_radiance_trace_gizmos_enabled = enabled;
    }

    pub fn set_selected_radiance_cell(&mut self, cell: [u32; 3], far_field: bool) {
        self.selected_radiance_cell = [cell[0].min(7), cell[1].min(3), cell[2].min(7)];
        self.selected_radiance_field_far = far_field;
        let selection = [self.selected_radiance_cell[0], self.selected_radiance_cell[1], self.selected_radiance_cell[2], u32::from(far_field)];
        self.queue.write_buffer(&self.gi_gizmo_selection_buffer, 0, bytemuck::cast_slice(&selection));
    }

    pub fn set_sdf_occupancy_gizmos_enabled(&mut self, enabled: bool) {
        self.sdf_occupancy_gizmos_enabled = enabled;
    }

    pub fn capture(&mut self, camera: &Camera, width: u32, height: u32) -> Result<CapturedFrame, String> {
        let width = width.max(1);
        let height = height.max(1);
        let format = self.config.format;
        if !matches!(format, wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            return Err(format!("capture does not support surface format {format:?}"));
        }

        let color_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture color target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (depth_texture, depth_view) = create_depth_texture(&self.device, width, height);
        let (depth_sample_texture, depth_sample_view) = create_depth_sample_texture(&self.device, width, height);
        let (_surface_normal_texture, surface_normal_view) = create_surface_normal_texture(&self.device, width, height);
        let (_surface_id_texture, surface_id_view) = create_surface_id_texture(&self.device, width, height);
        let capture_depth_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("capture depth bind group"),
            layout: &self.depth_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth_sample_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self._depth_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&surface_normal_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&surface_id_view) },
            ],
        });
        let capture_uniform = camera_uniform(width, height, camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&capture_uniform));
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));
        let capture_probe_uniform = SurfaceProbeUniform {
            screen_resolution: [width.div_ceil(SURFACE_PROBE_STRIDE), height.div_ceil(SURFACE_PROBE_STRIDE), width, height],
            cache_settings: [SURFACE_PROBE_STRIDE, SURFACE_PROBE_CAPACITY, 0, self.packed_surface_probe_frame_index()],
        };
        self.queue.write_buffer(&self.surface_probe_uniform_buffer, 0, bytemuck::bytes_of(&capture_probe_uniform));

        let bytes_per_pixel = 4_u32;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback_size = u64::from(padded_bytes_per_row) * u64::from(height);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let scene_depth_readback = create_depth_readback_buffer(&self.device, "capture scene depth", width, height);
        let has_directional_light = self.lights.iter().any(|light| matches!(light, Light::Directional(_)));
        let has_spot_light = self.lights.iter().any(|light| matches!(light, Light::Spot(_)));
        let point_light_count = self.lights.iter().filter(|light| matches!(light, Light::Point(_))).count().min(MAX_POINT_SHADOWS);
        let directional_depth_readbacks = has_directional_light.then(|| {
            (0..SHADOW_CASCADE_COUNT).map(|cascade| create_depth_readback_buffer(&self.device, &format!("capture directional cascade {cascade}"), SHADOW_MAP_SIZE, SHADOW_MAP_SIZE)).collect::<Vec<_>>()
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
        if self.sdf_gi_enabled && self.render_settings.irradiance_enabled {
            self.render_world_radiance_field_pass(&mut encoder, &capture_depth_bind_group, width, height);
        }
        self.render_scene_pass(&mut encoder, &color_view, &depth_view, &capture_depth_bind_group);
        encoder.pop_debug_group();
        if self.gizmo_vertex_count > 0 {
            self.render_gizmo_pass(&mut encoder, &color_view, &depth_view, &capture_depth_bind_group);
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
        let spot_shadow = CapturedDepthBuffer {
            width: spot_size,
            height: spot_size,
            layers: spot_depth_readback.as_ref().map(|buffer| read_depth_buffer(&self.device, buffer, spot_size, spot_size)).transpose()?.into_iter().collect(),
        };
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
        Ok(CapturedFrame { width, height, rgba8, scene_depth, directional_shadow, spot_shadow, point_shadow })
    }

    pub fn render(&mut self) -> Result<(), String> {
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
        let probe_uniform = SurfaceProbeUniform {
            screen_resolution: [self.config.width.div_ceil(SURFACE_PROBE_STRIDE), self.config.height.div_ceil(SURFACE_PROBE_STRIDE), self.config.width, self.config.height],
            cache_settings: [SURFACE_PROBE_STRIDE, SURFACE_PROBE_CAPACITY, 0, self.packed_surface_probe_frame_index()],
        };
        self.queue.write_buffer(&self.surface_probe_uniform_buffer, 0, bytemuck::bytes_of(&probe_uniform));
        self.frame_index = self.frame_index.wrapping_add(1);
        self.queue.write_buffer(&self.sdf_uniform_buffer, 0, bytemuck::bytes_of(&self.sdf_clipmap.uniform(self.sdf_gi_enabled, self.sky_lighting)));
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
            self.render_world_radiance_field_pass(&mut encoder, &self.depth_bind_group, self.config.width, self.config.height);
        }
        self.render_scene_pass(&mut encoder, &color_view, &self.depth_view, &self.depth_bind_group);
        encoder.pop_debug_group();
        if self.gizmo_vertex_count > 0 {
            self.render_gizmo_pass(&mut encoder, &color_view, &self.depth_view, &self.depth_bind_group);
        }
        if self.should_draw_gi_gizmos() {
            self.render_gi_gizmo_pass(&mut encoder, &color_view, &self.depth_view);
        }
        if self.ui_vertex_count > 0 {
            self.render_ui_pass(&mut encoder, &color_view);
        }
        self.resolve_gpu_timestamps(&mut encoder);
        self.queue.submit(Some(encoder.finish()));
        if self.sdf_gi_enabled && self.render_settings.irradiance_enabled {
            self.performance_stats.gi_history_valid_percent = 100.0;
        }
        self.queue.present(frame);
        if !self.has_presented {
            eprintln!("First frame presented");
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
                Some(wgpu::RenderPassColorAttachment {
                    view: normal_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: surface_id_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.depth_prepass_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    fn render_scene_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView, depth_bind_group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(self.camera.clear_color.red),
                        g: f64::from(self.camera.clear_color.green),
                        b: f64::from(self.camera.clear_color.blue),
                        a: f64::from(self.camera.clear_color.alpha),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(6),
                end_of_pass_write_index: Some(7),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, depth_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        if self.render_debug_mode == RenderDebugMode::Wireframe {
            pass.set_pipeline(&self.wireframe_pipeline);
            pass.set_index_buffer(self.wireframe_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.wireframe_index_count, 0, 0..1);
        } else {
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }

    fn render_world_radiance_field_pass(&self, encoder: &mut wgpu::CommandEncoder, depth_bind_group: &wgpu::BindGroup, width: u32, height: u32) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("world radiance field update"),
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::ComputePassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(4),
                end_of_pass_write_index: Some(5),
            }),
        });
        pass.set_pipeline(&self.radiance_field_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, depth_bind_group, &[]);
        pass.dispatch_workgroups(RADIANCE_FIELD_RESOLUTION[0].div_ceil(4), RADIANCE_FIELD_RESOLUTION[1].div_ceil(4), RADIANCE_FIELD_RESOLUTION[2].div_ceil(4));
        pass.set_pipeline(&self.surface_probe_update_pipeline);
        pass.dispatch_workgroups(width.div_ceil(SURFACE_PROBE_STRIDE * 8), height.div_ceil(SURFACE_PROBE_STRIDE * 8), 1);
        pass.set_pipeline(&self.surface_probe_maintenance_pipeline);
        pass.dispatch_workgroups(SURFACE_PROBE_CAPACITY.div_ceil(64), 1, 1);
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
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: shadow_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: if cascade_index == 0 {
                    self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                        query_set: &timestamps.query_set,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    })
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
            for (draw_range, bounds) in self.shadow_draw_ranges.iter().zip(&self.shadow_caster_bounds) {
                if shadow_bounds_intersect(self.shadow_matrices[cascade_index], *bounds) {
                    pass.draw_indexed(draw_range.clone(), 0, 0..1);
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
            self.queue.write_buffer(&self.spot_shadow_uniform_buffer, 0, bytemuck::bytes_of(&SpotShadowUniform { view_projection: spot_shadow_view_projection(light).to_cols_array_2d() }));
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spot shadow atlas pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.spot_shadow_view,
                    depth_ops: Some(wgpu::Operations { load: if rendered == 0 { wgpu::LoadOp::Clear(1.0) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: if rendered == 0 {
                    self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                        query_set: &timestamps.query_set,
                        beginning_of_pass_write_index: Some(0),
                        end_of_pass_write_index: Some(1),
                    })
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
            pass.draw_indexed(0..self.index_count, 0, 0..1);
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
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.spot_shadow_pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            self.shadow_scheduler.mark_clean(request.light_index as usize);
        }
        self.update_shadow_scheduler_stats();
    }

    fn render_ui_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(10),
                end_of_pass_write_index: Some(11),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.ui_pipeline);
        pass.set_vertex_buffer(0, self.ui_vertex_buffer.slice(..));
        pass.draw(0..self.ui_vertex_count, 0..1);
    }

    fn render_gizmo_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView, depth_bind_group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gizmo pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(8),
                end_of_pass_write_index: Some(9),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.gizmo_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, depth_bind_group, &[]);
        pass.set_vertex_buffer(0, self.gizmo_vertex_buffer.slice(..));
        pass.draw(0..self.gizmo_vertex_count, 0..1);
    }

    fn render_gi_gizmo_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("GI cache gizmo pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.gi_gizmo_pipeline);
        pass.set_bind_group(0, &self.gi_gizmo_bind_group, &[]);
        let surface_probe_count = SURFACE_PROBE_CAPACITY;
        let radiance_field_count = RADIANCE_FIELD_RESOLUTION.iter().product::<u32>() * 2;
        if self.surface_probe_gizmos_enabled {
            pass.draw(0..8, 0..surface_probe_count);
        }
        if self.world_radiance_gizmos_enabled {
            pass.draw(0..8, surface_probe_count..surface_probe_count + radiance_field_count);
        }
        if self.world_radiance_trace_gizmos_enabled {
            let trace_start = surface_probe_count + radiance_field_count;
            pass.draw(0..28, trace_start..trace_start + RADIANCE_FIELD_DIRECTIONS as u32);
            pass.draw(0..2, trace_start + RADIANCE_FIELD_DIRECTIONS as u32..trace_start + RADIANCE_FIELD_DIRECTIONS as u32 + 12);
        }
        if self.sdf_occupancy_gizmos_enabled {
            pass.set_pipeline(&self.sdf_occupancy_gizmo_pipeline);
            pass.draw(0..36, 0..SDF_CLIPMAP_RESOLUTION.iter().product());
        }
    }

    fn should_draw_gi_gizmos(&self) -> bool {
        let has_debug_gizmos = self.surface_probe_gizmos_enabled || self.world_radiance_gizmos_enabled || self.world_radiance_trace_gizmos_enabled || self.sdf_occupancy_gizmos_enabled;
        has_debug_gizmos && self.sdf_gi_enabled && self.render_settings.irradiance_enabled
    }
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
                    spot_shadow_rects[spot_slot] = [
                        spot_resolution as f32 / atlas_size as f32,
                        spot_resolution as f32 / atlas_size as f32,
                        cell as f32 * spot_resolution as f32 / atlas_size as f32,
                        row as f32 * spot_resolution as f32 / atlas_size as f32,
                    ];
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
            },
            u32::from(settings.shadows_enabled),
            u32::from(settings.irradiance_enabled),
            settings.shadow_cascade_count.clamp(1, SHADOW_CASCADE_COUNT as u32),
        ],
        shadow_settings: [
            settings.shadow_depth_bias.max(0.0),
            settings.shadow_normal_bias.max(0.0),
            settings.shadow_filter_radius.max(0.25),
            settings.shadow_resolution.clamp(256, SHADOW_MAP_SIZE) as f32,
        ],
        viewport_size: [width as f32, height as f32, 0.0, 0.0],
        shadow_flags: [
            u32::from(settings.contact_shadows_enabled),
            u32::from(settings.secondary_light_shadowing),
            u32::from(settings.ambient_occlusion_enabled),
            u32::from(settings.direct_light_visibility_enabled),
        ],
        ambient_light: [settings.ambient_intensity.max(0.0), 0.0, 0.0, 0.0],
        gi_settings: [u32::from(settings.surface_probes_enabled), 0, 0, 0],
    }
}

fn shadow_geometry_changed(previous: &[Light], current: &[Light]) -> bool {
    if previous.len() != current.len() {
        return true;
    }
    previous.iter().zip(current).any(|(previous, current)| match (previous, current) {
        (Light::Directional(previous), Light::Directional(current)) => previous.direction != current.direction,
        (Light::Point(previous), Light::Point(current)) => previous.position != current.position || previous.range != current.range,
        (Light::Spot(previous), Light::Spot(current)) => {
            previous.position != current.position
                || previous.direction != current.direction
                || previous.range != current.range
                || previous.inner_angle != current.inner_angle
                || previous.outer_angle != current.outer_angle
        }
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

fn mesh_topology_signature(instances: &[MeshInstance]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    instances.len().hash(&mut hasher);
    for instance in instances {
        instance.mesh.vertices().len().hash(&mut hasher);
        instance.mesh.indices().hash(&mut hasher);
    }
    hasher.finish()
}

fn mesh_instance_signature(instance: &MeshInstance) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytemuck::cast_slice::<_, u8>(instance.mesh.vertices()).hash(&mut hasher);
    instance.mesh.indices().hash(&mut hasher);
    instance.transform.position.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.transform.rotation.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.transform.scale.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.material.base_color.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.material.metallic.to_bits().hash(&mut hasher);
    instance.material.roughness.to_bits().hash(&mut hasher);
    instance.material.emission.to_array().map(|value| value.to_bits()).hash(&mut hasher);
    instance.material.emission_strength.to_bits().hash(&mut hasher);
    instance.probe_dynamic.hash(&mut hasher);
    hasher.finish()
}

fn flatten_instances(instances: &[MeshInstance]) -> Result<(Vec<RendererMeshVertex>, Vec<u32>), String> {
    if instances.is_empty() || instances.iter().any(|instance| instance.mesh.is_empty()) {
        return Err("renderer requires nonempty mesh instances".into());
    }

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut vertex_offset = 0_u32;
    for (instance_index, instance) in instances.iter().enumerate() {
        vertices.extend(flatten_instance(instance, instance_index));
        indices.extend(instance.mesh.indices().iter().map(|index| index + vertex_offset));
        vertex_offset += instance.mesh.vertices().len() as u32;
    }
    Ok((vertices, indices))
}

fn flatten_instance(instance: &MeshInstance, instance_index: usize) -> Vec<RendererMeshVertex> {
    let transform = instance.transform;
    instance
        .mesh
        .vertices()
        .iter()
        .map(|vertex| {
            let position = transform.transform_point(Vec3::from_array(vertex.position));
            let normal = transform.transform_vector(Vec3::from_array(vertex.normal)).normalize_or_zero();
            RendererMeshVertex {
                position: position.to_array(),
                normal: normal.to_array(),
                color: vertex.color,
                material_base_color: instance.material.base_color.to_array(),
                material_params: [instance.material.metallic.clamp(0.0, 1.0), instance.material.roughness.clamp(0.04, 1.0), 0.5, instance.material.emission_strength.max(0.0)],
                material_emission: instance.material.emission.to_array(),
                surface_id: instance_index as u32 + 1,
            }
        })
        .collect()
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
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: u64::from(layout.bytes_per_row.unwrap_or_default()) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    })
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

fn create_depth_sample_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth sample texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("directional cascade array"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        array_layer_count: Some(SHADOW_CASCADE_COUNT as u32),
        ..Default::default()
    });
    let views = std::array::from_fn(|cascade_index| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("directional shadow cascade"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: cascade_index as u32,
            array_layer_count: Some(1),
            ..Default::default()
        })
    });
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
    let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("point shadow depth array"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        array_layer_count: Some((MAX_POINT_SHADOWS as u32) * POINT_SHADOW_FACE_COUNT),
        ..Default::default()
    });
    let views = (0..MAX_POINT_SHADOWS * POINT_SHADOW_FACE_COUNT as usize)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("point shadow face"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    (texture, array_view, views)
}

fn irradiance_uniform(
    volumes: &[RendererIrradianceVolume],
    static_samples: &[ProbeSurfaceSample],
    dynamic_samples: &[ProbeSurfaceSample],
    lights: &[Light],
    sky_lighting: RendererSkyLighting,
    visibility_grid: &VisibilityGrid,
    evaluate_probes: bool,
) -> IrradianceUniform {
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
        uniform.volumes[volume_index] = IrradianceVolumeUniform {
            minimum: [volume.minimum[0], volume.minimum[1], volume.minimum[2], 0.0],
            maximum: [volume.maximum[0], volume.maximum[1], volume.maximum[2], 0.0],
            resolution: [resolution[0], resolution[1], resolution[2], 0],
            probe_offset: probe_offset as u32,
            _padding: [0; 7],
        };
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
    ((position[0] * 1000.0).round() as i32, (position[1] * 1000.0).round() as i32, (position[2] * 1000.0).round() as i32, position[3].round() as i32)
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
    fn new(camera_position: Vec3, geometry_bounds: &[SdfGeometry]) -> Self {
        let voxel_size = SDF_CLIPMAP_EXTENT / Vec3::new(SDF_CLIPMAP_RESOLUTION[0] as f32, SDF_CLIPMAP_RESOLUTION[1] as f32, SDF_CLIPMAP_RESOLUTION[2] as f32);
        let minimum = Self::snapped_minimum(camera_position, voxel_size);
        let maximum = minimum + SDF_CLIPMAP_EXTENT;
        let mut clipmap =
            Self { minimum, maximum, voxel_size, values: Vec::new(), emission_values: Vec::new(), geometry_bounds: Vec::new(), geometry_bvh: SdfGeometryBvh::new(&[]), grid_offset: [0; 3] };
        clipmap.rebuild(geometry_bounds);
        clipmap
    }

    fn update(&mut self, camera_position: Vec3, geometry_bounds: &[SdfGeometry]) -> Option<SdfClipmapUpdate> {
        let minimum = Self::snapped_minimum(camera_position, self.voxel_size);
        let geometry_changed = self.geometry_bounds != geometry_bounds;
        if self.minimum == minimum && !geometry_changed {
            return None;
        }
        let shift_vector = (minimum - self.minimum) / self.voxel_size;
        let shift = [shift_vector.x.round() as i32, shift_vector.y.round() as i32, shift_vector.z.round() as i32];
        self.minimum = minimum;
        self.maximum = minimum + SDF_CLIPMAP_EXTENT;
        if geometry_changed || shift.iter().enumerate().any(|(axis, value)| value.unsigned_abs() >= SDF_CLIPMAP_RESOLUTION[axis]) {
            self.grid_offset = [0; 3];
            self.rebuild(geometry_bounds);
            return Some(SdfClipmapUpdate { texture_regions: vec![SdfTextureRegion { origin: [0; 3], size: SDF_CLIPMAP_RESOLUTION }], voxels_updated: SDF_CLIPMAP_RESOLUTION.iter().product() });
        }

        for axis in 0..3 {
            self.grid_offset[axis] = (self.grid_offset[axis] as i32 + shift[axis]).rem_euclid(SDF_CLIPMAP_RESOLUTION[axis] as i32) as u32;
        }
        let logical_regions = scrolling_regions(shift, SDF_CLIPMAP_RESOLUTION);
        let mut texture_regions = Vec::new();
        let mut voxels_updated = 0;
        for region in logical_regions {
            self.rebuild_logical_region(region);
            voxels_updated += region.size.iter().product::<u32>();
            texture_regions.extend(self.texture_regions_for(region));
        }
        self.rebuild_conservative_clearance();
        Some(SdfClipmapUpdate { texture_regions, voxels_updated })
    }

    fn uniform(&self, enabled: bool, sky_lighting: RendererSkyLighting) -> SdfClipmapUniform {
        SdfClipmapUniform {
            minimum: self.minimum.extend(0.0).to_array(),
            maximum: self.maximum.extend(0.0).to_array(),
            resolution: [SDF_CLIPMAP_RESOLUTION[0], SDF_CLIPMAP_RESOLUTION[1], SDF_CLIPMAP_RESOLUTION[2], 0],
            settings: [u32::from(enabled), 0, 0, 0],
            grid_offset: [self.grid_offset[0], self.grid_offset[1], self.grid_offset[2], 0],
            sky_radiance: [sky_lighting.color.red * sky_lighting.intensity, sky_lighting.color.green * sky_lighting.intensity, sky_lighting.color.blue * sky_lighting.intensity, 0.0],
        }
    }

    fn snapped_minimum(camera_position: Vec3, voxel_size: Vec3) -> Vec3 {
        let snapped_center =
            Vec3::new((camera_position.x / voxel_size.x).floor() * voxel_size.x, (camera_position.y / voxel_size.y).floor() * voxel_size.y, (camera_position.z / voxel_size.z).floor() * voxel_size.z);
        Vec3::new(snapped_center.x - SDF_CLIPMAP_EXTENT.x * 0.5, snapped_center.y - SDF_CLIPMAP_EXTENT.y * 0.875, snapped_center.z - SDF_CLIPMAP_EXTENT.z * 0.5)
    }

    fn rebuild(&mut self, geometry_bounds: &[SdfGeometry]) {
        // This field represents the union of per-instance AABBs, not the source mesh surfaces.
        self.geometry_bounds.clear();
        self.geometry_bounds.extend_from_slice(geometry_bounds);
        self.geometry_bvh = SdfGeometryBvh::new(geometry_bounds);
        let voxel_count = SDF_CLIPMAP_RESOLUTION.iter().product::<u32>() as usize;
        self.values = vec![[SDF_MAX_DISTANCE, 0.0, 0.0, 0.0]; voxel_count];
        self.emission_values = vec![[0.0; 4]; voxel_count];
        self.rebuild_all_voxels();
        self.rebuild_conservative_clearance();
    }

    fn rebuild_conservative_clearance(&mut self) {
        let resolution = SDF_CLIPMAP_RESOLUTION;
        let voxel_count = resolution.iter().product::<u32>() as usize;
        let mut clearance = vec![u16::MAX; voxel_count];
        let grid_offset = self.grid_offset;
        let index = |x: u32, y: u32, z: u32| (x + y * resolution[0] + z * resolution[0] * resolution[1]) as usize;
        let physical_index = |x: u32, y: u32, z: u32| {
            let px = (x + grid_offset[0]) % resolution[0];
            let py = (y + grid_offset[1]) % resolution[1];
            let pz = (z + grid_offset[2]) % resolution[2];
            index(px, py, pz)
        };
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
                    let distance = clearance[index(x, y, z)];
                    let safe_distance = if distance == u16::MAX { SDF_MAX_DISTANCE } else { (distance as f32 * minimum_voxel_size - half_voxel_diagonal).max(0.0) };
                    self.emission_values[physical_index(x, y, z)][3] = safe_distance;
                }
            }
        }
    }

    fn rebuild_all_voxels(&mut self) {
        let row_width = SDF_CLIPMAP_RESOLUTION[0] as usize;
        let row_count = SDF_CLIPMAP_RESOLUTION[1] as usize * SDF_CLIPMAP_RESOLUTION[2] as usize;
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
                        let y = row % SDF_CLIPMAP_RESOLUTION[1] as usize;
                        let z = row / SDF_CLIPMAP_RESOLUTION[1] as usize;
                        let position = minimum + Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) * voxel_size;
                        let (distance, albedo, emission) = geometry_bvh.nearest(position, voxel_size * 0.5);
                        values[local_index] = [distance.clamp(-SDF_MAX_DISTANCE, SDF_MAX_DISTANCE), albedo.x, albedo.y, albedo.z];
                        emissions[local_index] = [emission.x, emission.y, emission.z, 0.0];
                    }
                });
            }
        });
    }

    fn rebuild_logical_region(&mut self, region: SdfLogicalRegion) {
        for z in region.start[2]..region.start[2] + region.size[2] {
            for y in region.start[1]..region.start[1] + region.size[1] {
                for x in region.start[0]..region.start[0] + region.size[0] {
                    let position = self.minimum + Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5) * self.voxel_size;
                    let physical_x = (x + self.grid_offset[0]) % SDF_CLIPMAP_RESOLUTION[0];
                    let physical_y = (y + self.grid_offset[1]) % SDF_CLIPMAP_RESOLUTION[1];
                    let physical_z = (z + self.grid_offset[2]) % SDF_CLIPMAP_RESOLUTION[2];
                    let index = (physical_x + physical_y * SDF_CLIPMAP_RESOLUTION[0] + physical_z * SDF_CLIPMAP_RESOLUTION[0] * SDF_CLIPMAP_RESOLUTION[1]) as usize;
                    let (nearest_distance, albedo, emission) = self.geometry_bvh.nearest(position, self.voxel_size * 0.5);
                    self.values[index] = [nearest_distance.clamp(-SDF_MAX_DISTANCE, SDF_MAX_DISTANCE), albedo.x, albedo.y, albedo.z];
                    self.emission_values[index] = [emission.x, emission.y, emission.z, 0.0];
                }
            }
        }
    }

    fn texture_regions_for(&self, region: SdfLogicalRegion) -> Vec<SdfTextureRegion> {
        let axes = (0..3).map(|axis| split_wrapped_range(region.start[axis], region.size[axis], self.grid_offset[axis], SDF_CLIPMAP_RESOLUTION[axis])).collect::<Vec<_>>();
        let mut regions = Vec::new();
        for &(x, width) in &axes[0] {
            for &(y, height) in &axes[1] {
                for &(z, depth) in &axes[2] {
                    regions.push(SdfTextureRegion { origin: [x, y, z], size: [width, height, depth] });
                }
            }
        }
        regions
    }
}

impl WorldRadianceField {
    fn new(camera_position: Vec3, extent: Vec3, resolution: [u32; 3]) -> Self {
        let cell_size = extent / Vec3::new(resolution[0] as f32, resolution[1] as f32, resolution[2] as f32);
        let minimum = Self::snapped_minimum(camera_position, cell_size, extent);
        Self { extent, resolution, minimum, maximum: minimum + extent, cell_size, grid_offset: [0; 3], frame_index: 0, refresh_frames_remaining: 0, needs_full_refresh: true }
    }

    fn uniform(&mut self, camera_position: Vec3) -> RadianceFieldUniform {
        let minimum = Self::snapped_minimum(camera_position, self.cell_size, self.extent);
        let shift_vector = (minimum - self.minimum) / self.cell_size;
        let mut shift = [shift_vector.x.round() as i32, shift_vector.y.round() as i32, shift_vector.z.round() as i32];
        let full_refresh = self.needs_full_refresh || shift.iter().enumerate().any(|(axis, value)| value.unsigned_abs() >= self.resolution[axis]);
        if full_refresh {
            self.grid_offset = [0; 3];
            shift = self.resolution.map(|value| value as i32);
            self.needs_full_refresh = false;
            self.refresh_frames_remaining = 0;
        } else {
            for axis in 0..3 {
                self.grid_offset[axis] = (self.grid_offset[axis] as i32 + shift[axis]).rem_euclid(self.resolution[axis] as i32) as u32;
            }
        }
        self.minimum = minimum;
        self.maximum = minimum + self.extent;
        self.frame_index = self.frame_index.wrapping_add(1);
        let refresh_interval = if self.refresh_frames_remaining > 0 { RADIANCE_FIELD_REFRESH_INTERVAL } else { 0 };
        self.refresh_frames_remaining = self.refresh_frames_remaining.saturating_sub(1);
        RadianceFieldUniform {
            minimum: self.minimum.extend(0.0).to_array(),
            maximum: self.maximum.extend(0.0).to_array(),
            resolution: [self.resolution[0], self.resolution[1], self.resolution[2], 0],
            grid_offset: [self.grid_offset[0], self.grid_offset[1], self.grid_offset[2], 0],
            scroll_shift: [shift[0], shift[1], shift[2], 0],
            settings: [self.frame_index, refresh_interval, RADIANCE_FIELD_DIRECTIONS as u32, u32::from(full_refresh)],
        }
    }

    fn snapped_minimum(camera_position: Vec3, cell_size: Vec3, extent: Vec3) -> Vec3 {
        let snapped_center =
            Vec3::new((camera_position.x / cell_size.x).floor() * cell_size.x, (camera_position.y / cell_size.y).floor() * cell_size.y, (camera_position.z / cell_size.z).floor() * cell_size.z);
        Vec3::new(snapped_center.x - extent.x * 0.5, snapped_center.y - extent.y * 0.875, snapped_center.z - extent.z * 0.5)
    }

    fn invalidate(&mut self) {
        self.needs_full_refresh = true;
        self.refresh_frames_remaining = 0;
    }

    fn refresh_temporally(&mut self) {
        self.refresh_frames_remaining = RADIANCE_FIELD_REFRESH_INTERVAL;
    }
}

fn scrolling_regions(shift: [i32; 3], resolution: [u32; 3]) -> Vec<SdfLogicalRegion> {
    let ranges = (0..3).map(|axis| scrolling_axis_ranges(shift[axis], resolution[axis])).collect::<Vec<_>>();
    let mut regions = Vec::new();
    for dirty_x in &ranges[0].0 {
        regions.push(SdfLogicalRegion { start: [dirty_x.0, 0, 0], size: [dirty_x.1, resolution[1], resolution[2]] });
    }
    for retained_x in &ranges[0].1 {
        for dirty_y in &ranges[1].0 {
            regions.push(SdfLogicalRegion { start: [retained_x.0, dirty_y.0, 0], size: [retained_x.1, dirty_y.1, resolution[2]] });
        }
    }
    for retained_x in &ranges[0].1 {
        for retained_y in &ranges[1].1 {
            for dirty_z in &ranges[2].0 {
                regions.push(SdfLogicalRegion { start: [retained_x.0, retained_y.0, dirty_z.0], size: [retained_x.1, retained_y.1, dirty_z.1] });
            }
        }
    }
    regions
}

fn scrolling_axis_ranges(shift: i32, resolution: u32) -> (Vec<(u32, u32)>, Vec<(u32, u32)>) {
    if shift == 0 {
        return (Vec::new(), vec![(0, resolution)]);
    }
    let magnitude = shift.unsigned_abs();
    if shift > 0 { (vec![(resolution - magnitude, magnitude)], vec![(0, resolution - magnitude)]) } else { (vec![(0, magnitude)], vec![(magnitude, resolution - magnitude)]) }
}

fn split_wrapped_range(start: u32, size: u32, offset: u32, resolution: u32) -> Vec<(u32, u32)> {
    if size == resolution {
        return vec![(0, resolution)];
    }
    let physical_start = (start + offset) % resolution;
    let first_size = size.min(resolution - physical_start);
    let mut ranges = vec![(physical_start, first_size)];
    if first_size < size {
        ranges.push((0, size - first_size));
    }
    ranges
}

fn upload_sdf_region(queue: &wgpu::Queue, texture: &wgpu::Texture, values: &[[f32; 4]], region: SdfTextureRegion) {
    let mut packed_values = Vec::with_capacity((region.size[0] * region.size[1] * region.size[2]) as usize);
    for z in region.origin[2]..region.origin[2] + region.size[2] {
        for y in region.origin[1]..region.origin[1] + region.size[1] {
            let start = (region.origin[0] + y * SDF_CLIPMAP_RESOLUTION[0] + z * SDF_CLIPMAP_RESOLUTION[0] * SDF_CLIPMAP_RESOLUTION[1]) as usize;
            let end = start + region.size[0] as usize;
            packed_values.extend_from_slice(&values[start..end]);
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d { x: region.origin[0], y: region.origin[1], z: region.origin[2] }, aspect: wgpu::TextureAspect::All },
        bytemuck::cast_slice(&packed_values),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(region.size[0] * 16), rows_per_image: Some(region.size[1]) },
        wgpu::Extent3d { width: region.size[0], height: region.size[1], depth_or_array_layers: region.size[2] },
    );
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
        let resolution = [32, 16, 32];
        let cell_size = (maximum - minimum) / Vec3::new(resolution[0] as f32, resolution[1] as f32, resolution[2] as f32);
        let cell_count = (resolution[0] * resolution[1] * resolution[2]) as usize;
        let mut grid = Self { minimum, maximum, cell_size, resolution, occupied: vec![false; cell_count], distance: vec![f32::INFINITY; cell_count], regions: vec![0; cell_count] };
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
        grid
    }

    fn cell_coordinate(&self, position: Vec3) -> [u32; 3] {
        let coordinate =
            ((position - self.minimum) / self.cell_size).floor().clamp(Vec3::ZERO, Vec3::new(self.resolution[0] as f32 - 1.0, self.resolution[1] as f32 - 1.0, self.resolution[2] as f32 - 1.0));
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

    fn surface_region_at(&self, position: Vec3, normal: Vec3) -> u16 {
        let direction = normal.normalize_or_zero();
        let offset_distance = direction.abs().dot(self.cell_size).max(self.cell_size.min_element());
        self.region_at(position + direction * offset_distance)
    }

    fn probe_location(&self, position: Vec3) -> Option<ProbeLocation> {
        let region = self.region_at(position);
        if region == 0 {
            return None;
        }
        let minimum_distance = self.cell_size.min_element() * 0.75;
        if self.distance_at(position) > minimum_distance {
            return Some(ProbeLocation { position });
        }
        let offsets = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        let mut best_position = None;
        let mut best_distance = f32::NEG_INFINITY;
        for radius in 1..=3 {
            for offset in offsets {
                let candidate = position + offset * self.cell_size * radius as f32;
                if self.region_at(candidate) != region {
                    continue;
                }
                let distance = self.distance_at(candidate);
                if distance > best_distance && distance > minimum_distance {
                    best_position = Some(candidate);
                    best_distance = distance;
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
            if position.x < self.minimum.x || position.y < self.minimum.y || position.z < self.minimum.z || position.x >= self.maximum.x || position.y >= self.maximum.y || position.z >= self.maximum.z
            {
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
        let weight = 1.0 / instance.mesh.vertices().len().max(1) as f32;
        target.extend(instance.mesh.vertices().iter().map(|vertex| ProbeSurfaceSample {
            position: instance.transform.transform_point(Vec3::from_array(vertex.position)),
            normal: instance.transform.transform_vector(Vec3::from_array(vertex.normal)).normalize_or_zero(),
            color: Vec3::new(vertex.color[0], vertex.color[1], vertex.color[2]) * Vec3::new(instance.material.base_color.red, instance.material.base_color.green, instance.material.base_color.blue),
            emission: Vec3::new(instance.material.emission.red, instance.material.emission.green, instance.material.emission.blue),
            emission_strength: instance.material.emission_strength,
            weight,
        }));
    }
    (static_samples, dynamic_samples)
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
            Some(SdfGeometry { bounds: GeometryBounds { minimum, maximum }, albedo: vertex_color * material_color, emission: emission_color })
        })
        .collect()
}

fn shadow_draw_data(instances: &[MeshInstance]) -> (Vec<Range<u32>>, Vec<GeometryBounds>) {
    let mut ranges = Vec::with_capacity(instances.len());
    let mut bounds = Vec::with_capacity(instances.len());
    let mut index_start = 0_u32;
    for instance in instances {
        let index_count = instance.mesh.indices().len() as u32;
        ranges.push(index_start..index_start + index_count);
        index_start += index_count;
        let mut minimum = Vec3::splat(f32::INFINITY);
        let mut maximum = Vec3::splat(f32::NEG_INFINITY);
        for vertex in instance.mesh.vertices() {
            let position = instance.transform.transform_point(Vec3::from_array(vertex.position));
            minimum = minimum.min(position);
            maximum = maximum.max(position);
        }
        bounds.push(GeometryBounds { minimum, maximum });
    }
    (ranges, bounds)
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
    if minimum.is_finite() && maximum.is_finite() { GeometryBounds { minimum, maximum } } else { GeometryBounds { minimum: Vec3::splat(-10.0), maximum: Vec3::splat(10.0) } }
}

fn probe_irradiance(
    position: Vec3,
    static_samples: &[ProbeSurfaceSample],
    dynamic_samples: &[ProbeSurfaceSample],
    lights: &[Light],
    visibility_grid: &VisibilityGrid,
    sky_lighting: RendererSkyLighting,
) -> (Vec3, f32, Vec4) {
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
                Light::Directional(light) => (world_normal.dot(light.direction.normalize_or_zero()).max(0.0), (world_position - position).normalize_or_zero(), light.color, light.intensity),
                Light::Point(light) => {
                    let to_light = light.position - world_position;
                    let distance_to_light = to_light.length();
                    let attenuation = (1.0 - distance_to_light / light.range.max(0.001)).max(0.0).powi(2);
                    (world_normal.dot(to_light.normalize_or_zero()).max(0.0) * attenuation, (world_position - position).normalize_or_zero(), light.color, light.intensity)
                }
                Light::Spot(light) => {
                    let to_light = light.position - world_position;
                    let distance_to_light = to_light.length();
                    let light_to_surface = (-to_light).normalize_or_zero();
                    let cone = light_to_surface.dot(light.direction.normalize_or_zero());
                    let cone_factor = ((cone - light.outer_angle.cos()) / (light.inner_angle.cos() - light.outer_angle.cos()).max(0.001)).clamp(0.0, 1.0);
                    let attenuation = (1.0 - distance_to_light / light.range.max(0.001)).max(0.0).powi(2) * cone_factor;
                    (world_normal.dot(to_light.normalize_or_zero()).max(0.0) * attenuation, (world_position - position).normalize_or_zero(), light.color, light.intensity)
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

fn probe_sky_visibility(position: Vec3, visibility_grid: &VisibilityGrid) -> f32 {
    let directions = [Vec3::Y, Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z];
    let visible = directions.iter().filter(|direction| visibility_grid.ray_reaches_sky(position, **direction)).count();
    visible as f32 / directions.len() as f32
}

#[cfg(test)]
mod sdf_clearance_tests {
    use super::*;

    fn occupied_at(clipmap: &SdfClipmap, position: Vec3) -> bool {
        let logical = ((position - clipmap.minimum) / clipmap.voxel_size).floor().as_uvec3();
        let physical_x = (logical.x + clipmap.grid_offset[0]) % SDF_CLIPMAP_RESOLUTION[0];
        let physical_y = (logical.y + clipmap.grid_offset[1]) % SDF_CLIPMAP_RESOLUTION[1];
        let physical_z = (logical.z + clipmap.grid_offset[2]) % SDF_CLIPMAP_RESOLUTION[2];
        let index = (physical_x + physical_y * SDF_CLIPMAP_RESOLUTION[0] + physical_z * SDF_CLIPMAP_RESOLUTION[0] * SDF_CLIPMAP_RESOLUTION[1]) as usize;
        clipmap.values[index][0] <= 0.0
    }

    #[test]
    fn conservative_voxel_field_covers_floors_walls_corners_and_thin_geometry_after_scroll() {
        let geometries = [
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(-2.0, -0.03, -2.0), maximum: Vec3::new(2.0, 0.03, 2.0) }, albedo: Vec3::ONE, emission: Vec3::ZERO },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(3.0, 0.0, -2.0), maximum: Vec3::new(3.02, 3.0, 2.0) }, albedo: Vec3::ONE, emission: Vec3::ZERO },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(4.0, 0.0, 4.0), maximum: Vec3::new(4.1, 0.2, 4.1) }, albedo: Vec3::ONE, emission: Vec3::ZERO },
            SdfGeometry { bounds: GeometryBounds { minimum: Vec3::new(-4.0, -1.0, 0.0), maximum: Vec3::new(-3.99, 1.0, 0.01) }, albedo: Vec3::ONE, emission: Vec3::ZERO },
        ];
        let mut clipmap = SdfClipmap::new(Vec3::ZERO, &geometries);
        let geometry_samples = [Vec3::ZERO, Vec3::new(3.0, 1.0, 0.0), Vec3::new(4.0, 0.0, 4.0), Vec3::new(-4.0, 0.0, 0.0)];
        assert!(geometry_samples.iter().all(|position| occupied_at(&clipmap, *position)));

        let empty_position = Vec3::new(8.0, 1.0, 8.0);
        let empty_logical = ((empty_position - clipmap.minimum) / clipmap.voxel_size).floor().as_uvec3();
        let empty_physical_x = (empty_logical.x + clipmap.grid_offset[0]) % SDF_CLIPMAP_RESOLUTION[0];
        let empty_physical_y = (empty_logical.y + clipmap.grid_offset[1]) % SDF_CLIPMAP_RESOLUTION[1];
        let empty_physical_z = (empty_logical.z + clipmap.grid_offset[2]) % SDF_CLIPMAP_RESOLUTION[2];
        let empty_index = (empty_physical_x + empty_physical_y * SDF_CLIPMAP_RESOLUTION[0] + empty_physical_z * SDF_CLIPMAP_RESOLUTION[0] * SDF_CLIPMAP_RESOLUTION[1]) as usize;
        let empty_center = clipmap.minimum + (empty_logical.as_vec3() + Vec3::splat(0.5)) * clipmap.voxel_size;
        let nearest_geometry_distance = geometries.iter().map(|geometry| signed_distance_to_bounds(empty_center, geometry.bounds)).fold(f32::INFINITY, f32::min);
        let safe_clearance = clipmap.emission_values[empty_index][3];
        assert!(safe_clearance > 0.0);
        assert!(safe_clearance <= nearest_geometry_distance);

        clipmap.update(Vec3::splat(0.6), &geometries).expect("camera movement should scroll the SDF");
        assert!(geometry_samples.iter().all(|position| occupied_at(&clipmap, *position)));
    }
}

#[cfg(test)]
mod shadow_scheduler_tests {
    use super::*;
    use math::PointLight;

    #[test]
    fn prioritizes_directional_lights_and_nearby_local_lights() {
        let camera = Camera::default();
        let lights = vec![
            Light::Point(PointLight { position: Vec3::new(30.0, 0.0, 0.0), color: Color::WHITE, intensity: 1.0, range: 10.0 }),
            Light::Directional(DirectionalLight::default()),
            Light::Point(PointLight { position: Vec3::new(2.0, 0.0, 0.0), color: Color::WHITE, intensity: 1.0, range: 10.0 }),
        ];
        let mut scheduler = ShadowScheduler::default();
        scheduler.rebuild(&lights, &camera, ShadowBudget { max_dynamic_lights: 2, max_point_lights: 2, max_spot_lights: 0, max_updates_per_frame: 2, atlas_size: 4096 });
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
        let requests = [
            ShadowRequest { light_index: 0, resource_kind: ShadowResourceKind::SpotMap, resource_slot: 0, policy: ShadowUpdatePolicy::OnChange, priority: 1, dirty: true },
            ShadowRequest { light_index: 1, resource_kind: ShadowResourceKind::SpotMap, resource_slot: 1, policy: ShadowUpdatePolicy::OnChange, priority: 1, dirty: true },
        ];
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
            .map(|index| {
                Light::Spot(SpotLight { position: Vec3::new(index as f32, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 })
            })
            .collect::<Vec<_>>();
        let mut scheduler = ShadowScheduler::default();
        scheduler.rebuild(&lights, &camera, ShadowBudget { max_dynamic_lights: 8, max_point_lights: 0, max_spot_lights: 8, max_updates_per_frame: 8, atlas_size: SPOT_SHADOW_MAP_SIZE * 2 });
        assert_eq!(scheduler.requests.len(), 4);
        assert!(scheduler.requests.iter().all(|request| request.resource_slot < 4));
    }

    #[test]
    fn camera_uniform_assigns_each_spot_shadow_slot() {
        let lights = [
            Light::Spot(SpotLight { position: Vec3::new(0.0, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 }),
            Light::Spot(SpotLight { position: Vec3::new(2.0, 2.0, 0.0), direction: Vec3::NEG_Y, color: Color::WHITE, intensity: 1.0, range: 10.0, inner_angle: 0.2, outer_angle: 0.5 }),
        ];
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
