use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::mem;
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
const SHADOW_MAP_SIZE: u32 = 2048;
const MAX_IRRADIANCE_VOLUMES: usize = 4;
const MAX_IRRADIANCE_PROBES: usize = 1024;
const VISIBILITY_REBUILD_INTERVAL: u32 = 4;
const GPU_TIMESTAMP_COUNT: u32 = 10;

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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    pub shadows_enabled: bool,
    pub irradiance_enabled: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self { shadows_enabled: true, irradiance_enabled: true }
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
    shadow_view_projection: [[f32; 4]; 4],
    spot_shadow_view_projection: [[f32; 4]; 4],
    light_directions: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_colors: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_positions: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_params: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_kinds: [[u32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_count: [u32; 4],
    camera_position: [f32; 4],
    debug_mode: [u32; 4],
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
    volume_count: [u32; 4],
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
    pub gpu_gizmo_ms: f32,
    pub gpu_ui_ms: f32,
    pub gpu_total_ms: f32,
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

#[derive(Clone, Debug, PartialEq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    wireframe_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    wireframe_index_buffer: wgpu::Buffer,
    wireframe_index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    irradiance_buffer: wgpu::Buffer,
    irradiance_cache: IrradianceUniform,
    irradiance_volume_layout: Vec<RendererIrradianceVolume>,
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
    shadow_pipeline: wgpu::RenderPipeline,
    _shadow_texture: wgpu::Texture,
    shadow_view: wgpu::TextureView,
    _shadow_sampler: wgpu::Sampler,
    shadow_uniform_buffer: wgpu::Buffer,
    shadow_bind_group: wgpu::BindGroup,
    spot_shadow_pipeline: wgpu::RenderPipeline,
    _spot_shadow_texture: wgpu::Texture,
    spot_shadow_view: wgpu::TextureView,
    _spot_shadow_sampler: wgpu::Sampler,
    spot_shadow_uniform_buffer: wgpu::Buffer,
    spot_shadow_bind_group: wgpu::BindGroup,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
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
    render_debug_mode: RenderDebugMode,
    render_settings: RenderSettings,
    gpu_timestamps: Option<GpuTimestampState>,
}

impl Renderer {
    pub fn new(window: Arc<Window>, graphics_api: GraphicsApi, camera: &Camera, lights: &[Light], instances: &[MeshInstance], irradiance_volumes: &[RendererIrradianceVolume]) -> Result<Self, String> {
        let (vertices, indices) = flatten_instances(instances)?;
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

        let (shadow_texture, shadow_view) = create_shadow_texture(&device);
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
        let shadow_matrix = lights
            .iter()
            .find_map(|light| match light {
                Light::Directional(light) => Some(shadow_view_projection(light, shadow_bounds)),
                _ => None,
            })
            .unwrap_or(Mat4::IDENTITY);
        let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow uniform"),
            contents: bytemuck::bytes_of(&ShadowUniform { view_projection: shadow_matrix.to_cols_array_2d() }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow bind group"),
            layout: &shadow_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_uniform_buffer.as_entire_binding() }],
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("shadow shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shadow_shader.wgsl").into()) });
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Front), ..Default::default() },
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

        let (spot_shadow_texture, spot_shadow_view) = create_shadow_texture(&device);
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
            contents: bytemuck::bytes_of(&ShadowUniform { view_projection: spot_shadow_matrix.to_cols_array_2d() }),
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
        let spot_shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spot shadow pipeline layout"),
            bind_group_layouts: &[Some(&spot_shadow_bind_group_layout)],
            immediate_size: 0,
        });
        let spot_shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spot shadow pipeline"),
            layout: Some(&spot_shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vertex_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Front), ..Default::default() },
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
        let (static_probe_samples, dynamic_probe_samples) = build_probe_samples(instances, true);
        let static_probe_signature = static_probe_signature(instances);
        let initial_irradiance = irradiance_uniform(irradiance_volumes, &static_probe_samples, &dynamic_probe_samples, lights, sky_lighting, &visibility_grid);
        let irradiance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("irradiance probes"),
            contents: bytemuck::bytes_of(&initial_irradiance),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
            ],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: irradiance_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&spot_shadow_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&spot_shadow_sampler) },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("diffuse shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout)], immediate_size: 0 });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<RendererMeshVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout.clone())], compilation_options: Default::default() },
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
                module: &shader,
                entry_point: Some("fragment_main"),
                targets: &[Some(wgpu::ColorTargetState { format: config.format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let wireframe_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("wireframe pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout)], compilation_options: Default::default() },
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
        let (depth_texture, depth_view) = create_depth_texture(&device, width, height);
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
            wireframe_pipeline,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            wireframe_index_buffer,
            wireframe_index_count: wireframe_indices.len() as u32,
            camera_buffer,
            camera_bind_group,
            irradiance_buffer,
            irradiance_cache: initial_irradiance,
            irradiance_volume_layout: irradiance_volumes.to_vec(),
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
            shadow_pipeline,
            _shadow_texture: shadow_texture,
            shadow_view,
            _shadow_sampler: shadow_sampler,
            shadow_uniform_buffer,
            shadow_bind_group,
            spot_shadow_pipeline,
            _spot_shadow_texture: spot_shadow_texture,
            spot_shadow_view,
            _spot_shadow_sampler: spot_shadow_sampler,
            spot_shadow_uniform_buffer,
            spot_shadow_bind_group,
            _depth_texture: depth_texture,
            depth_view,
            has_presented: false,
            camera: *camera,
            lights: lights.to_vec(),
            ui_pipeline,
            ui_vertex_buffer,
            ui_vertex_capacity: 6,
            ui_vertex_count: 0,
            gizmo_pipeline,
            gizmo_vertex_buffer,
            gizmo_vertex_capacity: 2,
            gizmo_vertex_count: 0,
            render_debug_mode: RenderDebugMode::default(),
            render_settings: RenderSettings::default(),
            gpu_timestamps,
        })
    }

    pub fn update_instances(&mut self, instances: &[MeshInstance]) -> Result<(), String> {
        let update_start = Instant::now();
        let (vertices, indices) = flatten_instances(instances)?;
        if indices.len() as u32 != self.index_count || vertices.len() * mem::size_of::<RendererMeshVertex>() != self.vertex_buffer.size() as usize {
            return Err("mesh topology cannot change after renderer initialization".into());
        }
        self.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        let wireframe_indices = wireframe_indices(&indices);
        self.queue.write_buffer(&self.wireframe_index_buffer, 0, bytemuck::cast_slice(&wireframe_indices));
        let signature = static_probe_signature(instances);
        if signature != self.static_probe_signature {
            let (static_samples, dynamic_samples) = build_probe_samples(instances, true);
            self.static_probe_samples = static_samples;
            self.dynamic_probe_samples = dynamic_samples;
            self.static_probe_signature = signature;
        } else {
            self.dynamic_probe_samples = build_dynamic_probe_samples(instances);
        }
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
        self._depth_texture = depth_texture;
        self.depth_view = depth_view;
        let camera_uniform = camera_uniform(width, height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
    }

    pub fn update_camera(&mut self, camera: &Camera) -> Result<(), String> {
        self.camera = *camera;
        let camera_uniform = camera_uniform(self.config.width, self.config.height, camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        Ok(())
    }

    pub fn update_lights(&mut self, lights: &[Light]) -> Result<(), String> {
        let update_start = Instant::now();
        if lights.len() > MAX_DIRECTIONAL_LIGHTS {
            return Err(format!("renderer supports at most {MAX_DIRECTIONAL_LIGHTS} directional lights"));
        }
        self.lights.clear();
        self.lights.extend_from_slice(lights);
        let shadow_matrix = self
            .lights
            .iter()
            .find_map(|light| match light {
                Light::Directional(light) => Some(shadow_view_projection(light, self.shadow_bounds)),
                _ => None,
            })
            .unwrap_or(Mat4::IDENTITY);
        self.queue.write_buffer(&self.shadow_uniform_buffer, 0, bytemuck::bytes_of(&ShadowUniform { view_projection: shadow_matrix.to_cols_array_2d() }));
        let spot_shadow_matrix = self
            .lights
            .iter()
            .find_map(|light| match light {
                Light::Spot(light) => Some(spot_shadow_view_projection(light)),
                _ => None,
            })
            .unwrap_or(Mat4::IDENTITY);
        self.queue.write_buffer(&self.spot_shadow_uniform_buffer, 0, bytemuck::bytes_of(&ShadowUniform { view_projection: spot_shadow_matrix.to_cols_array_2d() }));
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        self.performance_stats.light_update_ms = update_start.elapsed().as_secs_f32() * 1000.0;
        Ok(())
    }

    pub fn update_irradiance_volumes(&mut self, volumes: &[RendererIrradianceVolume], instances: &[MeshInstance], lights: &[Light]) {
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
            self.irradiance_probe_ages.fill(u32::MAX);
            self.visibility_rebuild_cooldown = VISIBILITY_REBUILD_INTERVAL;
            visibility_rebuild_ms = visibility_start.elapsed().as_secs_f32() * 1000.0;
        }
        if volumes != self.irradiance_volume_layout.as_slice() {
            self.irradiance_cache = irradiance_uniform(volumes, &self.static_probe_samples, &self.dynamic_probe_samples, lights, self.sky_lighting, &self.visibility_grid);
            self.irradiance_volume_layout = volumes.to_vec();
            self.irradiance_probe_ages = [0; MAX_IRRADIANCE_PROBES];
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
                    let position = probe_position_for_index(&active_volumes, index, &self.visibility_grid);
                    let distance_score = 4.0 / (1.0 + self.camera.transform.position.distance_squared(position));
                    let score = self.irradiance_probe_ages[index] as f32 * 0.15 + distance_score;
                    if score > best_score {
                        best_index = Some(index);
                        best_score = score;
                    }
                }
                let Some(index) = best_index else { break };
                let position = probe_position_for_index(&active_volumes, index, &self.visibility_grid);
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
        self.queue.write_buffer(&self.irradiance_buffer, 0, bytemuck::bytes_of(&self.irradiance_cache));
        self.performance_stats = RendererPerformanceStats {
            frame_cpu_ms: self.performance_stats.frame_cpu_ms,
            gpu_shadow_ms: self.performance_stats.gpu_shadow_ms,
            gpu_scene_ms: self.performance_stats.gpu_scene_ms,
            gpu_gizmo_ms: self.performance_stats.gpu_gizmo_ms,
            gpu_ui_ms: self.performance_stats.gpu_ui_ms,
            gpu_total_ms: self.performance_stats.gpu_total_ms,
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
        let scene_ms = to_ms(4, 5);
        let gizmo_ms = if self.gizmo_vertex_count > 0 { to_ms(6, 7) } else { 0.0 };
        let ui_ms = if self.ui_vertex_count > 0 { to_ms(8, 9) } else { 0.0 };
        let mut shadow_ms = 0.0;
        if self.render_settings.shadows_enabled {
            if self.lights.iter().any(|light| matches!(light, Light::Directional(_))) {
                shadow_ms += to_ms(0, 1);
            }
            if self.lights.iter().any(|light| matches!(light, Light::Spot(_))) {
                shadow_ms += to_ms(2, 3);
            }
        }
        drop(mapped);
        timestamps.readback_buffer.unmap();
        self.performance_stats.gpu_shadow_ms = shadow_ms;
        self.performance_stats.gpu_scene_ms = scene_ms;
        self.performance_stats.gpu_gizmo_ms = gizmo_ms;
        self.performance_stats.gpu_ui_ms = ui_ms;
        self.performance_stats.gpu_total_ms = shadow_ms + scene_ms + gizmo_ms + ui_ms;
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
        self.render_settings = settings;
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
        let (_depth_texture, depth_view) = create_depth_texture(&self.device, width, height);
        let capture_uniform = camera_uniform(width, height, camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&capture_uniform));

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
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture encoder") });
        encoder.push_debug_group("GPU capture shadow passes");
        self.render_shadow_pass(&mut encoder);
        self.render_spot_shadow_pass(&mut encoder);
        encoder.pop_debug_group();
        encoder.push_debug_group("GPU capture scene pass");
        self.render_scene_pass(&mut encoder, &color_view, &depth_view);
        encoder.pop_debug_group();
        if self.gizmo_vertex_count > 0 {
            self.render_gizmo_pass(&mut encoder, &color_view, &depth_view);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &color_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded_bytes_per_row), rows_per_image: Some(height) } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
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

        let current_camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights, self.render_debug_mode, self.render_settings, self.shadow_bounds);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&current_camera_uniform));
        Ok(CapturedFrame { width, height, rgba8 })
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
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("scene encoder") });
        encoder.push_debug_group("GPU shadow passes");
        self.render_shadow_pass(&mut encoder);
        self.render_spot_shadow_pass(&mut encoder);
        encoder.pop_debug_group();
        encoder.push_debug_group("GPU scene pass");
        self.render_scene_pass(&mut encoder, &color_view, &self.depth_view);
        encoder.pop_debug_group();
        if self.gizmo_vertex_count > 0 {
            self.render_gizmo_pass(&mut encoder, &color_view, &self.depth_view);
        }
        if self.ui_vertex_count > 0 {
            self.render_ui_pass(&mut encoder, &color_view);
        }
        self.resolve_gpu_timestamps(&mut encoder);
        self.queue.submit(Some(encoder.finish()));
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

    fn render_scene_pass(&self, encoder: &mut wgpu::CommandEncoder, color_view: &wgpu::TextureView, depth_view: &wgpu::TextureView) {
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
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(4),
                end_of_pass_write_index: Some(5),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
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

    fn render_shadow_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.render_settings.shadows_enabled || self.lights.is_empty() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.shadow_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(2),
                end_of_pass_write_index: Some(3),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(0, &self.shadow_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    fn render_spot_shadow_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.render_settings.shadows_enabled || !self.lights.iter().any(|light| matches!(light, Light::Spot(_))) {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spot shadow pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.spot_shadow_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: self.gpu_timestamps.as_ref().map(|timestamps| wgpu::RenderPassTimestampWrites {
                query_set: &timestamps.query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.spot_shadow_pipeline);
        pass.set_bind_group(0, &self.spot_shadow_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
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
                beginning_of_pass_write_index: Some(8),
                end_of_pass_write_index: Some(9),
            }),
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
                beginning_of_pass_write_index: Some(6),
                end_of_pass_write_index: Some(7),
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.gizmo_pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.gizmo_vertex_buffer.slice(..));
        pass.draw(0..self.gizmo_vertex_count, 0..1);
    }
}

