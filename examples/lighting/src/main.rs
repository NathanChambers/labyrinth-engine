use std::path::Path;

use labyrinth::{
    Application, Camera, CameraId, Color, DirectionalLight, Easing, EulerRot, FontAsset, GraphicsApi, IrradianceMode, IrradianceVolumeDesc, IrradianceVolumeId, KeyCode, LightId, Material, MeshId,
    PointLight, Quat, RendererSkyLighting, RuntimeContext, SpotLight, Transform, Vec2, Vec3, WindowMode, WindowSettings, ease, ping_pong, primitives, run,
};
use labyrinth_debug_ui::{DebugPanels, DebugPanelsConfig};

struct LightingDemo {
    cube: Option<MeshId>,
    sphere: Option<MeshId>,
    camera: Option<CameraId>,
    light: Option<LightId>,
    fill_light: Option<LightId>,
    point_light: Option<LightId>,
    spot_light: Option<LightId>,
    irradiance_volume: Option<IrradianceVolumeId>,
    large_scene: bool,
    gi_test_scene: bool,
    open_world: bool,
    shadow_test_scene: bool,
    room_test_scene: bool,
    shadow_isolation: bool,
    gi_mode_key_was_down: bool,
    debug_panels: Option<DebugPanels>,
    cube_rotation: Quat,
    sphere_time: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    light_angle: f32,
    font: Option<FontAsset>,
}

