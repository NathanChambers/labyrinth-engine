use std::path::Path;

use labyrinth::{
    Application, Camera, CameraId, Color, DirectionalLight, Easing, EulerRot, FontAsset, GraphicsApi, KeyCode, LightId, MeshId, Quat, RuntimeContext, Transform, UiAnchor, UiAutoLayout, UiButtonId,
    UiLayout, UiSliderId, Vec2, Vec3, WindowMode, WindowSettings, ease, ping_pong, primitives, run,
};

struct PrimitiveDemo {
    cube: Option<MeshId>,
    sphere: Option<MeshId>,
    camera: Option<CameraId>,
    light: Option<LightId>,
    light_slider: Option<UiSliderId>,
    light_lock: Option<UiButtonId>,
    time_slider: Option<UiSliderId>,
    cube_rotation: Quat,
    sphere_time: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    light_angle: f32,
    font: Option<FontAsset>,
}

impl PrimitiveDemo {
    fn setup_scene(&mut self, context: &mut RuntimeContext) {
        context.scene.spawn_mesh(primitives::plane(10.0, Color::WHITE), Transform::IDENTITY);
        self.cube = Some(context.scene.spawn_mesh(primitives::cube([0.0, 0.0, 0.0], 1.2, Color::rgb(0.9, 0.08, 0.06)), Transform::from_position(Vec3::new(-1.2, 0.6, 0.0))));
        self.sphere = Some(context.scene.spawn_mesh(primitives::sphere([0.0, 0.0, 0.0], 0.7, Color::rgb(0.08, 0.75, 0.18), 24, 16), Transform::from_position(Vec3::new(1.2, 0.7, 0.0))));
        self.camera = Some(context.scene.spawn_camera(Camera { clear_color: Color::rgb(0.06, 0.08, 0.12), ..Camera::default() }));
        self.light = Some(context.scene.spawn_light(DirectionalLight::default()));
    }

