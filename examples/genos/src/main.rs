mod building;

use std::path::Path;

use building::{Building, LampSlot, MovingBox, distributed_lamps};
use labyrinth::{
    Application, Camera, CameraId, Color, DirectionalLight, EulerRot, FontAsset, IrradianceMode, KeyCode, LightId, PointLight, Quat, RenderDebugMode, RuntimeContext, Transform, UiAnchor,
    UiAutoLayout, UiButtonId, UiContainerId, UiLabelId, UiLayout, UiSliderId, Vec2, Vec3, WindowSettings, primitives, run,
};

const MAX_POINT_LIGHTS: usize = 7;
const LAMP_TOTAL: f32 = 1.0;
const DAY_SECONDS: f32 = 120.0;
const LIGHT_COUNTS: [usize; 4] = [1, 3, 5, 7];
const DYNAMIC_PERCENTAGES: [u32; 3] = [0, 50, 100];
const SUN_SPEEDS: [f32; 4] = [0.0, 1.0, 4.0, 16.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum LampLayout {
    Spread,
    FirstSection,
}

#[derive(Default)]
struct InspectorButtons {
    lights: Vec<UiButtonId>,
    dynamic: Vec<UiButtonId>,
    layout: Vec<UiButtonId>,
    scale: Vec<UiButtonId>,
    sun: Vec<UiButtonId>,
    sky: Vec<UiButtonId>,
    speed: Vec<UiButtonId>,
    time_of_day: Option<UiSliderId>,
    time_of_day_value: Option<UiLabelId>,
    boxes: Vec<UiButtonId>,
    display_mode: Vec<UiButtonId>,
    irradiance: Option<UiButtonId>,
    surface_probe_gi: Option<UiButtonId>,
    shadows: Option<UiButtonId>,
    freeze_gi: Option<UiButtonId>,
    gizmos: Option<UiButtonId>,
    normals: Option<UiButtonId>,
    lights_gizmos: Option<UiButtonId>,
    global_probes: Option<UiButtonId>,
    screen_probes: Option<UiButtonId>,
    gi_traces: Option<UiButtonId>,
}

struct LampEntity {
    id: LightId,
    slot: LampSlot,
    moving: bool,
}

struct MovingBoxEntity {
    id: labyrinth::MeshId,
    motion: MovingBox,
}

struct GenosDemo {
    cols: u32,
    rows: u32,
    light_count: usize,
    dynamic_percent: u32,
    layout: LampLayout,
    sun_speed: f32,
    sun_frozen: bool,
    sky_on: bool,
    boxes_still: bool,
    seed: u64,
    time: f32,
    box_time: f32,
    day: f32,
    surface_probe_gi_enabled: bool,
    yaw: f32,
    pitch: f32,
    camera: Option<CameraId>,
    sun: Option<LightId>,
    lamps: Vec<LampEntity>,
    moving_boxes: Vec<MovingBoxEntity>,
    buttons: InspectorButtons,
    render_mode: RenderDebugMode,
}

impl Application for GenosDemo {
    fn setup(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        context.irradiance_mode = IrradianceMode::AabbSdf;
        context.render_settings.ambient_intensity = 0.0;
        context.render_settings.surface_probes_enabled = self.surface_probe_gi_enabled;
        self.build_scene(context);
        self.setup_ui(context)
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let delta_seconds = context.time.delta_seconds().min(0.25);
        self.time += delta_seconds;
        if !self.boxes_still {
            self.box_time += delta_seconds;
        }
        self.update_inspector(context)?;
        if !self.sun_frozen {
            self.day = (self.day + delta_seconds * self.sun_speed / DAY_SECONDS).rem_euclid(1.0);
        }
        self.update_sun(context);
        self.update_camera(context, delta_seconds)?;

        for lamp in &self.lamps {
            let point_light = context.scene.get_point_light_mut(lamp.id).ok_or("point light disappeared from the scene")?;
            point_light.position = if lamp.moving { lamp.slot.home + lamp.slot.motion.offset(self.time) } else { lamp.slot.home };
        }
        for moving_box in &self.moving_boxes {
            let (position, yaw) = moving_box.motion.pose(self.box_time);
            let mesh = context.scene.get_mesh_mut(moving_box.id).ok_or("moving box disappeared from the scene")?;
            mesh.transform = Transform::new(position, Quat::from_rotation_y(yaw), moving_box.motion.size);
        }
        Ok(())
    }
}

impl GenosDemo {
    fn build_scene(&mut self, context: &mut RuntimeContext) {
        let building = Building::new(self.cols, self.rows, self.seed);
        let lamp_slots = self.lamp_slots(&building, MAX_POINT_LIGHTS);
        let (floor_width, floor_depth) = building.floor_size();
        let floor = primitives::quad(floor_width, floor_depth, 0.0, Color::WHITE);
        context.scene.spawn_static_mesh(floor, Transform::from_position(building.floor_center()));

        for object in &building.static_boxes {
            let mesh = if object.cylinder { primitives::cylinder([0.0; 3], object.size.x * 0.5, object.size.y, object.color, 16) } else { primitives::cube([0.0; 3], 1.0, object.color) };
            let scale = if object.cylinder { Vec3::ONE } else { object.size };
            context.scene.spawn_static_mesh(mesh, Transform::new(object.center, Quat::IDENTITY, scale));
        }

        for motion in building.moving_boxes {
            let mesh = primitives::cube([0.0; 3], 1.0, motion.color);
            let (position, yaw) = motion.pose(0.0);
            let transform = Transform::new(position, Quat::from_rotation_y(yaw), motion.size);
            let id = context.scene.spawn_mesh(mesh, transform);
            self.moving_boxes.push(MovingBoxEntity { id, motion });
        }

        let mut camera = Camera { clear_color: Color::rgb(0.0, 0.0, 0.0), ..Camera::default() };
        camera.transform.position = Vec3::new(12.5, 1.7, 31.0);
        camera.look_at(Vec3::new(12.5, 1.6, 16.0));
        self.yaw = self.yaw_from_camera(camera.transform.rotation);
        self.pitch = -0.04;
        let camera_id = context.scene.spawn_camera(camera);
        context.active_camera = Some(camera_id);
        self.camera = Some(camera_id);

        let sun = context.scene.spawn_light(DirectionalLight::default());
        self.sun = Some(sun);
        context.active_lights.push(sun);

        let moving_count = ((self.light_count as f32 * self.dynamic_percent as f32 / 100.0).round() as usize).min(self.light_count);
        let moving_set = moving_set(self.light_count, moving_count);
        let intensity = LAMP_TOTAL / self.light_count as f32;
        for (index, slot) in lamp_slots.into_iter().enumerate() {
            let range = lamp_range(intensity);
            let light = PointLight { position: slot.home, color: slot.tint, intensity, range };
            let id = context.scene.spawn_point_light(light);
            self.lamps.push(LampEntity { id, slot, moving: moving_set.get(index).copied().unwrap_or(false) });
        }
        self.apply_active_lights(context);

        eprintln!(
            "Genos stress scene: {}×{} sections, {} static props, {} moving boxes, {} point lights ({} moving)",
            self.cols,
            self.rows,
            building.static_boxes.len(),
            self.moving_boxes.len(),
            self.light_count,
            moving_count
        );
        self.update_sun(context);
    }

    fn setup_ui(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../lighting/assets/fonts/default.ttf");
        context.ui.set_font(FontAsset::from_file(font_path)?);
        let panel_size = Vec2::new(420.0, 640.0);
        context.ui.set_window_layout(UiLayout::anchored(panel_size, UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(20.0, 20.0)));
        context.ui.add_label_layout(None, UiLayout::anchored(Vec2::new(320.0, 26.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(30.0, 22.0)), "Genos Stress Inspector", 15.0, Color::WHITE);
        let stack = context.ui.add_container(
            None,
            UiLayout::anchored(Vec2::new(panel_size.x, panel_size.y - 42.0), UiAnchor::TOP_LEFT, UiAnchor::TOP_LEFT, Vec2::new(20.0, 48.0)),
            UiAutoLayout::column(8.0, 2.0),
        );

        add_inspector_section(context, stack, "STRESS SCENE");
        self.buttons.lights = add_inspector_mode_row(context, stack, "Point lights", &["1", "3", "5", "7"], light_mode_index(self.light_count));
        self.buttons.dynamic =
            add_inspector_mode_row(context, stack, "Moving lights", &["0%", "50%", "100%"], DYNAMIC_PERCENTAGES.iter().position(|percent| *percent == self.dynamic_percent).unwrap_or(1));
        self.buttons.layout = add_inspector_mode_row(context, stack, "Lamp layout", &["Spread", "Section 0"], usize::from(self.layout == LampLayout::FirstSection));
        self.buttons.scale = add_inspector_mode_row(context, stack, "Scene scale", &["Small", "Big"], usize::from(self.cols > 1));
        self.buttons.sun = add_inspector_mode_row(context, stack, "Sun", &["Run", "Freeze"], usize::from(self.sun_frozen));
        self.buttons.sky = add_inspector_mode_row(context, stack, "Sky", &["On", "Off"], usize::from(!self.sky_on));
        self.buttons.speed = add_inspector_mode_row(context, stack, "Sun speed", &["x0", "x1", "x4", "x16"], SUN_SPEEDS.iter().position(|speed| *speed == self.sun_speed).unwrap_or(1));
        let (time_slider, time_label) = add_inspector_slider_row(context, stack, "Time of day", 0.0, 24.0, self.day * 24.0);
        self.buttons.time_of_day = Some(time_slider);
        self.buttons.time_of_day_value = Some(time_label);
        self.buttons.boxes = add_inspector_mode_row(context, stack, "Boxes", &["Move", "Still"], usize::from(self.boxes_still));

        add_inspector_section(context, stack, "DISPLAY MODE");
        let display = context.ui.add_container(Some(stack), UiLayout::new(Vec2::new(0.0, 24.0)), UiAutoLayout::row(0.0, 6.0));
        self.buttons.display_mode = [
            add_inspector_mode_button(context, display, "Lit", self.render_mode == RenderDebugMode::LitMaterials),
            add_inspector_mode_button(context, display, "Unlit", self.render_mode == RenderDebugMode::UnlitMaterials),
            add_inspector_mode_button(context, display, "Wire", self.render_mode == RenderDebugMode::Wireframe),
        ]
        .into();

        add_inspector_section(context, stack, "RENDER OPTIONS");
        self.buttons.irradiance = Some(add_inspector_checkbox(context, stack, "Global illumination", context.render_settings.irradiance_enabled));
        self.buttons.surface_probe_gi = Some(add_inspector_checkbox(context, stack, "Screen probe GI", context.render_settings.surface_probes_enabled));
        self.buttons.shadows = Some(add_inspector_checkbox(context, stack, "Shadows", context.render_settings.shadows_enabled));
        self.buttons.freeze_gi = Some(add_inspector_checkbox(context, stack, "Freeze GI radiance", context.freeze_gi_radiance));

        add_inspector_section(context, stack, "GIZMOS");
        self.buttons.gizmos = Some(add_inspector_checkbox(context, stack, "Show gizmos", context.gizmos_enabled));
        self.buttons.normals = Some(add_inspector_checkbox(context, stack, "Normals", context.normal_gizmos_enabled));
        self.buttons.lights_gizmos = Some(add_inspector_checkbox(context, stack, "Lights", context.light_gizmos_enabled));
        self.buttons.global_probes = Some(add_inspector_checkbox(context, stack, "GI global probes", context.gi_global_gizmos_enabled));
        self.buttons.screen_probes = Some(add_inspector_checkbox(context, stack, "Screen space probes", context.gi_gizmos_enabled));
        self.buttons.gi_traces = Some(add_inspector_checkbox(context, stack, "GI SDF ray traces", context.gi_trace_gizmos_enabled));
        Ok(())
    }

    fn lamp_slots(&self, building: &Building, count: usize) -> Vec<LampSlot> {
        match self.layout {
            LampLayout::Spread => distributed_lamps(building, count),
            LampLayout::FirstSection => building.lamps.iter().take(count).copied().collect(),
        }
    }

    fn apply_active_lights(&self, context: &mut RuntimeContext) {
        context.active_lights.clear();
        if let Some(sun) = self.sun {
            context.active_lights.push(sun);
        }
        context.active_lights.extend(self.lamps.iter().take(self.light_count).map(|lamp| lamp.id));
    }

    fn update_sun(&mut self, context: &mut RuntimeContext) {
        let hour_angle = std::f32::consts::TAU * (self.day - 0.25);
        let elevation = hour_angle.sin();
        let horizontal = (1.0 - elevation * elevation).sqrt();
        let fade = (elevation / 0.1).clamp(0.0, 1.0);
        if let Some(sun_id) = self.sun
            && let Some(sun) = context.scene.get_directional_light_mut(sun_id)
        {
            sun.direction = Vec3::new(-hour_angle.cos() * horizontal, elevation, -hour_angle.sin() * horizontal).normalize();
            sun.color = Color::rgb(1.0, 0.95, 0.85);
            sun.intensity = 1.8 * fade;
        }
        let sky = ((elevation + 0.1) / 0.6).clamp(0.0, 1.0) * if self.sky_on { 1.0 } else { 0.0 };
        context.sky_lighting.color = Color::rgb(0.11, 0.14, 0.19);
        context.sky_lighting.intensity = 0.65 * sky;
        if let Some(slider) = self.buttons.time_of_day {
            context.ui.set_slider_value(slider, self.day * 24.0);
        }
        if let Some(label) = self.buttons.time_of_day_value {
            context.ui.set_label_text(label, format!("{:04.1} h", self.day * 24.0));
        }
    }

    fn update_inspector(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        if let Some(index) = select_new_button(context, &self.buttons.lights, light_mode_index(self.light_count)) {
            self.light_count = LIGHT_COUNTS[index];
            let intensity = LAMP_TOTAL / self.light_count as f32;
            for lamp in &self.lamps {
                let point_light = context.scene.get_point_light_mut(lamp.id).ok_or("point light disappeared from the scene")?;
                point_light.intensity = intensity;
                point_light.range = lamp_range(intensity);
            }
            self.apply_active_lights(context);
        }
        if let Some(index) = select_new_button(context, &self.buttons.dynamic, DYNAMIC_PERCENTAGES.iter().position(|percent| *percent == self.dynamic_percent).unwrap_or(0)) {
            self.dynamic_percent = DYNAMIC_PERCENTAGES[index];
            let moving_count = (self.light_count * self.dynamic_percent as usize + 50) / 100;
            let moving = moving_set(self.light_count, moving_count);
            for (index, lamp) in self.lamps.iter_mut().enumerate() {
                lamp.moving = moving.get(index).copied().unwrap_or(false);
            }
        }
        if select_new_button(context, &self.buttons.layout, usize::from(self.layout == LampLayout::FirstSection)).is_some() {
            self.layout = if button_selected(context, &self.buttons.layout[1]) { LampLayout::FirstSection } else { LampLayout::Spread };
            let building = Building::new(self.cols, self.rows, self.seed);
            let slots = self.lamp_slots(&building, MAX_POINT_LIGHTS);
            let moving_count = (self.light_count * self.dynamic_percent as usize + 50) / 100;
            let moving = moving_set(self.light_count, moving_count);
            for (index, (lamp, slot)) in self.lamps.iter_mut().zip(slots).enumerate() {
                lamp.slot = slot;
                lamp.moving = moving.get(index).copied().unwrap_or(false);
            }
        }
        if select_new_button(context, &self.buttons.scale, usize::from(self.cols > 1)).is_some() {
            let wants_big = button_selected(context, &self.buttons.scale[1]);
            let (cols, rows) = if wants_big { (4, 4) } else { (1, 1) };
            if (self.cols, self.rows) != (cols, rows) {
                self.cols = cols;
                self.rows = rows;
                context.scene = Default::default();
                context.active_camera = None;
                context.active_lights.clear();
                self.camera = None;
                self.sun = None;
                self.lamps.clear();
                self.moving_boxes.clear();
                self.build_scene(context);
            }
        }
        if let Some(index) = select_new_button(context, &self.buttons.sun, usize::from(self.sun_frozen)) {
            self.sun_frozen = index == 1;
        }
        if let Some(slider) = self.buttons.time_of_day {
            let time_of_day = context.ui.slider_value(slider).unwrap_or(self.day * 24.0);
            if (time_of_day - self.day * 24.0).abs() > 0.01 {
                self.day = (time_of_day / 24.0).rem_euclid(1.0);
                self.sun_frozen = true;
            }
        }
        if let Some(index) = select_new_button(context, &self.buttons.sky, usize::from(!self.sky_on)) {
            self.sky_on = index == 0;
        }
        if let Some(index) = select_new_button(context, &self.buttons.speed, SUN_SPEEDS.iter().position(|speed| *speed == self.sun_speed).unwrap_or(1)) {
            self.sun_speed = SUN_SPEEDS[index];
        }
        if let Some(index) = select_new_button(context, &self.buttons.boxes, usize::from(self.boxes_still)) {
            self.boxes_still = index == 1;
        }
        sync_group(context, &self.buttons.lights, light_mode_index(self.light_count));
        sync_group(context, &self.buttons.dynamic, DYNAMIC_PERCENTAGES.iter().position(|percent| *percent == self.dynamic_percent).unwrap_or(0));
        sync_group(context, &self.buttons.layout, usize::from(self.layout == LampLayout::FirstSection));
        sync_group(context, &self.buttons.scale, usize::from(self.cols > 1));
        sync_group(context, &self.buttons.sun, usize::from(self.sun_frozen));
        sync_group(context, &self.buttons.sky, usize::from(!self.sky_on));
        sync_group(context, &self.buttons.speed, SUN_SPEEDS.iter().position(|speed| *speed == self.sun_speed).unwrap_or(1));
        sync_group(context, &self.buttons.boxes, usize::from(self.boxes_still));
        self.update_debug_options(context);
        Ok(())
    }

    fn update_debug_options(&mut self, context: &mut RuntimeContext) {
        let modes = [RenderDebugMode::LitMaterials, RenderDebugMode::UnlitMaterials, RenderDebugMode::Wireframe];
        if let Some(index) = select_new_button(context, &self.buttons.display_mode, modes.iter().position(|mode| *mode == self.render_mode).unwrap_or(0)) {
            self.render_mode = modes[index];
        }
        context.render_debug_mode = self.render_mode;
        sync_group(context, &self.buttons.display_mode, modes.iter().position(|mode| *mode == self.render_mode).unwrap_or(0));
        let mut settings = context.render_settings;
        settings.irradiance_enabled = read_toggle(context, self.buttons.irradiance, settings.irradiance_enabled);
        settings.surface_probes_enabled = read_toggle(context, self.buttons.surface_probe_gi, settings.surface_probes_enabled);
        settings.shadows_enabled = read_toggle(context, self.buttons.shadows, settings.shadows_enabled);
        context.render_settings = settings;
        context.gizmos_enabled = read_toggle(context, self.buttons.gizmos, context.gizmos_enabled);
        context.normal_gizmos_enabled = read_toggle(context, self.buttons.normals, context.normal_gizmos_enabled);
        context.light_gizmos_enabled = read_toggle(context, self.buttons.lights_gizmos, context.light_gizmos_enabled);
        context.gi_global_gizmos_enabled = read_toggle(context, self.buttons.global_probes, context.gi_global_gizmos_enabled);
        context.gi_gizmos_enabled = read_toggle(context, self.buttons.screen_probes, context.gi_gizmos_enabled);
        context.gi_trace_gizmos_enabled = read_toggle(context, self.buttons.gi_traces, context.gi_trace_gizmos_enabled);
        context.freeze_gi_radiance = read_toggle(context, self.buttons.freeze_gi, context.freeze_gi_radiance);
        for button in [
            self.buttons.irradiance,
            self.buttons.surface_probe_gi,
            self.buttons.shadows,
            self.buttons.freeze_gi,
            self.buttons.gizmos,
            self.buttons.normals,
            self.buttons.lights_gizmos,
            self.buttons.global_probes,
            self.buttons.screen_probes,
            self.buttons.gi_traces,
        ]
        .into_iter()
        .flatten()
        {
            set_checkbox_mark(context, button);
        }
    }

    fn update_camera(&mut self, context: &mut RuntimeContext, delta_seconds: f32) -> Result<(), String> {
        if context.input.cursor_captured() {
            let mouse = context.input.mouse_delta();
            self.yaw -= mouse.x * 0.002;
            self.pitch = (self.pitch - mouse.y * 0.002).clamp(-1.5, 1.5);
        }
        let camera_id = self.camera.ok_or("camera was not initialized")?;
        let camera = context.scene.get_camera_mut(camera_id).ok_or("camera was removed")?;
        let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
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
        if context.input.is_key_down(KeyCode::Space) {
            movement += Vec3::Y;
        }
        if context.input.is_key_down(KeyCode::KeyC) {
            movement -= Vec3::Y;
        }
        let speed = if context.input.is_key_down(KeyCode::ShiftLeft) { 20.0 } else { 5.0 };
        camera.transform.rotation = rotation;
        camera.transform.translate(movement.normalize_or_zero() * speed * delta_seconds);
        Ok(())
    }

    fn yaw_from_camera(&self, rotation: Quat) -> f32 {
        let forward = rotation * Vec3::NEG_Z;
        (-forward.x).atan2(-forward.z)
    }
}

fn add_inspector_section(context: &mut RuntimeContext, parent: UiContainerId, text: &str) {
    context.ui.add_label_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 16.0)), text, 10.0, Color::rgb(0.42, 0.7, 0.96));
}