fn camera_uniform(width: u32, height: u32, camera: &Camera, lights: &[Light], mode: RenderDebugMode, settings: RenderSettings, shadow_bounds: GeometryBounds) -> CameraUniform {
    let aspect = width as f32 / height as f32;
    let shadow_view_projection = lights
        .iter()
        .find_map(|light| match light {
            Light::Directional(light) => Some(shadow_view_projection(light, shadow_bounds)),
            _ => None,
        })
        .unwrap_or(Mat4::IDENTITY);
    let spot_shadow_view_projection = lights
        .iter()
        .find_map(|light| match light {
            Light::Spot(light) => Some(spot_shadow_view_projection(light)),
            _ => None,
        })
        .unwrap_or(Mat4::IDENTITY);
    let mut light_directions = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_colors = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_positions = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_params = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_kinds = [[0; 4]; MAX_DIRECTIONAL_LIGHTS];
    for (index, light) in lights.iter().take(MAX_DIRECTIONAL_LIGHTS).enumerate() {
        match light {
            Light::Directional(light) => {
                light_directions[index] = light.direction.extend(0.0).to_array();
                light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
                light_kinds[index] = [0, 0, 0, 0];
            }
            Light::Point(light) => {
                light_positions[index] = light.position.extend(light.range).to_array();
                light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
                light_params[index] = [light.range, 0.0, 0.0, 0.0];
                light_kinds[index] = [1, 0, 0, 0];
            }
            Light::Spot(light) => {
                light_directions[index] = light.direction.extend(0.0).to_array();
                light_positions[index] = light.position.extend(light.range).to_array();
                light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
                light_params[index] = [light.range, light.inner_angle.cos(), light.outer_angle.cos(), 0.0];
                light_kinds[index] = [2, 0, 0, 0];
            }
        }
    }
    CameraUniform {
        view_projection: camera.view_projection(aspect).to_cols_array_2d(),
        shadow_view_projection: shadow_view_projection.to_cols_array_2d(),
        spot_shadow_view_projection: spot_shadow_view_projection.to_cols_array_2d(),
        light_directions,
        light_colors,
        light_positions,
        light_params,
        light_kinds,
        light_count: [lights.len().min(MAX_DIRECTIONAL_LIGHTS) as u32, 0, 0, 0],
        camera_position: camera.transform.position.extend(0.0).to_array(),
        debug_mode: [
            match mode {
                RenderDebugMode::LitMaterials => 0,
                RenderDebugMode::UnlitMaterials => 1,
                RenderDebugMode::Wireframe => 2,
            },
            u32::from(settings.shadows_enabled),
            u32::from(settings.irradiance_enabled),
            0,
        ],
    }
}

