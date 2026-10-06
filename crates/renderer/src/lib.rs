use std::mem;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use math::{Camera, DirectionalLight, Vec3};
use mesh::{MeshInstance, MeshVertex, UiVertex};
use wgpu::util::DeviceExt;
use winit::window::Window;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_DIRECTIONAL_LIGHTS: usize = 8;

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
    light_directions: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_colors: [[f32; 4]; MAX_DIRECTIONAL_LIGHTS],
    light_count: [u32; 4],
}

pub struct Renderer {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    _depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    has_presented: bool,
    camera: Camera,
    lights: Vec<DirectionalLight>,
    ui_pipeline: wgpu::RenderPipeline,
    ui_vertex_buffer: wgpu::Buffer,
    ui_vertex_count: u32,
}

impl Renderer {
    pub fn new(window: Arc<Window>, graphics_api: GraphicsApi, camera: &Camera, lights: &[DirectionalLight], instances: &[MeshInstance]) -> Result<Self, String> {
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|error| error.to_string())?;

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let config = surface.get_default_config(&adapter, width, height).ok_or("selected graphics API cannot present to this window")?;
        surface.configure(&device, &config);

        let camera_uniform = camera_uniform(width, height, camera, lights);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera uniform"),
            contents: bytemuck::bytes_of(&camera_uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("diffuse shader"), source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()) });
        let pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("scene pipeline layout"), bind_group_layouts: &[Some(&bind_group_layout)], immediate_size: 0 });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<MeshVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex_main"), buffers: &[Some(vertex_layout)], compilation_options: Default::default() },
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

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("mesh indices"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
        let (depth_texture, depth_view) = create_depth_texture(&device, width, height);
        let ui_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI vertices"),
            contents: bytemuck::cast_slice(&[UiVertex { position: [0.0, 0.0], color: [0.0; 4] }; 6]),
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
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            camera_buffer,
            camera_bind_group,
            _depth_texture: depth_texture,
            depth_view,
            has_presented: false,
            camera: *camera,
            lights: lights.to_vec(),
            ui_pipeline,
            ui_vertex_buffer,
            ui_vertex_count: 0,
        })
    }

    pub fn update_instances(&mut self, instances: &[MeshInstance]) -> Result<(), String> {
        let (vertices, indices) = flatten_instances(instances)?;
        if indices.len() as u32 != self.index_count || vertices.len() * mem::size_of::<MeshVertex>() != self.vertex_buffer.size() as usize {
            return Err("mesh topology cannot change after renderer initialization".into());
        }
        self.queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
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
        let camera_uniform = camera_uniform(width, height, &self.camera, &self.lights);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
    }

    pub fn update_camera(&mut self, camera: &Camera) -> Result<(), String> {
        self.camera = *camera;
        let camera_uniform = camera_uniform(self.config.width, self.config.height, camera, &self.lights);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        Ok(())
    }

    pub fn update_lights(&mut self, lights: &[DirectionalLight]) -> Result<(), String> {
        if lights.len() > MAX_DIRECTIONAL_LIGHTS {
            return Err(format!("renderer supports at most {MAX_DIRECTIONAL_LIGHTS} directional lights"));
        }
        self.lights.clear();
        self.lights.extend_from_slice(lights);
        let camera_uniform = camera_uniform(self.config.width, self.config.height, &self.camera, &self.lights);
        self.queue.write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&camera_uniform));
        Ok(())
    }

    pub fn update_ui(&mut self, vertices: &[UiVertex]) {
        if vertices.is_empty() {
            self.ui_vertex_count = 0;
            return;
        }
        self.ui_vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("UI vertices"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        self.ui_vertex_count = vertices.len() as u32;
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
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
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
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
        if self.ui_vertex_count > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.ui_pipeline);
            pass.set_vertex_buffer(0, self.ui_vertex_buffer.slice(..));
            pass.draw(0..self.ui_vertex_count, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        if !self.has_presented {
            eprintln!("First frame presented");
            self.has_presented = true;
        }
        Ok(())
    }
}

fn camera_uniform(width: u32, height: u32, camera: &Camera, lights: &[DirectionalLight]) -> CameraUniform {
    let aspect = width as f32 / height as f32;
    let mut light_directions = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    let mut light_colors = [[0.0; 4]; MAX_DIRECTIONAL_LIGHTS];
    for (index, light) in lights.iter().take(MAX_DIRECTIONAL_LIGHTS).enumerate() {
        light_directions[index] = light.direction.extend(0.0).to_array();
        light_colors[index] = [light.color.red, light.color.green, light.color.blue, light.intensity];
    }
    CameraUniform { view_projection: camera.view_projection(aspect).to_cols_array_2d(), light_directions, light_colors, light_count: [lights.len().min(MAX_DIRECTIONAL_LIGHTS) as u32, 0, 0, 0] }
}

fn flatten_instances(instances: &[MeshInstance]) -> Result<(Vec<MeshVertex>, Vec<u32>), String> {
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
            MeshVertex { position: position.to_array(), normal: normal.to_array(), color: vertex.color }
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
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