fn add_inspector_mode_row(context: &mut RuntimeContext, parent: UiContainerId, label: &str, names: &[&str], selected: usize) -> Vec<UiButtonId> {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 24.0)), UiAutoLayout::row(0.0, 5.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(94.0, 20.0)), label, 11.0, Color::WHITE);
    names.iter().enumerate().map(|(index, name)| add_inspector_mode_button(context, row, name, selected == index)).collect()
}

fn add_inspector_checkbox(context: &mut RuntimeContext, parent: UiContainerId, name: &str, enabled: bool) -> UiButtonId {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 20.0)), UiAutoLayout::row(0.0, 8.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 18.0)), name, 11.0, Color::WHITE);
    let button = context.ui.add_toggle_button_layout(Some(row), UiLayout::new(Vec2::new(16.0, 16.0)), enabled);
    set_checkbox_mark(context, button);
    button
}

fn add_inspector_slider_row(context: &mut RuntimeContext, parent: UiContainerId, name: &str, minimum: f32, maximum: f32, value: f32) -> (UiSliderId, UiLabelId) {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 34.0)), UiAutoLayout::row(0.0, 5.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(94.0, 20.0)), name, 11.0, Color::WHITE);
    let slider = context.ui.add_slider_layout(Some(row), UiLayout::new(Vec2::new(180.0, 18.0)), minimum, maximum, value);
    let value_label = context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(50.0, 20.0)), "", 11.0, Color::WHITE);
    (slider, value_label)
}

