use std::path::Path;

use labyrinth::{
    Application, Camera, CameraId, Color, DirectionalLight, Easing, EulerRot, FontAsset, GraphicsApi, IrradianceMode, IrradianceVolumeDesc, IrradianceVolumeId, KeyCode, LightId, Material, MeshId,
    PointLight, Quat, RenderDebugMode, RendererSkyLighting, RuntimeContext, SpotLight, Transform, UiAnchor, UiAutoLayout, UiButtonId, UiContainerId, UiGraphId, UiLabelId, UiLayout, UiPanelId,
    UiSliderId, Vec2, Vec3, WindowMode, WindowSettings, ease, ping_pong, primitives, run,
};

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
    inspector_header_button: Option<UiButtonId>,
    inspector_content: Option<UiContainerId>,
    display_mode_drawer: InspectorDrawer,
    render_options_drawer: InspectorDrawer,
    gizmos_drawer: InspectorDrawer,
    lit_materials_button: Option<UiButtonId>,
    unlit_materials_button: Option<UiButtonId>,
    wireframe_button: Option<UiButtonId>,
    shadows_button: Option<UiButtonId>,
    irradiance_button: Option<UiButtonId>,
    gizmos_button: Option<UiButtonId>,
    normals_gizmos_button: Option<UiButtonId>,
    lights_gizmos_button: Option<UiButtonId>,
    global_probe_gizmos_button: Option<UiButtonId>,
    screen_probe_gizmos_button: Option<UiButtonId>,
    gi_trace_gizmos_button: Option<UiButtonId>,
    sdf_occupancy_gizmos_button: Option<UiButtonId>,
    near_radiance_field_button: Option<UiButtonId>,
    far_radiance_field_button: Option<UiButtonId>,
    radiance_cell_sliders: [Option<UiSliderId>; 3],
    radiance_cell_label: Option<UiLabelId>,
    render_mode: RenderDebugMode,
    performance_label: Option<UiLabelId>,
    performance_frame_label: Option<UiLabelId>,
    performance_graph: Option<UiGraphId>,
    performance_panel: Option<UiPanelId>,
    performance_header_button: Option<UiButtonId>,
    frame_history: Vec<f32>,
    smoothed_fps: f32,
    cube_rotation: Quat,
    sphere_time: f32,
    camera_yaw: f32,
    camera_pitch: f32,
    light_angle: f32,
    font: Option<FontAsset>,
}

#[derive(Clone, Copy, Default)]
struct InspectorDrawer {
    header: Option<UiButtonId>,
    content: Option<UiContainerId>,
    content_height: f32,
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
        let panel_size = Vec2::new(320.0, 680.0);
        let panel_layout = UiLayout::anchored(panel_size, UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(24.0, 24.0));
        context.ui.set_window_layout(panel_layout);
        let inspector_header_button =
            context.ui.add_toggle_button_layout(None, UiLayout::anchored(Vec2::new(panel_size.x, 26.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(24.0, 24.0)), false);
        context.ui.set_button_text(inspector_header_button, "Lighting Inspector", 15.0, Color::WHITE);
        context.ui.set_button_text_centered(inspector_header_button, false);
        context.ui.set_button_toggled_colors(inspector_header_button, Color::rgba(0.1, 0.12, 0.17, 1.0), Color::rgba(0.1, 0.12, 0.17, 1.0));

        let stack_size = Vec2::new(panel_size.x, panel_size.y - 44.0);
        let stack_layout = UiLayout::anchored(stack_size, UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(24.0, 50.0));
        let stack = context.ui.add_container(None, stack_layout, UiAutoLayout::column(10.0, 3.0));
        let display_mode_drawer = add_inspector_drawer(context, stack, "DISPLAY MODE", true, 22.0);
        let mode_row = context.ui.add_container(display_mode_drawer.content, UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 6.0));
        self.lit_materials_button = Some(add_inspector_mode_button(context, mode_row, "Lit", true));
        self.unlit_materials_button = Some(add_inspector_mode_button(context, mode_row, "Unlit", false));
        self.wireframe_button = Some(add_inspector_mode_button(context, mode_row, "Wire", false));

