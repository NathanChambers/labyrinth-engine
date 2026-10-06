#[cfg(windows)]
mod windows_fullscreen;

mod font;
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
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::monitor::VideoModeHandle;
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

pub use font::FontAsset;
pub use input::InputState;
pub use math::{Camera, Color, DirectionalLight, Easing, EulerRot, Mat2, Mat3, Mat4, Quat, Transform, Vec2, Vec3, Vec4, ease, lerp, lerp_vec3, ping_pong, slerp_quat};
pub use mesh::{Mesh, MeshInstance, MeshVertex};
pub use renderer::GraphicsApi;
pub use scene::{CameraId, LightId, MeshId, Scene};
pub use time::{Time, TimeScaleId};
pub use ui::{UiAnchor, UiAutoLayout, UiButtonId, UiCanvas, UiContainerId, UiFlexDirection, UiLabelId, UiLayout, UiPivot, UiRect, UiSliderId};
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
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self { title: "Labyrinth Engine".into(), width: 1280, height: 720, mode: WindowMode::Windowed, graphics_api: GraphicsApi::Vulkan }
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
    };
    application.setup(&mut context)?;
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
        let renderer = match Renderer::new(window.clone(), self.settings.graphics_api, camera, &lights, self.context.scene.instances()) {
            Ok(renderer) => renderer,
            Err(error) => return self.fail(event_loop, error),
        };
        eprintln!("Renderer ready");
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
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
                let delta_seconds = self.last_frame.elapsed().as_secs_f32();
                self.last_frame = Instant::now();
                self.context.time.advance(delta_seconds);
                if let Some(renderer) = &mut self.renderer {
                    let size = window.inner_size();
                    self.context.ui.update(&self.context.input, size.width.max(1), size.height.max(1));
                    let result = self.application.update(&mut self.context);
                    let result = result.and_then(|_| {
                        let camera = self.context.active_camera.and_then(|id| self.context.scene.get_camera(id)).ok_or("active camera was removed before update")?;
                        renderer.update_camera(camera)
                    });
                    let lights = self.context.active_lights.iter().filter_map(|id| self.context.scene.get_light(*id)).copied().collect::<Vec<_>>();
                    let size = window.inner_size();
                    let ui_vertices = self.context.ui.vertices(size.width.max(1), size.height.max(1));
                    let result = result.and_then(|_| renderer.update_lights(&lights)).and_then(|_| renderer.update_instances(self.context.scene.instances())).and_then(|_| {
                        renderer.update_ui(&ui_vertices);
                        renderer.render()
                    });
                    if let Err(error) = result {
                        self.fail(event_loop, error);
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