impl LightingDemo {
    fn setup_scene(&mut self, context: &mut RuntimeContext) {
        if self.gi_test_scene {
            self.spawn_gi_test_scene(context);
        } else if self.shadow_test_scene {
            self.spawn_shadow_test_scene(context);
        } else if self.open_world {
            self.spawn_open_world_scene(context);
        } else if self.room_test_scene {
            self.spawn_room_test_scene(context);
        } else if self.large_scene {
            self.spawn_large_lighting_scene(context);
        } else {
            context.scene.spawn_static_mesh(primitives::plane(20.0, Color::WHITE), Transform::IDENTITY);
            self.spawn_lighting_test_room(context);
        }
        self.irradiance_volume = Some(context.irradiance_volumes.add(if self.room_test_scene {
            IrradianceVolumeDesc { minimum: Vec3::new(-15.0, 0.0, -15.0), maximum: Vec3::new(15.0, 6.0, 15.0), resolution: [12, 7, 12], enabled: true }
        } else if self.shadow_test_scene {
            IrradianceVolumeDesc { minimum: Vec3::new(-8.0, 0.0, -8.0), maximum: Vec3::new(8.0, 6.0, 8.0), resolution: [4, 2, 4], enabled: false }
        } else if self.gi_test_scene {
            IrradianceVolumeDesc { minimum: Vec3::new(-5.0, 0.0, -5.0), maximum: Vec3::new(5.0, 5.0, 5.0), resolution: [16, 5, 12], enabled: true }
        } else if self.open_world {
            IrradianceVolumeDesc { minimum: Vec3::new(-64.0, 0.0, -64.0), maximum: Vec3::new(64.0, 12.0, 64.0), resolution: [12, 4, 12], enabled: true }
        } else if self.large_scene {
            IrradianceVolumeDesc { minimum: Vec3::new(-20.0, 0.18, -20.0), maximum: Vec3::new(20.0, 5.5, 20.0), resolution: [10, 5, 10], enabled: true }
        } else {
            IrradianceVolumeDesc { minimum: Vec3::new(-3.0, 0.12, -3.0), maximum: Vec3::new(3.0, 3.0, 3.0), resolution: [8, 4, 8], enabled: true }
        }));
        if !self.room_test_scene {
            let object_depth = if self.gi_test_scene { -3.2 } else { 0.0 };
            let object_x = if self.gi_test_scene { 1.5 } else { 1.2 };
            self.cube = Some(context.scene.spawn_mesh(primitives::cube([0.0, 0.0, 0.0], 1.2, Color::rgb(0.9, 0.08, 0.06)), Transform::from_position(Vec3::new(-object_x, 0.6, object_depth))));
            if let Some(cube_id) = self.cube
                && let Some(cube) = context.scene.get_mesh_mut(cube_id)
            {
                cube.material = Material { metallic: 0.3, roughness: 0.2, ..Material::default() };
            }
            self.sphere =
                Some(context.scene.spawn_mesh(primitives::sphere([0.0, 0.0, 0.0], 0.7, Color::rgb(0.08, 0.75, 0.18), 24, 16), Transform::from_position(Vec3::new(object_x, 0.7, object_depth))));
            if let Some(sphere_id) = self.sphere
                && let Some(sphere) = context.scene.get_mesh_mut(sphere_id)
            {
                sphere.material = Material { metallic: 0.2, roughness: 0.16, ..Material::default() };
            }
        }
        self.camera = Some(context.scene.spawn_camera(Camera { clear_color: Color::rgb(0.06, 0.08, 0.12), ..Camera::default() }));
        if self.shadow_test_scene {
            if let Some(camera) = self.camera.and_then(|id| context.scene.get_camera_mut(id)) {
                camera.transform.position = Vec3::new(0.0, 3.8, 9.5);
            }
            self.camera_yaw = 0.0;
            self.camera_pitch = -0.2;
        } else if self.gi_test_scene {
            if let Some(camera) = self.camera.and_then(|id| context.scene.get_camera_mut(id)) {
                camera.transform.position = Vec3::new(0.0, 2.4, 7.0);
            }
            self.camera_yaw = 0.0;
            self.camera_pitch = -0.16;
        } else if self.open_world {
            if let Some(camera) = self.camera.and_then(|id| context.scene.get_camera_mut(id)) {
                camera.transform.position = Vec3::new(0.0, 8.0, 22.0);
            }
            self.camera_yaw = 0.0;
            self.camera_pitch = -0.22;
        } else if self.room_test_scene {
            if let Some(camera) = self.camera.and_then(|id| context.scene.get_camera_mut(id)) {
                camera.transform.position = Vec3::new(0.0, 4.0, 13.0);
            }
            self.camera_yaw = 0.0;
            self.camera_pitch = -0.2;
        } else if self.large_scene {
            if let Some(camera) = self.camera.and_then(|id| context.scene.get_camera_mut(id)) {
                camera.transform.position = Vec3::new(0.0, 2.8, 12.0);
            }
            self.camera_yaw = 0.0;
            self.camera_pitch = -0.18;
        }
        if self.room_test_scene {
            context.sky_lighting = RendererSkyLighting { color: Color::rgb(0.08, 0.09, 0.11), intensity: 0.04 };
            self.point_light = Some(context.scene.spawn_point_light(PointLight { position: Vec3::new(0.0, 4.0, 0.0), color: Color::WHITE, intensity: 2.0, range: 24.0 }));
        } else if self.shadow_test_scene {
            context.sky_lighting = RendererSkyLighting { color: Color::rgb(0.12, 0.14, 0.18), intensity: 0.08 };
            self.light = Some(context.scene.spawn_light(DirectionalLight { direction: Vec3::new(-0.55, 0.85, -0.35).normalize(), color: Color::WHITE, intensity: 1.8 }));
        } else if self.gi_test_scene {
            context.sky_lighting = RendererSkyLighting { color: Color::rgb(0.18, 0.2, 0.22), intensity: 0.08 };
            self.light = Some(context.scene.spawn_light(DirectionalLight { intensity: 0.12, ..DirectionalLight::default() }));
            self.point_light = Some(context.scene.spawn_point_light(PointLight { position: Vec3::new(-2.2, 1.4, -2.0), color: Color::WHITE, intensity: 3.5, range: 4.0 }));
            self.spot_light = Some(context.scene.spawn_spot_light(SpotLight {
                position: Vec3::new(3.0, 2.4, -0.8),
                direction: Vec3::new(-0.5, -0.4, -1.0).normalize(),
                color: Color::WHITE,
                intensity: 2.0,
                range: 9.0,
                inner_angle: 0.22,
                outer_angle: 0.65,
            }));
        } else {
            context.sky_lighting = RendererSkyLighting { color: Color::rgb(0.18, 0.28, 0.52), intensity: 0.65 };
            self.light = Some(context.scene.spawn_light(DirectionalLight { color: Color::rgb(1.0, 0.78, 0.55), intensity: 1.25, ..DirectionalLight::default() }));
            self.fill_light = Some(context.scene.spawn_light(DirectionalLight { direction: Vec3::new(-0.45, 0.75, -0.3).normalize(), color: Color::rgb(0.28, 0.42, 1.0), intensity: 0.1 }));
            self.point_light = Some(context.scene.spawn_point_light(PointLight { position: Vec3::new(-3.5, 1.8, -2.0), color: Color::WHITE, intensity: 4.0, range: 8.0 }));
            self.spot_light = Some(context.scene.spawn_spot_light(SpotLight {
                position: Vec3::new(3.5, 4.8, 3.0),
                direction: Vec3::new(-0.4, -1.0, -0.7).normalize(),
                color: Color::WHITE,
                intensity: 7.0,
                range: 12.0,
                inner_angle: 0.25,
                outer_angle: 0.7,
            }));
        }
    }