        let render_options_drawer = add_inspector_drawer(context, stack, "RENDER OPTIONS", true, 39.0);
        self.irradiance_button = Some(add_inspector_checkbox(context, render_options_drawer.content.unwrap(), "Global illumination", true));
        self.shadows_button = Some(add_inspector_checkbox(context, render_options_drawer.content.unwrap(), "Shadows", true));

        let gizmos_drawer = add_inspector_drawer(context, stack, "GIZMOS", true, 380.0);
        let gizmos_content = gizmos_drawer.content.unwrap();
        self.gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "Show gizmos", context.gizmos_enabled));
        self.normals_gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "Normals", context.normal_gizmos_enabled));
        self.lights_gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "Lights", context.light_gizmos_enabled));
        self.global_probe_gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "GI global probes", context.gi_global_gizmos_enabled));
        self.screen_probe_gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "Screen space probes", context.gi_gizmos_enabled));
        self.gi_trace_gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "Selected GI cell rays", context.gi_trace_gizmos_enabled));
        add_inspector_color_legend(context, gizmos_content, "GEOMETRY", &[("HIT", Color::rgb(0.1, 1.0, 0.2)), ("MISS", Color::rgb(1.0, 0.5, 0.08)), ("INVALID", Color::rgb(1.0, 0.08, 0.05))]);
        add_inspector_color_legend(
            context,
            gizmos_content,
            "LIGHTING",
            &[
                ("DIRECT", Color::rgb(1.0, 0.86, 0.12)),
                ("SHADOWED", Color::rgb(0.12, 0.45, 1.0)),
                ("EMISSIVE", Color::rgb(1.0, 0.12, 0.85)),
                ("SKY", Color::rgb(0.1, 0.9, 1.0)),
                ("UNLIT", Color::rgb(0.68, 0.72, 0.78)),
            ],
        );
        add_inspector_color_legend(
            context,
            gizmos_content,
            "TRACE STOP",
            &[("HIT", Color::rgb(0.1, 1.0, 0.2)), ("STEPS", Color::rgb(1.0, 0.12, 0.85)), ("DIST", Color::rgb(1.0, 0.78, 0.08)), ("GRID EXIT", Color::rgb(0.25, 0.5, 1.0))],
        );
        add_inspector_color_legend(context, gizmos_content, "LAST SDF d", &[("0 m", Color::rgb(0.08, 0.2, 1.0)), ("1 m", Color::rgb(0.54, 0.16, 0.52)), ("2 m+", Color::rgb(1.0, 0.12, 0.04))]);
        add_inspector_section(context, gizmos_content, "GI FIELD CELL");
        let field_row = context.ui.add_container(Some(gizmos_content), UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 6.0));
        self.near_radiance_field_button = Some(add_inspector_mode_button(context, field_row, "Near", !context.gi_selected_radiance_field_far));
        self.far_radiance_field_button = Some(add_inspector_mode_button(context, field_row, "Far", context.gi_selected_radiance_field_far));
        self.radiance_cell_label = Some(context.ui.add_label_layout(Some(gizmos_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Cell (4, 1, 4)", 11.0, Color::WHITE));
        self.radiance_cell_sliders = [
            Some(add_inspector_slider(context, gizmos_content, "X index 0–7", 7.0, context.gi_selected_radiance_cell[0] as f32)),
            Some(add_inspector_slider(context, gizmos_content, "Y index 0–3", 3.0, context.gi_selected_radiance_cell[1] as f32)),
            Some(add_inspector_slider(context, gizmos_content, "Z index 0–7", 7.0, context.gi_selected_radiance_cell[2] as f32)),
        ];
        self.sdf_occupancy_gizmos_button = Some(add_inspector_checkbox(context, gizmos_content, "SDF occupied voxels", context.sdf_occupancy_gizmos_enabled));
        self.inspector_header_button = Some(inspector_header_button);
        self.inspector_content = Some(stack);
        self.display_mode_drawer = display_mode_drawer;
        self.render_options_drawer = render_options_drawer;
        self.gizmos_drawer = gizmos_drawer;
        let performance_panel_layout = UiLayout::anchored(Vec2::new(320.0, 180.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-24.0, 24.0));
        self.performance_panel = Some(context.ui.add_panel(performance_panel_layout, Color::rgba(0.03, 0.04, 0.06, 0.94)));
        let performance_header_button = context.ui.add_toggle_button_layout(None, UiLayout::anchored(Vec2::new(320.0, 26.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-24.0, 24.0)), false);
        context.ui.set_button_text(performance_header_button, "Performance", 15.0, Color::WHITE);
        context.ui.set_button_text_centered(performance_header_button, false);
        context.ui.set_button_toggled_colors(performance_header_button, Color::rgba(0.1, 0.12, 0.17, 1.0), Color::rgba(0.1, 0.12, 0.17, 1.0));
        self.performance_header_button = Some(performance_header_button);
        self.performance_label = Some(context.ui.add_label_layout(
            None,
            UiLayout::anchored(Vec2::new(300.0, 20.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-36.0, 58.0)),
            "CPU --",
            11.0,
            Color::rgb(0.62, 0.68, 0.78),
        ));
        self.performance_frame_label = Some(context.ui.add_label_layout(
            None,
            UiLayout::anchored(Vec2::new(280.0, 20.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-36.0, 82.0)),
            "FPS -- | Frame -- ms",
            12.0,
            Color::WHITE,
        ));
        self.performance_graph = Some(context.ui.add_graph(
            UiLayout::anchored(Vec2::new(300.0, 72.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-36.0, 108.0)),
            Color::rgb(0.25, 0.75, 1.0),
            Color::rgba(0.05, 0.07, 0.1, 0.95),
        ));
        Ok(())
    }

    fn update_performance_label(&mut self, context: &mut RuntimeContext) {
        let Some(label) = self.performance_label else { return };
        let stats = context.performance_stats;
        let text = format!("CPU {:.1} ms | SDF {:.2}/{:.2} ms | GI {:.2} ms", stats.frame_cpu_ms, stats.sdf_build_cpu_ms, stats.sdf_upload_ms, stats.gpu_sdf_gi_ms);
        context.ui.set_label_text(label, text);
        let delta = context.time.delta_seconds().max(0.0001);
        let fps = 1.0 / delta;
        self.smoothed_fps = if self.smoothed_fps == 0.0 { fps } else { self.smoothed_fps + (fps - self.smoothed_fps) * 0.1 };
        self.frame_history.push(delta * 1000.0);
        if self.frame_history.len() > 48 {
            self.frame_history.remove(0);
        }
        if let Some(label) = self.performance_frame_label {
            context.ui.set_label_text(label, format!("FPS {:.1} | Frame {:.2} ms", self.smoothed_fps, delta * 1000.0));
        }
        if let Some(graph) = self.performance_graph {
            context.ui.set_graph_values(graph, &self.frame_history);
        }
    }

    fn update_render_mode(&mut self, context: &mut RuntimeContext) {
        let buttons = [self.lit_materials_button, self.unlit_materials_button, self.wireframe_button];
        let modes = [RenderDebugMode::LitMaterials, RenderDebugMode::UnlitMaterials, RenderDebugMode::Wireframe];
        let mut selected = self.render_mode;
        for (button, mode) in buttons.into_iter().zip(modes) {
            if let Some(button) = button
                && context.ui.button_toggled(button).unwrap_or(false)
                && mode != self.render_mode
            {
                selected = mode;
            }
        }
        self.render_mode = selected;
        context.render_debug_mode = selected;
        for (button, mode) in buttons.into_iter().zip(modes) {
            if let Some(button) = button {
                context.ui.set_button_toggled(button, mode == selected);
            }
        }
    }

    fn update_render_effects(&mut self, context: &mut RuntimeContext) {
        update_inspector_drawer(context, self.display_mode_drawer, "DISPLAY MODE");
        update_inspector_drawer(context, self.render_options_drawer, "RENDER OPTIONS");
        update_inspector_drawer(context, self.gizmos_drawer, "GIZMOS");
        let panel_collapsed = self.inspector_header_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        if let Some(content) = self.inspector_content {
            context.ui.set_container_visible(content, !panel_collapsed);
        }
        let drawers = [self.display_mode_drawer, self.render_options_drawer, self.gizmos_drawer];
        let expanded_drawer_count = drawers.iter().filter(|drawer| drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false)).count();
        let expanded_content_height =
            drawers.iter().filter(|drawer| drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false)).map(|drawer| drawer.content_height).sum::<f32>();
        let stack_child_count = drawers.len() + expanded_drawer_count;
        let stack_height = 20.0 + drawers.len() as f32 * 22.0 + expanded_content_height + 3.0 * stack_child_count.saturating_sub(1) as f32;
        let panel_height = if panel_collapsed { 26.0 } else { 26.0 + stack_height };
        context.ui.set_window_layout(UiLayout::anchored(Vec2::new(320.0, panel_height), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(24.0, 24.0)));

        let performance_collapsed = self.performance_header_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        let performance_height = if performance_collapsed { 26.0 } else { 180.0 };
        if let Some(panel) = self.performance_panel {
            context.ui.set_panel_layout(panel, UiLayout::anchored(Vec2::new(320.0, performance_height), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-24.0, 24.0)));
        }
        if let Some(label) = self.performance_label {
            context.ui.set_label_visible(label, !performance_collapsed);
        }
        if let Some(label) = self.performance_frame_label {
            context.ui.set_label_visible(label, !performance_collapsed);
        }
        if let Some(graph) = self.performance_graph {
            context.ui.set_graph_visible(graph, !performance_collapsed);
        }

        let mut settings = context.render_settings;
        settings.shadows_enabled = self.shadows_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(true);
        settings.irradiance_enabled = self.irradiance_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(true);
        context.render_settings = settings;
        context.gizmos_enabled = self.gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        context.normal_gizmos_enabled = self.normals_gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        context.light_gizmos_enabled = self.lights_gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        context.gi_global_gizmos_enabled = self.global_probe_gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        context.gi_gizmos_enabled = self.screen_probe_gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        context.gi_trace_gizmos_enabled = self.gi_trace_gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        context.sdf_occupancy_gizmos_enabled = self.sdf_occupancy_gizmos_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        for (index, slider) in self.radiance_cell_sliders.into_iter().enumerate() {
            if let Some(slider) = slider {
                context.gi_selected_radiance_cell[index] = context.ui.slider_value(slider).unwrap_or(0.0).round() as u32;
            }
        }
        let mut selected_far_field = context.gi_selected_radiance_field_far;
        let near_toggled = self.near_radiance_field_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(!selected_far_field);
        let far_toggled = self.far_radiance_field_button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(selected_far_field);
        let near_changed = near_toggled != !selected_far_field;
        let far_changed = far_toggled != selected_far_field;
        if near_changed && near_toggled {
            selected_far_field = false;
        } else if far_changed && far_toggled {
            selected_far_field = true;
        }
        context.gi_selected_radiance_field_far = selected_far_field;
        if let Some(label) = self.radiance_cell_label {
            let field_name = if selected_far_field { "Far" } else { "Near" };
            context
                .ui
                .set_label_text(label, format!("{field_name} cell ({}, {}, {})", context.gi_selected_radiance_cell[0], context.gi_selected_radiance_cell[1], context.gi_selected_radiance_cell[2]));
        }
        if let Some(button) = self.near_radiance_field_button {
            context.ui.set_button_toggled(button, !selected_far_field);
        }
        if let Some(button) = self.far_radiance_field_button {
            context.ui.set_button_toggled(button, selected_far_field);
        }
        for button in [
            self.shadows_button,
            self.irradiance_button,
            self.gizmos_button,
            self.normals_gizmos_button,
            self.lights_gizmos_button,
            self.global_probe_gizmos_button,
            self.screen_probe_gizmos_button,
            self.gi_trace_gizmos_button,
            self.sdf_occupancy_gizmos_button,
        ]
        .into_iter()
        .flatten()
        {
            set_checkbox_mark(context, button);
        }
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
        if self.room_test_scene {
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
        self.update_camera(context, delta_seconds)?;
        self.update_render_mode(context);
        self.update_render_effects(context);
        self.update_performance_label(context);
        self.update_light(context, delta_seconds)
    }
}

