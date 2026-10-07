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
pub use gi::{IrradianceVolume, IrradianceVolumeDesc, IrradianceVolumeId, IrradianceVolumes};
pub use gizmo::{GizmoCanvas, GizmoVertex};
pub use input::InputState;
pub use math::{Camera, Color, DirectionalLight, Easing, EulerRot, Light, Mat2, Mat3, Mat4, PointLight, Quat, SpotLight, Transform, Vec2, Vec3, Vec4, ease, lerp, lerp_vec3, ping_pong, slerp_quat};
pub use mesh::{Material, Mesh, MeshInstance, MeshVertex};
pub use renderer::{GraphicsApi, RenderDebugMode, RenderSettings, RendererGizmoVertex, RendererIrradianceVolume, RendererPerformanceStats, RendererSkyLighting};
pub use scene::{CameraId, LightId, MeshId, Scene};
pub use time::{Time, TimeScaleId};
pub use ui::{UiAnchor, UiAutoLayout, UiButtonId, UiCanvas, UiContainerId, UiFlexDirection, UiGraphId, UiLabelId, UiLayout, UiPanelId, UiPivot, UiRect, UiSliderId};
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
    pub irradiance_update_budget: usize,
    pub sky_lighting: RendererSkyLighting,
    pub performance_stats: RendererPerformanceStats,
    pub gizmos: GizmoCanvas,
    pub gizmos_enabled: bool,
    pub ui_enabled: bool,
    pub render_debug_mode: RenderDebugMode,
    pub render_settings: RenderSettings,
    pub debug_capture: Option<DebugCapture>,
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
        irradiance_update_budget: 8,
        sky_lighting: RendererSkyLighting::default(),
        performance_stats: RendererPerformanceStats::default(),
        gizmos: GizmoCanvas::default(),
        gizmos_enabled: true,
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

fn renderer_irradiance_volumes(volumes: &IrradianceVolumes) -> Vec<RendererIrradianceVolume> {
    volumes
        .iter()
        .map(|(_, volume)| RendererIrradianceVolume {
            minimum: volume.desc.minimum.to_array(),
            maximum: volume.desc.maximum.to_array(),
            resolution: volume.desc.resolution,
            enabled: volume.desc.enabled,
        })
        .collect()
}

impl<A> Runner<A> {
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
        self.context.input.set_cursor_captured(true);
        self.apply_cursor_mode(&window);
        eprintln!("Initializing {:?} renderer", self.settings.graphics_api);
        apply_window_mode(&window, self.settings.mode);
        let camera = match self.context.active_camera.and_then(|id| self.context.scene.get_camera(id)) {
            Some(camera) => camera,
            None => return self.fail(event_loop, "no active camera is assigned".into()),
        };
        let lights = self.context.active_lights.iter().filter_map(|id| self.context.scene.get_light(*id)).copied().collect::<Vec<_>>();
        let irradiance_volumes = renderer_irradiance_volumes(&self.context.irradiance_volumes);
        let renderer = match Renderer::new(window.clone(), self.settings.graphics_api, camera, &lights, self.context.scene.instances(), &irradiance_volumes) {
            Ok(renderer) => renderer,
            Err(error) => return self.fail(event_loop, error),
        };
        eprintln!("Renderer ready");
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
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
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if !self.context.input.cursor_captured() && !self.context.input.alt_held() => {
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
                        self.context.input.set_cursor_captured(!self.context.input.alt_held());
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
                        self.context.render_settings = RenderSettings { shadows_enabled, irradiance_enabled };
                        self.context.gizmos_enabled = gizmos_enabled;
                    }
                    if let Some(mode) = self.context.debug_capture.as_ref().and_then(DebugCapture::render_mode) {
                        self.context.render_debug_mode = match mode {
                            scripting::CaptureRenderMode::Lit => RenderDebugMode::LitMaterials,
                            scripting::CaptureRenderMode::Unlit => RenderDebugMode::UnlitMaterials,
                            scripting::CaptureRenderMode::Wireframe => RenderDebugMode::Wireframe,
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
                    if !capture_without_gizmos && self.context.gizmos_enabled {
                        self.context.irradiance_volumes.draw_debug_gizmos(&mut self.context.gizmos);
                        self.context.gizmos.lights(self.context.scene.lights());
                        self.context.gizmos.mesh_normals(self.context.scene.instances(), 0.18);
                    }
                    let gizmo_vertices =
                        self.context.gizmos.vertices().iter().map(|vertex| RendererGizmoVertex { position: vertex.position.to_array(), color: vertex.color.to_array() }).collect::<Vec<_>>();
                    let irradiance_volumes = renderer_irradiance_volumes(&self.context.irradiance_volumes);
                    let result = result.and_then(|_| renderer.update_lights(&lights)).and_then(|_| renderer.update_instances(self.context.scene.instances())).and_then(|_| {
                        renderer.set_irradiance_update_budget(self.context.irradiance_update_budget);
                        renderer.update_render_settings(self.context.render_settings);
                        renderer.update_sky_lighting(self.context.sky_lighting);
                        renderer.update_irradiance_volumes(&irradiance_volumes, self.context.scene.instances(), &lights);
                        self.context.performance_stats = renderer.performance_stats();
                        renderer.update_render_debug_mode(self.context.render_debug_mode);
                        renderer.update_gizmos(&gizmo_vertices);
                        renderer.update_ui(&ui_vertices);
                        renderer.render()
                    });
                    if result.is_ok() {
                        let frame_cpu_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
                        renderer.set_frame_cpu_ms(frame_cpu_ms);
                        self.context.performance_stats.frame_cpu_ms = frame_cpu_ms;
                    }
                    let result = result.and_then(|_| {
                        if self.context.debug_capture.as_ref().is_some_and(DebugCapture::has_pending_report) {
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