    fn spawn_gi_test_scene(&mut self, context: &mut RuntimeContext) {
        let wall_color = Color::rgb(0.72, 0.72, 0.72);
        context.scene.spawn_static_mesh(primitives::plane(16.0, wall_color), Transform::IDENTITY);
        self.spawn_box(context, Vec3::new(0.0, 2.5, -5.0), Vec3::new(10.0, 5.0, 0.2), wall_color);
        self.spawn_box(context, Vec3::new(-5.0, 2.5, 1.0), Vec3::new(0.2, 5.0, 12.0), wall_color);
        self.spawn_box(context, Vec3::new(5.0, 2.5, 1.0), Vec3::new(0.2, 5.0, 12.0), wall_color);
        self.spawn_box(context, Vec3::new(0.0, 5.0, 1.0), Vec3::new(10.0, 0.2, 12.0), wall_color);
    }

    fn spawn_shadow_test_scene(&mut self, context: &mut RuntimeContext) {
        context.scene.spawn_static_mesh(primitives::plane(24.0, Color::rgb(0.42, 0.45, 0.48)), Transform::IDENTITY);
        self.spawn_box(context, Vec3::new(-3.0, 1.0, -1.0), Vec3::new(2.0, 2.0, 2.0), Color::rgb(0.85, 0.08, 0.06));
        self.spawn_box(context, Vec3::new(-1.8, 1.2, -1.0), Vec3::new(1.6, 2.4, 1.6), Color::rgb(0.08, 0.75, 0.18));
        context.scene.spawn_static_mesh(primitives::sphere([0.0, 0.0, 0.0], 1.0, Color::rgb(0.08, 0.35, 0.95), 32, 20), Transform::from_position(Vec3::new(1.0, 1.0, -1.0)));
        self.spawn_box(context, Vec3::new(4.0, 1.0, -1.0), Vec3::new(2.0, 2.0, 2.0), Color::rgb(0.9, 0.55, 0.08));
        self.spawn_box(context, Vec3::new(4.0, 2.75, -1.0), Vec3::new(2.0, 1.5, 2.0), Color::rgb(0.65, 0.2, 0.8));
        self.spawn_box(context, Vec3::new(0.0, 2.0, -5.0), Vec3::new(12.0, 4.0, 0.25), Color::rgb(0.58, 0.62, 0.66));
    }