fn add_inspector_mode_button(context: &mut RuntimeContext, parent: UiContainerId, name: &str, selected: bool) -> UiButtonId {
    let button = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(58.0, 24.0)), selected);
    context.ui.set_button_text(button, name, 10.0, Color::WHITE);
    button
}

fn select_new_button(context: &RuntimeContext, buttons: &[UiButtonId], current: usize) -> Option<usize> {
    buttons.iter().enumerate().find_map(|(index, button)| (index != current && button_selected(context, button)).then_some(index))
}

fn light_mode_index(count: usize) -> usize {
    LIGHT_COUNTS.iter().enumerate().min_by_key(|(_, candidate)| candidate.abs_diff(count)).map(|(index, _)| index).unwrap_or(0)
}

fn button_selected(context: &RuntimeContext, button: &UiButtonId) -> bool {
    context.ui.button_toggled(*button).unwrap_or(false)
}

fn sync_group(context: &mut RuntimeContext, buttons: &[UiButtonId], selected: usize) {
    for (index, button) in buttons.iter().enumerate() {
        context.ui.set_button_toggled(*button, index == selected);
    }
}

fn read_toggle(context: &RuntimeContext, button: Option<UiButtonId>, fallback: bool) -> bool {
    button.and_then(|button| context.ui.button_toggled(button)).unwrap_or(fallback)
}

