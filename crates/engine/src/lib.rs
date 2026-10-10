#[cfg(windows)]
mod windows_fullscreen;

mod capture;
mod font;
mod gi;
mod gizmo;
mod input;
pub mod primitives;
mod scene;
mod time;
mod ui;

use std::sync::Arc;
use std::time::Instant;

use renderer::Renderer;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::monitor::VideoModeHandle;
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

pub use capture::DebugCapture;
pub use font::FontAsset;
pub use gi::{IrradianceClipmapDesc, IrradianceClipmapLevel, IrradianceMode, IrradianceVolume, IrradianceVolumeDesc, IrradianceVolumeId, IrradianceVolumes};
pub use gizmo::{GizmoCanvas, GizmoVertex};
pub use input::InputState;
pub use math::{Camera, Color, DirectionalLight, Easing, EulerRot, Light, Mat2, Mat3, Mat4, PointLight, Quat, SpotLight, Transform, Vec2, Vec3, Vec4, ease, lerp, lerp_vec3, ping_pong, slerp_quat};
pub use mesh::{Material, Mesh, MeshInstance, MeshVertex};
pub use renderer::{
    GraphicsApi, RADIANCE_FIELD_CHANNEL_COUNT, RADIANCE_FIELD_RAY_COUNT, RADIANCE_FIELD_RAYS_PER_CHANNEL, RADIANCE_SURFACE_CANDIDATE_COUNT, RadianceFieldResolution, RenderDebugMode, RenderSettings,
    RendererGiRayDiagnostic, RendererGiTraceDebugMode, RendererGizmoVertex, RendererIrradianceVolume, RendererPerformanceStats, RendererRadianceFieldGrid, RendererRadianceSurfaceCandidate,
    RendererRadianceSurfaceInspection, RendererSkyLighting, ShadowQuality,
};
pub use scene::{CameraId, LightId, MeshId, Scene};
pub use time::{Time, TimeScaleId};
pub use ui::{UiAnchor, UiAutoLayout, UiButtonId, UiCanvas, UiContainerId, UiFlexDirection, UiGraphId, UiLabelId, UiLayout, UiPanelId, UiPanelWindowId, UiPivot, UiRect, UiSliderId};
pub use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WindowMode {
    #[default]
    Windowed,
    Borderless,
    Exclusive,
}

pub struct WindowSettings {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub mode: WindowMode,
    pub graphics_api: GraphicsApi,
    pub debug_script: Option<std::path::PathBuf>,
    pub debug_capture_root: std::path::PathBuf,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self { title: "Labyrinth Engine".into(), width: 1280, height: 720, mode: WindowMode::Windowed, graphics_api: GraphicsApi::Vulkan, debug_script: None, debug_capture_root: "captures".into() }
    }
}