fn wireframe_indices(indices: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(indices.len() * 2);
    for triangle in indices.as_chunks::<3>().0 {
        result.extend_from_slice(&[triangle[0], triangle[1], triangle[1], triangle[2], triangle[2], triangle[0]]);
    }
    result
}

fn shadow_view_projection(light: &DirectionalLight, bounds: GeometryBounds) -> Mat4 {
    let direction = light.direction.normalize_or_zero();
    let center = (bounds.minimum + bounds.maximum) * 0.5;
    let extent = (bounds.maximum - bounds.minimum).max_element() * 0.5 + 2.0;
    let distance = (bounds.maximum - bounds.minimum).length().max(20.0);
    let position = center + direction * distance;
    let up = if direction.dot(Vec3::Y).abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    let view = Mat4::look_at_rh(position, center, up);
    let projection = Mat4::orthographic_rh(-extent, extent, -extent, extent, 0.1, distance * 2.0 + extent);
    projection * view
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

fn flatten_instances(instances: &[MeshInstance]) -> Result<(Vec<RendererMeshVertex>, Vec<u32>), String> {
    if instances.is_empty() || instances.iter().any(|instance| instance.mesh.is_empty()) {
        return Err("renderer requires nonempty mesh instances".into());
    }

    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut vertex_offset = 0_u32;
    for instance in instances {
        let transform = instance.transform;
        vertices.extend(instance.mesh.vertices().iter().map(|vertex| {
            let position = transform.transform_point(Vec3::from_array(vertex.position));
            let normal = transform.transform_vector(Vec3::from_array(vertex.normal)).normalize_or_zero();
            RendererMeshVertex {
                position: position.to_array(),
                normal: normal.to_array(),
                color: vertex.color,
                material_base_color: instance.material.base_color.to_array(),
                material_params: [instance.material.metallic.clamp(0.0, 1.0), instance.material.roughness.clamp(0.04, 1.0), 0.5, instance.material.emission_strength.max(0.0)],
                material_emission: instance.material.emission.to_array(),
            }
        }));
        indices.extend(instance.mesh.indices().iter().map(|index| index + vertex_offset));
        vertex_offset += instance.mesh.vertices().len() as u32;
    }
    Ok((vertices, indices))
}

fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth texture"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn create_shadow_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("directional shadow map"),
        size: wgpu::Extent3d { width: SHADOW_MAP_SIZE, height: SHADOW_MAP_SIZE, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn irradiance_uniform(
    volumes: &[RendererIrradianceVolume],
    static_samples: &[ProbeSurfaceSample],
    dynamic_samples: &[ProbeSurfaceSample],
    lights: &[Light],
    sky_lighting: RendererSkyLighting,
    visibility_grid: &VisibilityGrid,
) -> IrradianceUniform {
    let mut uniform = IrradianceUniform {
        volumes: [IrradianceVolumeUniform { minimum: [0.0; 4], maximum: [0.0; 4], resolution: [1, 1, 1, 0], probe_offset: 0, _padding: [0; 7] }; MAX_IRRADIANCE_VOLUMES],
        probes: [[0.0; 4]; MAX_IRRADIANCE_PROBES],
        directions: [[0.0; 4]; MAX_IRRADIANCE_PROBES],
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
                    let position = visibility_grid.relocated_probe_position(probe_position(*volume, [x, y, z]));
                    let (color, sky_visibility, direction) = probe_irradiance(position, static_samples, dynamic_samples, lights, visibility_grid, sky_lighting);
                    let index = probe_offset + z as usize * resolution[0] as usize * resolution[1] as usize + y as usize * resolution[0] as usize + x as usize;
                    uniform.probes[index] = [color.x, color.y, color.z, sky_visibility];
                    uniform.directions[index] = [direction.x, direction.y, direction.z, direction.w];
                }
            }
        }
        probe_offset += probe_count;
        uniform.volume_count[0] = (volume_index + 1) as u32;
    }
    uniform
}