fn set_checkbox_mark(context: &mut RuntimeContext, button: UiButtonId) {
    let text = if context.ui.button_toggled(button).unwrap_or(false) { "✓" } else { "" };
    context.ui.set_button_text(button, text, 11.0, Color::WHITE);
}

fn moving_set(count: usize, dynamic_count: usize) -> Vec<bool> {
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|a, b| {
        let key = |index: usize| (index as f64 * 0.618_033_988_749_895).fract();
        key(*a).total_cmp(&key(*b))
    });
    let mut moving = vec![false; count];
    for index in order.into_iter().take(dynamic_count) {
        moving[index] = true;
    }
    moving
}

fn lamp_range(intensity: f32) -> f32 {
    (intensity * 72.0 * 0.8 / std::f32::consts::PI / (0.5 / 255.0)).sqrt()
}

fn parse_scale(text: &str) -> Result<(u32, u32), String> {
    match text {
        "small" => Ok((1, 1)),
        "big" => Ok((4, 4)),
        _ => {
            let result = if let Some((cols, rows)) = text.split_once('x') {
                (cols.parse::<u32>().ok(), rows.parse::<u32>().ok())
            } else {
                let size = text.parse::<u32>().ok();
                (size, size)
            };
            match result {
                (Some(cols), Some(rows)) if cols > 0 && rows > 0 && u64::from(cols) * u64::from(rows) <= 64 => Ok((cols, rows)),
                _ => Err(format!("invalid scale {text}; use small, big, N, or NxM (maximum 64 sections)")),
            }
        }
    }
}