pub trait Application {
    fn setup(&mut self, _context: &mut RuntimeContext) -> Result<(), String> {
        Ok(())
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String>;

    fn cleanup(&mut self, _context: &mut RuntimeContext) {}
}

pub struct RuntimeContext {
    pub scene: Scene,
    pub time: Time,
    pub input: InputState,
    pub ui: UiCanvas,
    pub active_camera: Option<CameraId>,
    pub active_lights: Vec<LightId>,
    pub irradiance_volumes: IrradianceVolumes,
    pub irradiance_mode: IrradianceMode,
    pub irradiance_clipmap: IrradianceClipmapDesc,
    pub irradiance_update_budget: usize,
    /// World radiance-field cell counts, read when the renderer initializes.
    pub gi_radiance_field_resolution: RadianceFieldResolution,
    /// Current renderer-owned bounds and resolution for the scene radiance field.
    pub gi_radiance_field_grid: RendererRadianceFieldGrid,
    pub sky_lighting: RendererSkyLighting,
    pub performance_stats: RendererPerformanceStats,
    pub gi_trace_diagnostics: [RendererGiRayDiagnostic; RADIANCE_FIELD_RAY_COUNT],
    /// Latest GPU-evaluated surface point and its eight radiance-field contributions.
    pub gi_surface_inspection: RendererRadianceSurfaceInspection,
    /// Whether a surface contribution pick is queued or being read back.
    pub gi_surface_inspection_pending: bool,
    /// Pixel requested for the next depth-prepass surface inspection.
    pub gi_surface_inspector_pick_request: Option<[u32; 2]>,
    /// Whether the selected surface contribution gizmos should be drawn.
    pub gi_surface_inspector_enabled: bool,
    pub gi_trace_selected_ray: u32,
    pub gi_trace_debug_mode: RendererGiTraceDebugMode,
    pub gi_trace_overview_enabled: bool,
    pub gizmos: GizmoCanvas,
    pub gizmos_enabled: bool,
    pub normal_gizmos_enabled: bool,
    pub light_gizmos_enabled: bool,
    pub gi_global_gizmos_enabled: bool,
    pub gi_gizmos_enabled: bool,
    pub gi_trace_gizmos_enabled: bool,
    pub gi_selected_radiance_cell: [u32; 3],
    /// Whether a scene click should capture the cursor; tools can disable this while picking.
    pub cursor_capture_on_scene_click: bool,
    pub gi_radiance_temporal_accumulation_enabled: bool,
    /// Requests one blocking readback of the most recently rendered GPU timestamps.
    pub request_gpu_timing_sample: bool,
    pub sdf_occupancy_gizmos_enabled: bool,
    pub freeze_gi_radiance: bool,
    pub ui_enabled: bool,
    pub render_debug_mode: RenderDebugMode,
    pub render_settings: RenderSettings,
    pub debug_capture: Option<DebugCapture>,
}

fn draw_radiance_surface_inspection_gizmos(gizmos: &mut GizmoCanvas, inspection: RendererRadianceSurfaceInspection, grid: RendererRadianceFieldGrid) {
    if !inspection.valid {
        return;
    }
    let surface = Vec3::from_array(inspection.world_position);
    let normal = Vec3::from_array(inspection.normal).normalize_or_zero();
    gizmos.dot(surface, 0.12, Color::rgb(0.0, 0.95, 1.0));
    gizmos.line(surface, surface + normal * 0.35, Color::rgb(0.2, 0.6, 1.0));
    let visibility_origin = surface + Vec3::from_array(inspection.sdf_origin_offset);
    gizmos.line(surface, visibility_origin, Color::rgb(1.0, 0.72, 0.12));
    gizmos.dot(visibility_origin, 0.07, Color::rgb(0.35, 1.0, 0.48));
    if !inspection.inside_field {
        return;
    }
    let resolution = Vec3::new(grid.resolution[0] as f32, grid.resolution[1] as f32, grid.resolution[2] as f32);
    if resolution.min_element() <= 0.0 {
        return;
    }
    let cell_size = (Vec3::from_array(grid.maximum) - Vec3::from_array(grid.minimum)) / resolution;
    for candidate in inspection.candidates {
        let grid_position = Vec3::new(candidate.grid_position[0] as f32, candidate.grid_position[1] as f32, candidate.grid_position[2] as f32);
        let logical_position = Vec3::from_array(grid.minimum) + (grid_position + Vec3::splat(0.5)) * cell_size;
        let probe = if candidate.probe_valid { Vec3::from_array(candidate.probe_position) } else { logical_position };
        let color = if candidate.probe_valid {
            Color::rgb(display_radiance(candidate.contribution_radiance[0]), display_radiance(candidate.contribution_radiance[1]), display_radiance(candidate.contribution_radiance[2]))
        } else {
            Color::rgb(1.0, 0.25, 0.18)
        };
        gizmos.line(surface, probe, if candidate.probe_valid { Color::rgb(0.58, 0.65, 0.72) } else { color });
        gizmos.dot(probe, 0.06 + candidate.final_weight.sqrt() * 0.24, color);
    }
    if let Some(candidate) = inspection.candidates.iter().filter(|candidate| candidate.has_occupied_voxel_after_start).max_by(|first, second| first.final_weight.total_cmp(&second.final_weight)) {
        gizmos.wire_box(Vec3::from_array(candidate.first_occupied_after_start_minimum), Vec3::from_array(candidate.first_occupied_after_start_maximum), Color::rgba(1.0, 0.52, 0.08, 0.38));
    }
}

fn display_radiance(value: f32) -> f32 {
    let value = value.max(0.0);
    value / (1.0 + value)
}

pub fn run<A: Application + 'static>(settings: WindowSettings, mut application: A) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut context = RuntimeContext {
        scene: Scene::default(),
        time: Time::default(),
        input: InputState::default(),
        ui: UiCanvas::new(UiRect::new(24.0, 24.0, 320.0, 130.0)),
        active_camera: None,
        active_lights: Vec::new(),
        irradiance_volumes: IrradianceVolumes::default(),
        irradiance_mode: IrradianceMode::default(),
        irradiance_clipmap: IrradianceClipmapDesc::default(),
        irradiance_update_budget: 8,
        gi_radiance_field_resolution: RadianceFieldResolution::default(),
        gi_radiance_field_grid: RendererRadianceFieldGrid::default(),
        sky_lighting: RendererSkyLighting::default(),
        performance_stats: RendererPerformanceStats::default(),
        gi_trace_diagnostics: [RendererGiRayDiagnostic::default(); RADIANCE_FIELD_RAY_COUNT],
        gi_surface_inspection: RendererRadianceSurfaceInspection::default(),
        gi_surface_inspection_pending: false,
        gi_surface_inspector_pick_request: None,
        gi_surface_inspector_enabled: false,
        gi_trace_selected_ray: 0,
        gi_trace_debug_mode: RendererGiTraceDebugMode::Geometry,
        gi_trace_overview_enabled: false,
        gizmos: GizmoCanvas::default(),
        gizmos_enabled: false,
        normal_gizmos_enabled: false,
        light_gizmos_enabled: false,
        gi_global_gizmos_enabled: false,
        gi_gizmos_enabled: false,
        gi_trace_gizmos_enabled: false,
        gi_selected_radiance_cell: [4, 1, 4],
        cursor_capture_on_scene_click: true,
        gi_radiance_temporal_accumulation_enabled: true,
        request_gpu_timing_sample: false,
        sdf_occupancy_gizmos_enabled: false,
        freeze_gi_radiance: false,
        ui_enabled: true,
        render_debug_mode: RenderDebugMode::default(),
        render_settings: RenderSettings::default(),
        debug_capture: None,
    };
    application.setup(&mut context)?;
    if let Some(path) = &settings.debug_script {
        let source = std::fs::read_to_string(path).map_err(|error| format!("failed to read debug script: {error}"))?;
        context.debug_capture = Some(DebugCapture::from_script(&source, settings.debug_capture_root.clone())?);
    }
    let mut runner = Runner { settings, application, context, last_frame: Instant::now(), window: None, renderer: None, failure: None };
    let event_loop_result = event_loop.run_app(&mut runner).map_err(|error| error.to_string());
    runner.release_cursor();
    runner.application.cleanup(&mut runner.context);
    event_loop_result?;
    match runner.failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Runner<A> {
    settings: WindowSettings,
    application: A,
    context: RuntimeContext,
    last_frame: Instant,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    failure: Option<String>,
}