fn add_inspector_drawer(context: &mut RuntimeContext, parent: UiContainerId, title: &str, expanded: bool, content_height: f32) -> InspectorDrawer {
    let header = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 22.0)), expanded);
    let content = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, content_height)), UiAutoLayout::column(0.0, 3.0));
    context.ui.set_container_visible(content, expanded);
    context.ui.set_button_text_centered(header, false);
    context.ui.set_button_toggled_colors(header, Color::rgb(0.15, 0.2, 0.28), Color::rgb(0.1, 0.12, 0.17));
    let drawer = InspectorDrawer { header: Some(header), content: Some(content), content_height };
    update_inspector_drawer(context, drawer, title);
    drawer
}

fn update_inspector_drawer(context: &mut RuntimeContext, drawer: InspectorDrawer, title: &str) {
    let Some(header) = drawer.header else { return };
    let expanded = context.ui.button_toggled(header).unwrap_or(false);
    if let Some(content) = drawer.content {
        context.ui.set_container_visible(content, expanded);
    }
    context.ui.set_button_text(header, title, 12.0, Color::WHITE);
}

fn add_inspector_section(context: &mut RuntimeContext, parent: UiContainerId, text: &str) {
    context.ui.add_label_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 15.0)), text, 11.0, Color::rgb(0.42, 0.7, 0.96));
}