    fn spawn_room_test_scene(&mut self, context: &mut RuntimeContext) {
        let floor = Color::rgb(0.28, 0.3, 0.34);
        let wall = Color::rgb(0.48, 0.5, 0.54);
        context.scene.spawn_static_mesh(primitives::plane(32.0, floor), Transform::IDENTITY);
        self.spawn_box(context, Vec3::new(-1.6, 0.75, 0.0), Vec3::new(1.5, 1.5, 1.5), Color::rgb(0.04, 0.85, 0.12));
        self.spawn_box(context, Vec3::new(1.6, 0.75, 0.0), Vec3::new(1.5, 1.5, 1.5), Color::rgb(0.9, 0.04, 0.03));
        // Central room: the split north wall leaves one doorway centered on the hallway entrance.
        self.spawn_box(context, Vec3::new(-5.0, 2.0, 0.0), Vec3::new(0.3, 4.0, 10.0), wall);
        self.spawn_box(context, Vec3::new(5.0, 2.0, 0.0), Vec3::new(0.3, 4.0, 10.0), wall);
        self.spawn_box(context, Vec3::new(0.0, 2.0, -5.0), Vec3::new(10.0, 4.0, 0.3), wall);
        self.spawn_box(context, Vec3::new(-3.335, 2.0, 5.0), Vec3::new(3.33, 4.0, 0.3), wall);
        self.spawn_box(context, Vec3::new(3.335, 2.0, 5.0), Vec3::new(3.33, 4.0, 0.3), wall);

        // Outer four-sided room wraps the center room, leaving one continuous ring hallway.
        self.spawn_box(context, Vec3::new(-13.0, 2.0, 0.0), Vec3::new(0.3, 4.0, 26.0), wall);
        self.spawn_box(context, Vec3::new(13.0, 2.0, 0.0), Vec3::new(0.3, 4.0, 26.0), wall);
        self.spawn_box(context, Vec3::new(0.0, 2.0, -13.0), Vec3::new(26.0, 4.0, 0.3), wall);
        self.spawn_box(context, Vec3::new(0.0, 2.0, 13.0), Vec3::new(26.0, 4.0, 0.3), wall);
    }

    fn spawn_large_lighting_scene(&mut self, context: &mut RuntimeContext) {
        let floor_color = Color::rgb(0.78, 0.8, 0.84);
        let wall_color = Color::rgb(0.62, 0.66, 0.72);
        let trim_color = Color::rgb(0.3, 0.34, 0.4);
        context.scene.spawn_static_mesh(primitives::plane(28.0, floor_color), Transform::IDENTITY);

        self.spawn_box(context, Vec3::new(0.0, 3.0, -10.0), Vec3::new(28.0, 6.0, 0.25), wall_color);
        self.spawn_box(context, Vec3::new(-14.0, 3.0, 2.0), Vec3::new(0.25, 6.0, 24.0), wall_color);
        self.spawn_box(context, Vec3::new(14.0, 3.0, 2.0), Vec3::new(0.25, 6.0, 24.0), wall_color);
        self.spawn_box(context, Vec3::new(0.0, 5.5, 2.0), Vec3::new(28.0, 0.25, 24.0), wall_color);

        self.spawn_box(context, Vec3::new(-8.0, 2.5, 0.0), Vec3::new(0.25, 5.0, 12.0), wall_color);
        self.spawn_box(context, Vec3::new(8.0, 2.5, 0.0), Vec3::new(0.25, 5.0, 12.0), wall_color);
        self.spawn_box(context, Vec3::new(-5.0, 2.5, -4.0), Vec3::new(6.0, 5.0, 0.25), wall_color);
        self.spawn_box(context, Vec3::new(5.0, 2.5, -4.0), Vec3::new(6.0, 5.0, 0.25), wall_color);
        self.spawn_box(context, Vec3::new(-5.0, 2.5, 4.0), Vec3::new(6.0, 5.0, 0.25), wall_color);
        self.spawn_box(context, Vec3::new(5.0, 2.5, 4.0), Vec3::new(6.0, 5.0, 0.25), wall_color);

        for x in [-11.0, -7.0, -3.0, 3.0, 7.0, 11.0] {
            self.spawn_box(context, Vec3::new(x, 1.0, 8.0), Vec3::splat(1.0), trim_color);
            self.spawn_box(context, Vec3::new(x, 2.8, 8.0), Vec3::new(0.65, 2.6, 0.65), wall_color);
        }
        for z in [-7.0, -1.0, 5.0, 11.0] {
            for x in [-11.0, -5.0, 5.0, 11.0] {
                let color = if (x as i32 + z as i32) % 2 == 0 { trim_color } else { wall_color };
                self.spawn_box(context, Vec3::new(x, 0.5, z), Vec3::new(0.7, 1.0, 0.7), color);
            }
        }

        let emissive_panels =
            [(Vec3::new(-11.0, 2.0, -8.5), Color::rgb(1.0, 0.12, 0.04)), (Vec3::new(0.0, 2.0, -8.5), Color::rgb(0.08, 1.0, 0.18)), (Vec3::new(11.0, 2.0, -8.5), Color::rgb(0.08, 0.3, 1.0))];
        for (position, color) in emissive_panels {
            let emissive_material = Material { base_color: color, emission: color, emission_strength: 8.0, ..Material::default() };
            self.spawn_static_material_box(context, position, Vec3::new(1.0, 2.0, 0.15), color, emissive_material);
        }
    }