fn probe_position(volume: RendererIrradianceVolume, coordinate: [u32; 3]) -> Vec3 {
    let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
    let denominator = [resolution[0].saturating_sub(1).max(1), resolution[1].saturating_sub(1).max(1), resolution[2].saturating_sub(1).max(1)];
    let fraction = Vec3::new(coordinate[0] as f32 / denominator[0] as f32, coordinate[1] as f32 / denominator[1] as f32, coordinate[2] as f32 / denominator[2] as f32);
    Vec3::from_array(volume.minimum) + (Vec3::from_array(volume.maximum) - Vec3::from_array(volume.minimum)) * fraction
}

fn probe_position_for_index(volumes: &[&RendererIrradianceVolume], index: usize, visibility_grid: &VisibilityGrid) -> Vec3 {
    let mut local_index = index;
    for volume in volumes {
        let resolution = [volume.resolution[0].max(1), volume.resolution[1].max(1), volume.resolution[2].max(1)];
        let probe_count = resolution[0] as usize * resolution[1] as usize * resolution[2] as usize;
        if local_index < probe_count {
            let x = local_index % resolution[0] as usize;
            let y = local_index / resolution[0] as usize % resolution[1] as usize;
            let z = local_index / (resolution[0] as usize * resolution[1] as usize);
            return visibility_grid.relocated_probe_position(probe_position(**volume, [x as u32, y as u32, z as u32]));
        }
        local_index -= probe_count;
    }
    Vec3::ZERO
}