fn add_inspector_checkbox(context: &mut RuntimeContext, parent: UiContainerId, name: &str, enabled: bool) -> UiButtonId {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 18.0)), UiAutoLayout::row(0.0, 8.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 17.0)), name, 12.0, Color::WHITE);
    let button = context.ui.add_toggle_button_layout(Some(row), UiLayout::new(Vec2::new(15.0, 15.0)), enabled);
    set_checkbox_mark(context, button);
    button
}

fn add_inspector_color_legend(context: &mut RuntimeContext, parent: UiContainerId, label: &str, entries: &[(&str, Color)]) {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 15.0)), UiAutoLayout::row(0.0, 3.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(54.0, 15.0)), label, 11.0, Color::rgb(0.72, 0.76, 0.82));
    for (text, color) in entries {
        let width = (text.len() as f32 * 6.2).max(26.0);
        context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(width, 15.0)), *text, 11.0, *color);
    }
}

fn add_inspector_slider(context: &mut RuntimeContext, parent: UiContainerId, name: &str, maximum: f32, value: f32) -> UiSliderId {
    context.ui.add_label_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 13.0)), name, 10.0, Color::rgb(0.72, 0.76, 0.82));
    context.ui.add_slider_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 13.0)), 0.0, maximum, value)
}

fn add_inspector_mode_button(context: &mut RuntimeContext, parent: UiContainerId, name: &str, selected: bool) -> UiButtonId {
    let button = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(68.0, 22.0)), selected);
    context.ui.set_button_text(button, name, 11.0, Color::WHITE);
    button
}