    fn spawn_open_world_scene(&mut self, context: &mut RuntimeContext) {
        let ground = Color::rgb(0.24, 0.32, 0.22);
        let stone = Color::rgb(0.34, 0.38, 0.42);
        let accent = Color::rgb(0.58, 0.42, 0.2);
        context.scene.spawn_static_mesh(primitives::plane(128.0, ground), Transform::IDENTITY);
        for z in -8_i32..=8_i32 {
            for x in -8_i32..=8_i32 {
                let world_x = x as f32 * 7.5;
                let world_z = z as f32 * 7.5;
                let height = 0.8 + ((x * 17 + z * 31).rem_euclid(5) as f32) * 0.35;
                let width = 0.9 + ((x - z).abs() % 3) as f32 * 0.25;
                let color = if (x + z).rem_euclid(5) == 0 { accent } else { stone };
                self.spawn_box(context, Vec3::new(world_x, height * 0.5, world_z), Vec3::new(width, height, width), color);
                if (x * 3 + z * 5).rem_euclid(7) == 0 {
                    self.spawn_box(context, Vec3::new(world_x + 1.8, 1.2, world_z - 1.4), Vec3::new(0.7, 2.4, 0.7), Color::rgb(0.18, 0.25, 0.16));
                }
            }
        }
        for (position, color) in [(Vec3::new(-18.0, 3.0, -28.0), Color::rgb(1.0, 0.12, 0.04)), (Vec3::new(18.0, 3.0, -28.0), Color::rgb(0.04, 0.2, 1.0))] {
            let material = Material { base_color: color, emission: color, emission_strength: 4.0, ..Material::default() };
            self.spawn_static_material_box(context, position, Vec3::new(2.0, 6.0, 0.3), color, material);
        }
    }