    fn setup_ui(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/default.ttf");
        let font = FontAsset::from_file(font_path)?;
        context.ui.set_font(font.clone());
        self.font = Some(font);
        let panel_layout = UiLayout::anchored(Vec2::new(320.0, 110.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(24.0, 24.0));
        context.ui.set_window_layout(panel_layout);
        context.ui.add_label_layout(None, UiLayout::anchored(Vec2::new(200.0, 26.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(34.0, 24.0)), "Lighting", 16.0, Color::WHITE);

        let stack = context.ui.add_container(None, UiLayout::anchored(Vec2::new(320.0, 84.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(24.0, 50.0)), UiAutoLayout::column(10.0, 4.0));
        let light_row = context.ui.add_container(Some(stack), UiLayout::new(Vec2::new(0.0, 30.0)), UiAutoLayout::row(0.0, 4.0));
        context.ui.add_label_layout(
            Some(light_row),
            UiLayout::anchored(Vec2::new(112.0, 22.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(0.0, 4.0)),
            "Light angle",
            13.0,
            Color::rgb(0.75, 0.8, 0.9),
        );
        self.light_slider = Some(context.ui.add_slider_layout(Some(light_row), UiLayout::anchored(Vec2::new(0.0, 18.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(0.0, 6.0)), 0.0, 360.0, 0.0));
        self.light_lock = Some(context.ui.add_toggle_button_layout(Some(light_row), UiLayout::anchored(Vec2::new(28.0, 26.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(0.0, 2.0)), false));

        let sphere_row = context.ui.add_container(Some(stack), UiLayout::new(Vec2::new(0.0, 30.0)), UiAutoLayout::row(0.0, 4.0));
        context.ui.add_label_layout(
            Some(sphere_row),
            UiLayout::anchored(Vec2::new(112.0, 22.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(0.0, 4.0)),
            "Sphere time",
            13.0,
            Color::rgb(0.75, 0.8, 0.9),
        );
        self.time_slider = Some(context.ui.add_slider_layout(Some(sphere_row), UiLayout::anchored(Vec2::new(0.0, 18.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(0.0, 6.0)), 0.0, 2.0, 0.5));
        Ok(())
    }

    fn setup_active_entities(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        context.active_camera = self.camera;
        context.active_lights = self.light.into_iter().collect();
        let camera_id = self.camera.ok_or("camera was not created")?;
        let camera = context.scene.get_camera_mut(camera_id).ok_or("camera was not stored")?;
        camera.transform.rotation = Quat::from_euler(EulerRot::YXZ, self.camera_yaw, self.camera_pitch, 0.0);
        Ok(())
    }

    fn update_cube(&mut self, context: &mut RuntimeContext, delta_seconds: f32) -> Result<(), String> {
        self.cube_rotation = (Quat::from_rotation_y(delta_seconds * 1.2) * Quat::from_rotation_x(delta_seconds * 0.7) * self.cube_rotation).normalize();
        let cube_id = self.cube.ok_or("cube was not initialized")?;
        let cube = context.scene.get_mesh_mut(cube_id).ok_or("cube object was removed before update")?;
        cube.transform = Transform::from_position(Vec3::new(-1.2, 0.8, 0.0)).with_rotation(self.cube_rotation);
        Ok(())
    }

    fn update_sphere(&mut self, context: &mut RuntimeContext, delta_seconds: f32) -> Result<(), String> {
        let time_scale = self.time_slider.and_then(|slider| context.ui.slider_value(slider)).unwrap_or(0.5);
        self.sphere_time += delta_seconds * time_scale;
        let sphere_id = self.sphere.ok_or("sphere was not initialized")?;
        let sphere = context.scene.get_mesh_mut(sphere_id).ok_or("sphere object was removed before update")?;
        let amount = ease(Easing::EaseInOutCubic, ping_pong(self.sphere_time * 0.35, 1.0));
        let position = Vec3::new(0.4, 0.7, 0.0).lerp(Vec3::new(2.0, 1.4, 0.0), amount);
        sphere.transform = Transform::from_position(position);
        Ok(())
    }

    fn update_camera(&mut self, context: &mut RuntimeContext, delta_seconds: f32) -> Result<(), String> {
        let mouse_delta = context.input.mouse_delta();
        if context.input.cursor_captured() {
            self.camera_yaw -= mouse_delta.x * 0.002;
            self.camera_pitch = (self.camera_pitch - mouse_delta.y * 0.002).clamp(-1.5, 1.5);
        }
        let camera_id = self.camera.ok_or("camera was not initialized")?;
        let camera = context.scene.get_camera_mut(camera_id).ok_or("camera was removed before update")?;
        let rotation = Quat::from_euler(EulerRot::YXZ, self.camera_yaw, self.camera_pitch, 0.0);
        let mut movement = Vec3::ZERO;
        if context.input.is_key_down(KeyCode::KeyW) {
            movement += rotation * Vec3::NEG_Z;
        }
        if context.input.is_key_down(KeyCode::KeyS) {
            movement += rotation * Vec3::Z;
        }
        if context.input.is_key_down(KeyCode::KeyD) {
            movement += rotation * Vec3::X;
        }
        if context.input.is_key_down(KeyCode::KeyA) {
            movement += rotation * Vec3::NEG_X;
        }
        camera.transform.rotation = rotation;
        camera.transform.translate(movement.normalize_or_zero() * (4.0 * delta_seconds));
        Ok(())
    }

    fn update_light(&mut self, context: &mut RuntimeContext, delta_seconds: f32) -> Result<(), String> {
        let locked = self.light_lock.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        let light_angle = self.light_slider.and_then(|slider| context.ui.slider_value(slider)).unwrap_or(0.0);
        self.light_angle = if locked { light_angle } else { (light_angle + delta_seconds * 30.0).rem_euclid(360.0) };
        if let Some(slider) = self.light_slider {
            context.ui.set_slider_value(slider, self.light_angle);
        }
        let light_id = self.light.ok_or("light was not initialized")?;
        let light = context.scene.get_light_mut(light_id).ok_or("light was removed before update")?;
        let angle = self.light_angle.to_radians();
        light.direction = Vec3::new(angle.cos() * 0.7, 1.0, angle.sin() * 0.7).normalize();
        Ok(())
    }
}

impl Application for PrimitiveDemo {
    fn setup(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        self.setup_scene(context);
        self.setup_ui(context)?;
        self.setup_active_entities(context)
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let delta_seconds = context.time.delta_seconds();
        self.update_cube(context, delta_seconds)?;
        self.update_sphere(context, delta_seconds)?;
        self.update_camera(context, delta_seconds)?;
        self.update_light(context, delta_seconds)
    }
}

fn main() -> Result<(), String> {
    let mut settings = WindowSettings::default();
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--windowed" => settings.mode = WindowMode::Windowed,
            "--borderless" => settings.mode = WindowMode::Borderless,
            "--fullscreen" => settings.mode = WindowMode::Exclusive,
            "--vulkan" => settings.graphics_api = GraphicsApi::Vulkan,
            "--dx12" => settings.graphics_api = GraphicsApi::DirectX12,
            "--opengl" => settings.graphics_api = GraphicsApi::OpenGl,
            "--metal" => settings.graphics_api = GraphicsApi::Metal,
            _ => return Err(format!("unknown option: {argument}")),
        }
    }

    run(
        settings,
        PrimitiveDemo {
            cube: None,
            sphere: None,
            camera: None,
            light: None,
            light_slider: None,
            light_lock: None,
            time_slider: None,
            cube_rotation: Quat::IDENTITY,
            sphere_time: 0.0,
            camera_yaw: 0.58,
            camera_pitch: -0.33,
            light_angle: 0.0,
            font: None,
        },
    )
}