fn renderer_irradiance_volumes(volumes: &IrradianceVolumes, mode: IrradianceMode, clipmap: IrradianceClipmapDesc, camera: &Camera) -> Vec<RendererIrradianceVolume> {
    let clipmap_level_count = if matches!(mode, IrradianceMode::Clipmap | IrradianceMode::Combined) { clipmap.active_level_count() } else { 0 };
    let authored_limit = if mode == IrradianceMode::Combined { 4 - clipmap_level_count } else { 4 };
    let mut renderer_volumes = if matches!(mode, IrradianceMode::Volumes | IrradianceMode::Combined) {
        volumes
            .iter()
            .filter(|(_, volume)| volume.desc.enabled)
            .take(authored_limit)
            .map(|(_, volume)| RendererIrradianceVolume {
                minimum: volume.desc.minimum.to_array(),
                maximum: volume.desc.maximum.to_array(),
                resolution: volume.desc.resolution,
                enabled: volume.desc.enabled,
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if matches!(mode, IrradianceMode::Clipmap | IrradianceMode::Combined) {
        for volume in clipmap.volumes_at(camera.transform.position).into_iter().take(clipmap_level_count) {
            renderer_volumes.push(RendererIrradianceVolume { minimum: volume.minimum.to_array(), maximum: volume.maximum.to_array(), resolution: volume.resolution, enabled: true });
        }
    }
    renderer_volumes
}

impl<A> Runner<A> {
    fn release_cursor(&mut self) {
        self.context.input.set_cursor_captured(false);
        if let Some(window) = &self.window {
            self.apply_cursor_mode(window);
        }
    }

    fn set_mode(&mut self, mode: WindowMode) {
        self.settings.mode = mode;
        if let Some(window) = &self.window {
            apply_window_mode(window, mode);
            window.request_redraw();
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: String) {
        self.failure = Some(message);
        event_loop.exit();
    }

    fn apply_cursor_mode(&self, window: &Window) {
        if self.context.input.cursor_captured() {
            let _ = window.set_cursor_grab(CursorGrabMode::Confined).or_else(|_| window.set_cursor_grab(CursorGrabMode::Locked));
            window.set_cursor_visible(false);
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
    }
}

impl<A> ApplicationHandler for Runner<A>
where
    A: Application + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
        if self.window.is_some() {
            return;
        }

        eprintln!("Creating window");
        let attributes = Window::default_attributes().with_title(self.settings.title.clone()).with_inner_size(LogicalSize::new(self.settings.width, self.settings.height));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(event_loop, error.to_string()),
        };
        self.window = Some(window.clone());
        self.context.input.set_cursor_captured(true);
        self.apply_cursor_mode(&window);
        eprintln!("Initializing {:?} renderer", self.settings.graphics_api);
        apply_window_mode(&window, self.settings.mode);
        let camera = match self.context.active_camera.and_then(|id| self.context.scene.get_camera(id)) {
            Some(camera) => camera,
            None => return self.fail(event_loop, "no active camera is assigned".into()),
        };
        let lights = self.context.active_lights.iter().filter_map(|id| self.context.scene.get_light(*id)).copied().collect::<Vec<_>>();
        let irradiance_volumes = renderer_irradiance_volumes(&self.context.irradiance_volumes, self.context.irradiance_mode, self.context.irradiance_clipmap, camera);
        let renderer = match Renderer::new_with_radiance_field_resolution(
            window.clone(),
            self.settings.graphics_api,
            camera,
            &lights,
            self.context.scene.instances(),
            &irradiance_volumes,
            self.context.gi_radiance_field_resolution,
        ) {
            Ok(renderer) => renderer,
            Err(error) => return self.fail(event_loop, error),
        };
        eprintln!("Renderer ready");
        window.request_redraw();
        self.renderer = Some(renderer);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.release_cursor();
        self.renderer = None;
        self.window = None;
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device_id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.context.input.cursor_captured()
        {
            self.context.input.add_mouse_delta(Vec2::new(delta.0 as f32, delta.1 as f32));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != window_id {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(false) => {
                self.context.input.clear_keys();
                self.context.input.set_left_mouse_down(false);
                self.context.input.set_cursor_captured(false);
                self.apply_cursor_mode(window);
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. }
                if !self.context.input.cursor_captured()
                    && !self.context.input.alt_held()
                    && self.context.cursor_capture_on_scene_click
                    && !self.context.ui.pointer_over_ui(self.context.input.cursor_position(), window.inner_size().width, window.inner_size().height) =>
            {
                self.context.input.set_left_mouse_down(true);
                self.context.input.set_cursor_captured(true);
                self.apply_cursor_mode(window);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.context.input.set_left_mouse_down(state == ElementState::Pressed);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.context.input.set_cursor_position(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    self.context.input.set_key(key, event.state == ElementState::Pressed);
                    if key == KeyCode::Escape && event.state == ElementState::Pressed && !event.repeat {
                        self.context.input.set_cursor_captured(false);
                        self.apply_cursor_mode(window);
                    }
                    if matches!(key, KeyCode::AltLeft | KeyCode::AltRight) {
                        self.context.input.set_cursor_captured(!self.context.input.alt_held() && self.context.cursor_capture_on_scene_click);
                        self.apply_cursor_mode(window);
                    }
                    if key == KeyCode::F11 && event.state == ElementState::Pressed && !event.repeat {
                        let next_mode = match self.settings.mode {
                            WindowMode::Windowed => WindowMode::Borderless,
                            WindowMode::Borderless => WindowMode::Exclusive,
                            WindowMode::Exclusive => WindowMode::Windowed,
                        };
                        self.set_mode(next_mode);
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame_start = Instant::now();
                let delta_seconds = self.last_frame.elapsed().as_secs_f32();
                self.last_frame = Instant::now();
                self.context.time.advance(delta_seconds);
                if let Some(renderer) = &mut self.renderer {
                    let size = window.inner_size();
                    self.context.ui.update(&self.context.input, size.width.max(1), size.height.max(1));
                    let result = self.application.update(&mut self.context);
                    let result = result.and_then(|_| {
                        if let Some(capture) = &mut self.context.debug_capture {
                            capture.advance(&mut self.context.scene, self.context.active_camera, delta_seconds)?;
                        }
                        Ok(())
                    });
                    if let Some((shadows_enabled, irradiance_enabled, gizmos_enabled)) = self.context.debug_capture.as_ref().and_then(DebugCapture::render_effects) {
                        self.context.render_settings = RenderSettings { shadows_enabled, irradiance_enabled, ..self.context.render_settings };
                        self.context.gizmos_enabled = gizmos_enabled;
                    }
                    if let Some(ambient_occlusion_enabled) = self.context.debug_capture.as_ref().and_then(DebugCapture::ambient_occlusion) {
                        self.context.render_settings.ambient_occlusion_enabled = ambient_occlusion_enabled;
                    }
                    if let Some(direct_light_visibility_enabled) = self.context.debug_capture.as_ref().and_then(DebugCapture::direct_light_visibility) {
                        self.context.render_settings.direct_light_visibility_enabled = direct_light_visibility_enabled;
                    }
                    if let Some(quality) = self.context.debug_capture.as_ref().and_then(DebugCapture::shadow_quality) {
                        let quality = match quality {
                            0 => ShadowQuality::Low,
                            1 => ShadowQuality::Medium,
                            3 => ShadowQuality::Ultra,
                            _ => ShadowQuality::High,
                        };
                        self.context.render_settings = self.context.render_settings.with_shadow_quality(quality);
                    }
                    if let Some(mode) = self.context.debug_capture.as_ref().and_then(DebugCapture::render_mode) {
                        self.context.render_debug_mode = match mode {
                            scripting::CaptureRenderMode::Lit => RenderDebugMode::LitMaterials,
                            scripting::CaptureRenderMode::Unlit => RenderDebugMode::UnlitMaterials,
                            scripting::CaptureRenderMode::Wireframe => RenderDebugMode::Wireframe,
                            scripting::CaptureRenderMode::ShadowVisibility => RenderDebugMode::ShadowVisibility,
                            scripting::CaptureRenderMode::GiOnly => RenderDebugMode::GiOnly,
                        };
                    }
                    if let Some(ui_enabled) = self.context.debug_capture.as_ref().and_then(DebugCapture::ui_enabled) {
                        self.context.ui_enabled = ui_enabled;
                    }
                    let result = result.and_then(|_| {
                        let camera = self.context.active_camera.and_then(|id| self.context.scene.get_camera(id)).ok_or("active camera was removed before update")?;
                        renderer.update_camera(camera)
                    });
                    let lights = self.context.active_lights.iter().filter_map(|id| self.context.scene.get_light(*id)).copied().collect::<Vec<_>>();
                    let size = window.inner_size();
                    let ui_vertices = if self.context.ui_enabled { self.context.ui.vertices(size.width.max(1), size.height.max(1)) } else { Vec::new() };
                    let capture_without_gizmos = self.context.debug_capture.as_ref().is_some_and(DebugCapture::has_pending_capture);
                    self.context.gizmos.clear();
                    let camera = self.context.active_camera.and_then(|id| self.context.scene.get_camera(id));
                    let clipmaps = camera
                        .map(|camera| self.context.irradiance_clipmap.volumes_at(camera.transform.position).into_iter().take(self.context.irradiance_clipmap.active_level_count()).collect::<Vec<_>>())
                        .unwrap_or_default();
                    if !capture_without_gizmos && self.context.gizmos_enabled {
                        if self.context.gi_global_gizmos_enabled {
                            self.context.irradiance_volumes.draw_debug_gizmos_for_mode(&mut self.context.gizmos, self.context.irradiance_mode, &clipmaps);
                        }
                        if self.context.light_gizmos_enabled {
                            self.context.gizmos.lights(self.context.scene.lights());
                        }
                        if self.context.normal_gizmos_enabled {
                            self.context.gizmos.mesh_normals(self.context.scene.instances(), 0.18);
                        }
                    }
                    if !capture_without_gizmos && self.context.gi_surface_inspector_enabled {
                        draw_radiance_surface_inspection_gizmos(&mut self.context.gizmos, self.context.gi_surface_inspection, self.context.gi_radiance_field_grid);
                    }
                    let gizmo_vertices =
                        self.context.gizmos.vertices().iter().map(|vertex| RendererGizmoVertex { position: vertex.position.to_array(), color: vertex.color.to_array() }).collect::<Vec<_>>();
                    let irradiance_volumes =
                        camera.map(|camera| renderer_irradiance_volumes(&self.context.irradiance_volumes, self.context.irradiance_mode, self.context.irradiance_clipmap, camera)).unwrap_or_default();
                    let result = result.and_then(|_| renderer.update_lights(&lights)).and_then(|_| renderer.update_instances(self.context.scene.instances())).and_then(|_| {
                        renderer.set_irradiance_update_budget(self.context.irradiance_update_budget);
                        renderer.update_render_settings(self.context.render_settings);
                        renderer.update_sky_lighting(self.context.sky_lighting);
                        renderer.set_gi_radiance_temporal_accumulation_enabled(self.context.gi_radiance_temporal_accumulation_enabled);
                        renderer.update_irradiance_volumes(&irradiance_volumes, self.context.scene.instances(), &lights, self.context.irradiance_mode == IrradianceMode::AabbSdf);
                        self.context.gi_radiance_field_grid = renderer.radiance_field_grid();
                        self.context.performance_stats = renderer.performance_stats();
                        renderer.update_render_debug_mode(self.context.render_debug_mode);
                        renderer.set_surface_probe_gizmos_enabled(self.context.gi_gizmos_enabled && self.context.gizmos_enabled && !capture_without_gizmos);
                        renderer.set_gi_radiance_frozen(self.context.freeze_gi_radiance);
                        renderer.set_world_radiance_gizmos_enabled(self.context.gi_global_gizmos_enabled && self.context.gizmos_enabled && !capture_without_gizmos);
                        renderer.set_world_radiance_trace_gizmos_enabled(self.context.gi_trace_gizmos_enabled && self.context.gizmos_enabled && !capture_without_gizmos);
                        renderer.set_selected_radiance_cell(self.context.gi_selected_radiance_cell);
                        renderer.set_selected_radiance_trace_options(self.context.gi_trace_selected_ray, self.context.gi_trace_debug_mode, self.context.gi_trace_overview_enabled);
                        if let Some(pixel) = self.context.gi_surface_inspector_pick_request.take() {
                            renderer.request_radiance_surface_inspection(pixel);
                        }
                        renderer.set_sdf_occupancy_gizmos_enabled(self.context.sdf_occupancy_gizmos_enabled && self.context.gizmos_enabled && !capture_without_gizmos);
                        renderer.update_gizmos(&gizmo_vertices);
                        renderer.update_ui(&ui_vertices);
                        renderer.render()?;
                        self.context.gi_trace_diagnostics = renderer.gi_trace_diagnostics();
                        self.context.gi_surface_inspection = renderer.radiance_surface_inspection();
                        self.context.gi_surface_inspection_pending = renderer.radiance_surface_inspection_pending();
                        Ok(())
                    });
                    if result.is_ok() {
                        let frame_cpu_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
                        renderer.set_frame_cpu_ms(frame_cpu_ms);
                        self.context.performance_stats.frame_cpu_ms = frame_cpu_ms;
                    }
                    let result = result.and_then(|_| {
                        let capture_report_requested = self.context.debug_capture.as_ref().is_some_and(DebugCapture::has_pending_report);
                        if self.context.request_gpu_timing_sample || capture_report_requested {
                            self.context.request_gpu_timing_sample = false;
                            renderer.update_gpu_timestamps()?;
                            self.context.performance_stats = renderer.performance_stats();
                        }
                        if let Some(capture) = &mut self.context.debug_capture {
                            let camera = self.context.active_camera.and_then(|id| self.context.scene.get_camera(id)).ok_or("active camera was removed before capture")?;
                            capture.capture_pending(renderer, camera)?;
                            capture.report_pending(renderer)?;
                        }
                        Ok(())
                    });
                    if let Err(error) = result {
                        self.fail(event_loop, error);
                    } else if self.context.debug_capture.as_ref().is_some_and(DebugCapture::exit_requested) {
                        event_loop.exit();
                    } else {
                        self.context.input.clear_frame();
                        window.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }
}

fn apply_window_mode(window: &Window, mode: WindowMode) {
    let fullscreen = match mode {
        WindowMode::Windowed => None,
        WindowMode::Borderless => Some(Fullscreen::Borderless(None)),
        WindowMode::Exclusive => {
            let video_mode = window.current_monitor().and_then(|monitor| {
                let monitor_size = monitor.size();
                monitor.video_modes().max_by_key(|mode| {
                    let size = mode.size();
                    let native_size = size == monitor_size;
                    let area = u64::from(size.width) * u64::from(size.height);
                    (native_size, area, mode.refresh_rate_millihertz())
                })
            });
            Some(match video_mode {
                Some(video_mode) if exclusive_mode_supported(window, &video_mode) => Fullscreen::Exclusive(video_mode),
                _ => {
                    eprintln!("Exclusive fullscreen unavailable; using borderless fullscreen");
                    Fullscreen::Borderless(None)
                }
            })
        }
    };
    window.set_fullscreen(fullscreen);
}

#[cfg(windows)]
fn exclusive_mode_supported(_window: &Window, video_mode: &VideoModeHandle) -> bool {
    windows_fullscreen::mode_supported(video_mode)
}

#[cfg(target_os = "linux")]
fn exclusive_mode_supported(window: &Window, _video_mode: &VideoModeHandle) -> bool {
    use winit::platform::wayland::WindowExtWayland;

    window.xdg_toplevel().is_none()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn exclusive_mode_supported(_window: &Window, _video_mode: &VideoModeHandle) -> bool {
    true
}