    fn spawn_box(&self, context: &mut RuntimeContext, position: Vec3, scale: Vec3, color: Color) {
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, color), Transform::from_position(position).with_scale(scale));
    }

    fn spawn_static_material_box(&self, context: &mut RuntimeContext, position: Vec3, scale: Vec3, color: Color, material: Material) {
        let id = context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, color), Transform::from_position(position).with_scale(scale));
        if let Some(instance) = context.scene.get_mesh_mut(id) {
            instance.material = material;
        }
    }

    fn spawn_lighting_test_room(&mut self, context: &mut RuntimeContext) {
        let wall_color = Color::rgb(0.72, 0.74, 0.78);
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, wall_color), Transform::from_position(Vec3::new(0.0, 1.5, -3.0)).with_scale(Vec3::new(6.0, 3.0, 0.2)));
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, wall_color), Transform::from_position(Vec3::new(-3.0, 1.5, 0.0)).with_scale(Vec3::new(0.2, 3.0, 6.0)));
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, wall_color), Transform::from_position(Vec3::new(3.0, 1.5, 0.0)).with_scale(Vec3::new(0.2, 3.0, 6.0)));
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, wall_color), Transform::from_position(Vec3::new(-4.5, 1.5, 3.0)).with_scale(Vec3::new(3.0, 3.0, 0.2)));
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, wall_color), Transform::from_position(Vec3::new(4.5, 1.5, 3.0)).with_scale(Vec3::new(3.0, 3.0, 0.2)));
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, wall_color), Transform::from_position(Vec3::new(0.0, 3.0, 0.0)).with_scale(Vec3::new(3.0, 0.2, 3.0)));
        context.scene.spawn_static_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, Color::rgb(0.48, 0.28, 0.12)), Transform::from_position(Vec3::new(0.0, 1.0, 2.9)).with_scale(Vec3::new(1.2, 2.0, 0.1)));
    }

    fn setup_ui(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/default.ttf");
        let font = FontAsset::from_file(font_path)?;
        context.ui.set_font(font.clone());
        self.font = Some(font);
        let mut debug_config = DebugPanelsConfig::new("Lighting Inspector").with_window_background();
        debug_config.performance_offset = Vec2::new(-24.0, 24.0);
        let debug_panels = DebugPanels::new(context, debug_config);
        self.debug_panels = Some(debug_panels);
        Ok(())
    }

    fn update_render_effects(&mut self, context: &mut RuntimeContext) {
        self.debug_panels.as_mut().expect("shared debug panels are initialized").update(context);
    }

    fn setup_active_entities(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        context.active_camera = self.camera;
        context.active_lights.clear();
        if let Some(light) = self.light {
            context.active_lights.push(light);
        }
        if !self.shadow_isolation {
            if let Some(light) = self.fill_light {
                context.active_lights.push(light);
            }
            if let Some(light) = self.point_light {
                context.active_lights.push(light);
            }
            if let Some(light) = self.spot_light {
                context.active_lights.push(light);
            }
        }
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
        self.sphere_time += delta_seconds * 0.5;
        let sphere_id = self.sphere.ok_or("sphere was not initialized")?;
        let sphere = context.scene.get_mesh_mut(sphere_id).ok_or("sphere object was removed before update")?;
        let amount = ease(Easing::EaseInOutCubic, ping_pong(self.sphere_time * 0.35, 1.0));
        let position = Vec3::new(0.4, 0.7, 0.0).lerp(Vec3::new(2.0, 1.4, 0.0), amount);
        sphere.transform = Transform::from_position(position);
        Ok(())
    }

    fn update_camera(&mut self, context: &mut RuntimeContext, delta_seconds: f32, movement_keys_active: bool) -> Result<(), String> {
        let mouse_delta = context.input.mouse_delta();
        if context.input.cursor_captured() {
            self.camera_yaw -= mouse_delta.x * 0.002;
            self.camera_pitch = (self.camera_pitch - mouse_delta.y * 0.002).clamp(-1.5, 1.5);
        }
        let camera_id = self.camera.ok_or("camera was not initialized")?;
        let camera = context.scene.get_camera_mut(camera_id).ok_or("camera was removed before update")?;
        let rotation = Quat::from_euler(EulerRot::YXZ, self.camera_yaw, self.camera_pitch, 0.0);
        let mut movement = Vec3::ZERO;
        if !movement_keys_active {
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
        }
        camera.transform.rotation = rotation;
        camera.transform.translate(movement.normalize_or_zero() * (4.0 * delta_seconds));
        Ok(())
    }

    fn update_light(&mut self, context: &mut RuntimeContext, delta_seconds: f32) -> Result<(), String> {
        if self.room_test_scene {
            let light_id = self.point_light.ok_or("point light was not initialized")?;
            let mut movement = Vec3::ZERO;
            if context.input.is_key_down(KeyCode::KeyJ) {
                movement.x -= 1.0;
            }
            if context.input.is_key_down(KeyCode::KeyL) {
                movement.x += 1.0;
            }
            if context.input.is_key_down(KeyCode::KeyI) {
                movement.y += 1.0;
            }
            if context.input.is_key_down(KeyCode::KeyK) {
                movement.y -= 1.0;
            }
            if context.input.is_key_down(KeyCode::KeyU) {
                movement.z -= 1.0;
            }
            if context.input.is_key_down(KeyCode::KeyO) {
                movement.z += 1.0;
            }
            if movement != Vec3::ZERO {
                let point_light = context.scene.get_point_light_mut(light_id).ok_or("point light was removed before update")?;
                point_light.position += movement.normalize() * (2.0 * delta_seconds);
            }
            return Ok(());
        }
        if !self.shadow_test_scene {
            self.light_angle = (self.light_angle + delta_seconds * 30.0).rem_euclid(360.0);
        }
        let light_id = self.light.ok_or("light was not initialized")?;
        let light = context.scene.get_directional_light_mut(light_id).ok_or("light was removed before update")?;
        let angle = self.light_angle.to_radians();
        light.direction = Vec3::new(angle.cos() * 0.7, 1.0, angle.sin() * 0.7).normalize();
        Ok(())
    }
}