#[derive(Clone, Copy, PartialEq)]
struct GeometryBounds {
    minimum: Vec3,
    maximum: Vec3,
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

    fn relocated_probe_position(&self, position: Vec3) -> Vec3 {
        let minimum_distance = self.cell_size.min_element() * 0.75;
        if self.distance_at(position) > minimum_distance {
            return position;
        }
        let offsets = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        let mut best_position = position;
        let mut best_distance = self.distance_at(position);
        for radius in 1..=3 {
            for offset in offsets {
                let candidate = position + offset * self.cell_size * radius as f32;
                let distance = self.distance_at(candidate);
                if distance > best_distance {
                    best_position = candidate;
                    best_distance = distance;
                }
            }
            if best_distance > minimum_distance {
                break;
            }
        }
        best_position
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

    fn ray_reaches_target(&self, origin: Vec3, target: Vec3) -> bool {
        let ray = target - origin;
        let length = ray.length();
        if length <= 0.01 {
            return true;
        }
        let direction = ray / length;
        let minimum_step = self.cell_size.min_element().max(0.01) * 0.5;
        let target_tolerance = self.cell_size.length().max(minimum_step * 2.0);
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
    let sample_scale = 3.5;
    let probe_region = visibility_grid.region_at(position);
    for sample in static_samples.iter().chain(dynamic_samples.iter()) {
        let world_position = sample.position;
        let world_normal = sample.normal;
        let color = sample.color;
        let sample_region = visibility_grid.region_at(world_position + world_normal * visibility_grid.cell_size.min_element().max(0.01));
        if probe_region != 0 && sample_region != 0 && probe_region != sample_region {
            continue;
        }
        let distance = position.distance(world_position);
        let sample_attenuation = 1.0 / (1.0 + distance * distance * 0.15);
        if sample_attenuation < 0.01 {
            continue;
        }
        let source_direction = (world_position - position).normalize_or_zero();
        let source_visible = visibility_grid.ray_reaches_target(visibility_grid.relocated_probe_position(position), world_position);
        if !source_visible {
            continue;
        }
        for light in lights {
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
            let contribution = direct * light_intensity * sample_attenuation * sample_scale * sample.weight;
            irradiance += reflected * contribution;
            direction_sum += source_direction * contribution;
            direction_weight += contribution;
        }
        let emissive_contribution = sample.emission * sample.emission_strength.max(0.0) * sample_scale * sample.weight * sample_attenuation * 2.0;
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
    let origin = visibility_grid.relocated_probe_position(position);
    let visible = directions.iter().filter(|direction| visibility_grid.ray_reaches_sky(origin, **direction)).count();
    visible as f32 / directions.len() as f32
}