fn set_checkbox_mark(context: &mut RuntimeContext, button: UiButtonId) {
    let text = if context.ui.button_toggled(button).unwrap_or(false) { "✓" } else { "" };
    context.ui.set_button_text(button, text, 12.0, Color::WHITE);
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
            lit_materials_button: None,
            unlit_materials_button: None,
            wireframe_button: None,
            shadows_button: None,
            irradiance_button: None,
            gizmos_button: None,
            normals_gizmos_button: None,
            lights_gizmos_button: None,
            global_probe_gizmos_button: None,
            screen_probe_gizmos_button: None,
            gi_trace_gizmos_button: None,
            sdf_occupancy_gizmos_button: None,
            near_radiance_field_button: None,
            far_radiance_field_button: None,
            radiance_cell_sliders: [None; 3],
            radiance_cell_label: None,
            render_mode: RenderDebugMode::LitMaterials,
            performance_label: None,
            performance_frame_label: None,
            performance_graph: None,
            performance_panel: None,
            performance_header_button: None,
            frame_history: Vec::new(),
            smoothed_fps: 0.0,
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
            inspector_header_button: None,
            inspector_content: None,
            display_mode_drawer: InspectorDrawer::default(),
            render_options_drawer: InspectorDrawer::default(),
            gizmos_drawer: InspectorDrawer::default(),
        },
    )
}