fn main() -> Result<(), String> {
    let mut cols = 1;
    let mut rows = 1;
    let mut light_count = 5;
    let mut dynamic_percent = 50;
    let mut sun_speed = 1.0;
    let mut seed = 1;
    let mut surface_probe_gi_enabled = true;
    let mut boxes_still = false;
    let mut debug_script = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--scale" => (cols, rows) = parse_scale(&args.next().ok_or("--scale needs a value")?)?,
            "--lights" => light_count = args.next().ok_or("--lights needs a value")?.parse().map_err(|_| "invalid --lights value")?,
            "--dynamic" => dynamic_percent = args.next().ok_or("--dynamic needs a value")?.parse().map_err(|_| "invalid --dynamic value")?,
            "--sun-speed" => sun_speed = args.next().ok_or("--sun-speed needs a value")?.parse().map_err(|_| "invalid --sun-speed value")?,
            "--seed" => seed = args.next().ok_or("--seed needs a value")?.parse().map_err(|_| "invalid --seed value")?,
            "--global-grid-only" => surface_probe_gi_enabled = false,
            "--boxes-still" => boxes_still = true,
            argument if argument.starts_with("--capture-script=") => debug_script = Some(argument.trim_start_matches("--capture-script=").into()),
            "--help" | "-h" => {
                println!(
                    "genos [--scale small|big|N|NxM] [--lights 1..7] [--dynamic 0..100] [--sun-speed 0|1|4|16] [--seed N] [--boxes-still] [--global-grid-only] [--capture-script=PATH]\nClick to capture mouse; WASD + mouse to fly, Space/C to rise or descend, Shift to move faster. Escape releases the mouse."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    if !(1..=MAX_POINT_LIGHTS).contains(&light_count) {
        return Err(format!("--lights must be between 1 and {MAX_POINT_LIGHTS}; the renderer supports eight active lights including the sun"));
    }
    if dynamic_percent > 100 {
        return Err("--dynamic must be from 0 to 100".into());
    }
    if !SUN_SPEEDS.contains(&sun_speed) {
        return Err("--sun-speed must be one of 0, 1, 4, or 16".into());
    }

    let mut settings = WindowSettings::default();
    settings.title = "Genos lighting stress scene".into();
    settings.debug_script = debug_script;
    run(
        settings,
        GenosDemo {
            cols,
            rows,
            light_count,
            dynamic_percent,
            layout: LampLayout::Spread,
            sun_speed,
            sun_frozen: false,
            sky_on: true,
            boxes_still,
            seed,
            time: 0.0,
            box_time: 0.0,
            day: 0.5,
            surface_probe_gi_enabled,
            yaw: 0.0,
            pitch: 0.0,
            camera: None,
            sun: None,
            lamps: Vec::new(),
            moving_boxes: Vec::new(),
            buttons: InspectorButtons::default(),
            render_mode: RenderDebugMode::LitMaterials,
        },
    )
}