impl Application for LightingDemo {
    fn setup(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        self.setup_scene(context);
        if self.room_test_scene {
            context.irradiance_mode = IrradianceMode::AabbSdf;
        }
        context.gi_radiance_temporal_accumulation_enabled = true;
        context.gizmos_enabled = false;
        context.gi_gizmos_enabled = false;
        self.setup_ui(context)?;
        self.setup_active_entities(context)
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let delta_seconds = context.time.delta_seconds();
        let gi_mode_key_down = context.input.is_key_down(KeyCode::KeyC);
        if gi_mode_key_down && !self.gi_mode_key_was_down {
            context.irradiance_mode = match context.irradiance_mode {
                IrradianceMode::Volumes => IrradianceMode::Clipmap,
                IrradianceMode::Clipmap => IrradianceMode::Combined,
                IrradianceMode::Combined => IrradianceMode::AabbSdf,
                IrradianceMode::AabbSdf => IrradianceMode::Volumes,
            };
        }
        self.gi_mode_key_was_down = gi_mode_key_down;
        if !self.shadow_test_scene && !self.room_test_scene {
            self.update_cube(context, delta_seconds)?;
        }
        if !self.shadow_test_scene && !self.room_test_scene {
            self.update_sphere(context, delta_seconds)?;
        }
        self.update_render_effects(context);
        let movement_keys_active = self.debug_panels.as_ref().is_some_and(DebugPanels::movement_keys_active);
        self.update_camera(context, delta_seconds, movement_keys_active)?;
        self.update_light(context, delta_seconds)
    }
}

fn main() -> Result<(), String> {
    let mut settings = WindowSettings::default();
    let mut large_scene = false;
    let mut gi_test_scene = false;
    let mut open_world = false;
    let mut shadow_test_scene = false;
    let mut room_test_scene = false;
    let mut shadow_isolation = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--windowed" => settings.mode = WindowMode::Windowed,
            "--borderless" => settings.mode = WindowMode::Borderless,
            "--fullscreen" => settings.mode = WindowMode::Exclusive,
            "--vulkan" => settings.graphics_api = GraphicsApi::Vulkan,
            "--dx12" => settings.graphics_api = GraphicsApi::DirectX12,
            "--opengl" => settings.graphics_api = GraphicsApi::OpenGl,
            "--metal" => settings.graphics_api = GraphicsApi::Metal,
            "--large-scene" => large_scene = true,
            "--gi-test" => gi_test_scene = true,
            "--open-world" => open_world = true,
            "--shadow-test" => shadow_test_scene = true,
            "--room-test" => room_test_scene = true,
            "--shadow-isolation" => shadow_isolation = true,
            argument if argument.starts_with("--capture-script=") => settings.debug_script = Some(argument.trim_start_matches("--capture-script=").into()),
            _ => return Err(format!("unknown option: {argument}")),
        }
    }

    run(
        settings,
        LightingDemo {
            cube: None,
            sphere: None,
            camera: None,
            light: None,
            fill_light: None,
            point_light: None,
            spot_light: None,
            irradiance_volume: None,
            debug_panels: None,
            cube_rotation: Quat::IDENTITY,
            sphere_time: 0.0,
            camera_yaw: 0.58,
            camera_pitch: -0.33,
            light_angle: 0.0,
            font: None,
            large_scene,
            gi_test_scene,
            open_world,
            shadow_test_scene,
            room_test_scene,
            shadow_isolation,
            gi_mode_key_was_down: false,
        },
    )
}
