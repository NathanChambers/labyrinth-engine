use labyrinth::{
    Color, KeyCode, RendererGiRayDiagnostic, RendererGiTraceDebugMode, RendererRadianceFieldGrid, RuntimeContext, UiAnchor, UiAutoLayout, UiButtonId, UiContainerId, UiLabelId, UiLayout,
    UiPanelWindowId, Vec2, Vec3,
};

const INSPECTOR_WIDTH: f32 = 440.0;
const INSPECTOR_TOP: f32 = 220.0;
const RAY_COUNT: usize = labyrinth::RADIANCE_FIELD_RAY_COUNT;
const CHANNEL_COUNT: usize = labyrinth::RADIANCE_FIELD_CHANNEL_COUNT;
const RAYS_PER_CHANNEL: usize = labyrinth::RADIANCE_FIELD_RAYS_PER_CHANNEL;
const MOVE_REPEAT_SECONDS: f32 = 0.12;

#[derive(Clone, Copy, Default)]
struct Drawer {
    header: Option<UiButtonId>,
    content: Option<UiContainerId>,
    content_height: f32,
}

pub struct RadianceInspector {
    open_button: UiButtonId,
    content: UiContainerId,
    window: UiPanelWindowId,
    freeze_button: UiButtonId,
    refine_button: UiButtonId,
    gpu_timing_button: UiButtonId,
    gpu_timing_label: UiLabelId,
    pick_button: UiButtonId,
    move_button: UiButtonId,
    status_label: UiLabelId,
    cell_label: UiLabelId,
    grid_metrics_label: UiLabelId,
    cell_world_position_label: UiLabelId,
    cell_drawer: Drawer,
    rays_drawer: Drawer,
    ray_buttons: [UiButtonId; RAY_COUNT],
    geometry_button: UiButtonId,
    radiance_button: UiButtonId,
    overview_button: UiButtonId,
    show_other_gizmos_button: UiButtonId,
    cell_swatches: [UiButtonId; CHANNEL_COUNT],
    cell_rgb_labels: [UiLabelId; CHANNEL_COUNT],
    cell_sample_labels: [UiLabelId; CHANNEL_COUNT],
    cell_refresh_label: UiLabelId,
    ray_weights_label: UiLabelId,
    direct_swatch: UiButtonId,
    emission_swatch: UiButtonId,
    sky_swatch: UiButtonId,
    total_swatch: UiButtonId,
    total_label: UiLabelId,
    direct_label: UiLabelId,
    emission_label: UiLabelId,
    sky_label: UiLabelId,
    status_trace_label: UiLabelId,
    hit_position_label: UiLabelId,
    hit_normal_label: UiLabelId,
    sampling_label: UiLabelId,
    cell_center_label: UiLabelId,
    ray_origin_label: UiLabelId,
    relocation_offset_label: UiLabelId,
    pick_armed: bool,
    movement_keys_active: bool,
    previous_move_mask: u8,
    move_repeat_elapsed: f32,
}

impl RadianceInspector {
    pub fn new(context: &mut RuntimeContext, gizmos_parent: UiContainerId) -> Self {
        let open_button = context.ui.add_toggle_button_layout(Some(gizmos_parent), UiLayout::new(Vec2::new(0.0, 22.0)), false);
        context.ui.set_button_text(open_button, "Selected GI cell rays", 11.0, Color::WHITE);
        context.ui.set_button_text_centered(open_button, false);
        context.ui.set_button_toggled_colors(open_button, Color::rgb(0.18, 0.32, 0.44), Color::rgb(0.15, 0.2, 0.28));

        let panel_layout = UiLayout::anchored(Vec2::new(INSPECTOR_WIDTH, 500.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-24.0, INSPECTOR_TOP));
        let panel = context.ui.add_panel(panel_layout, Color::rgba(0.03, 0.04, 0.06, 0.96));
        let header_layout = UiLayout::anchored(Vec2::new(INSPECTOR_WIDTH, 26.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-24.0, INSPECTOR_TOP));
        let header = context.ui.add_toggle_button_layout(None, header_layout, false);
        context.ui.set_button_text(header, "GI Radiance Inspector", 15.0, Color::WHITE);
        context.ui.set_button_text_centered(header, false);
        context.ui.set_button_toggled_colors(header, Color::rgba(0.1, 0.12, 0.17, 1.0), Color::rgba(0.1, 0.12, 0.17, 1.0));

        let content = context.ui.add_container(
            None,
            UiLayout::anchored(Vec2::new(INSPECTOR_WIDTH, 740.0), UiAnchor::TOP_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-24.0, INSPECTOR_TOP + 26.0)),
            UiAutoLayout::column(4.0, 0.5),
        );
        let window = context.ui.register_panel_window(panel, header, content, 26.0);
        add_section(context, content, "RADIANCE FIELD");
        let field_row = context.ui.add_container(Some(content), UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 6.0));
        let freeze_button = add_checkbox(context, field_row, "Freeze cache", context.freeze_gi_radiance);
        let refine_button = add_checkbox(context, field_row, "Refine samples", context.gi_radiance_temporal_accumulation_enabled);
        let gpu_timing_row = context.ui.add_container(Some(content), UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 6.0));
        let gpu_timing_button = context.ui.add_toggle_button_layout(Some(gpu_timing_row), UiLayout::new(Vec2::new(136.0, 22.0)), false);
        context.ui.set_button_text(gpu_timing_button, "Sample GPU timing", 10.0, Color::WHITE);
        context.ui.set_button_toggled_colors(gpu_timing_button, Color::rgb(0.18, 0.32, 0.44), Color::rgb(0.15, 0.2, 0.28));
        let gpu_timing_label = context.ui.add_label_layout(Some(gpu_timing_row), UiLayout::new(Vec2::new(210.0, 18.0)), "Scene pass GPU: -- ms", 10.0, Color::rgb(0.72, 0.76, 0.82));
        let tool_row = context.ui.add_container(Some(content), UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 6.0));
        let pick_button = context.ui.add_toggle_button_layout(Some(tool_row), UiLayout::new(Vec2::new(96.0, 22.0)), false);
        context.ui.set_button_text(pick_button, "Pick surface", 11.0, Color::WHITE);
        context.ui.set_button_toggled_colors(pick_button, Color::rgb(0.5, 0.28, 0.12), Color::rgb(0.15, 0.2, 0.28));
        let move_button = context.ui.add_toggle_button_layout(Some(tool_row), UiLayout::new(Vec2::new(96.0, 22.0)), false);
        context.ui.set_button_text(move_button, "Move cell", 11.0, Color::WHITE);
        context.ui.set_button_toggled_colors(move_button, Color::rgb(0.18, 0.42, 0.3), Color::rgb(0.15, 0.2, 0.28));
        let status_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 16.0)), "Pick a surface or move the selected cell", 10.0, Color::rgb(0.72, 0.76, 0.82));
        let cell_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 15.0)), "Radiance field · cell (4, 1, 4) / (16, 4, 16)", 11.0, Color::rgb(0.72, 0.76, 0.82));
        let grid_metrics_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 15.0)), "Field size: -- · Cell size: --", 10.0, Color::rgb(0.72, 0.76, 0.82));
        let cell_world_position_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 15.0)), "Cell center: --", 10.0, Color::rgb(0.72, 0.76, 0.82));

        let cell_drawer = add_drawer(context, content, "RADIANCE CELL", true, 155.0);
        let cell_content = cell_drawer.content.expect("radiance cell content exists");
        let mut cell_swatches = [None; CHANNEL_COUNT];
        let mut cell_rgb_labels = [None; CHANNEL_COUNT];
        let mut cell_sample_labels = [None; CHANNEL_COUNT];
        for channel_index in 0..CHANNEL_COUNT {
            let row = context.ui.add_container(Some(cell_content), UiLayout::new(Vec2::new(0.0, 20.0)), UiAutoLayout::row(0.0, 5.0));
            context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(28.0, 18.0)), format!("L{channel_index}"), 10.0, Color::rgb(0.72, 0.76, 0.82));
            cell_swatches[channel_index] = Some(add_color_swatch(context, row, [0.0; 3], 16.0));
            cell_rgb_labels[channel_index] = Some(context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(118.0, 18.0)), "RGB --", 10.0, Color::WHITE));
            cell_sample_labels[channel_index] = Some(context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(145.0, 18.0)), "0 samples", 10.0, Color::rgb(0.72, 0.76, 0.82)));
        }
        let cell_refresh_label = context.ui.add_label_layout(Some(cell_content), UiLayout::new(Vec2::new(0.0, 16.0)), "Last refresh: --", 10.0, Color::rgb(0.72, 0.76, 0.82));

        let rays_drawer = add_drawer(context, content, "RAY INSPECTOR", false, 475.0);
        let ray_content = rays_drawer.content.expect("ray inspector content exists");
        let mut ray_buttons = [None; RAY_COUNT];
        let channel_names = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
        for (channel_index, channel_name) in channel_names.into_iter().enumerate() {
            let row = context.ui.add_container(Some(ray_content), UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 3.0));
            context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(30.0, 18.0)), channel_name, 10.0, Color::rgb(0.72, 0.76, 0.82));
            for sample_index in 0..RAYS_PER_CHANNEL {
                let ray_index = channel_index * RAYS_PER_CHANNEL + sample_index;
                let button = context.ui.add_toggle_button_layout(Some(row), UiLayout::new(Vec2::new(44.0, 22.0)), ray_index as u32 == context.gi_trace_selected_ray);
                context.ui.set_button_text(button, format!("R{ray_index}"), 10.0, Color::WHITE);
                context.ui.set_button_toggled_colors(button, Color::rgb(0.2, 0.48, 0.62), Color::rgb(0.15, 0.2, 0.28));
                ray_buttons[ray_index] = Some(button);
            }
        }
        let mode_row = context.ui.add_container(Some(ray_content), UiLayout::new(Vec2::new(0.0, 22.0)), UiAutoLayout::row(0.0, 6.0));
        let geometry_button = add_mode_button(context, mode_row, "Geometry", context.gi_trace_debug_mode == RendererGiTraceDebugMode::Geometry);
        let radiance_button = add_mode_button(context, mode_row, "Radiance", context.gi_trace_debug_mode == RendererGiTraceDebugMode::Radiance);
        let overview_button = add_checkbox(context, ray_content, "All-ray overview", context.gi_trace_overview_enabled);
        let show_other_gizmos_button = add_checkbox(context, ray_content, "Show other gizmos", false);
        let status_trace_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Waiting for trace data", 11.0, Color::WHITE);
        let sampling_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Direction: --", 10.0, Color::rgb(0.72, 0.76, 0.82));
        let cell_center_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Cell center: --", 10.0, Color::rgb(0.15, 0.85, 1.0));
        let ray_origin_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Ray origin: --", 10.0, Color::WHITE);
        let relocation_offset_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Relocation offset: --", 10.0, Color::rgb(1.0, 0.72, 0.12));
        let hit_position_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Hit position: --", 10.0, Color::rgb(0.72, 0.76, 0.82));
        let hit_normal_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 15.0)), "Normal: --", 10.0, Color::rgb(0.15, 0.5, 1.0));
        let total_row = context.ui.add_container(Some(ray_content), UiLayout::new(Vec2::new(0.0, 20.0)), UiAutoLayout::row(0.0, 5.0));
        context.ui.add_label_layout(Some(total_row), UiLayout::new(Vec2::new(38.0, 18.0)), "Total", 10.0, Color::rgb(0.72, 0.76, 0.82));
        let total_swatch = add_color_swatch(context, total_row, [0.0; 3], 16.0);
        let total_label = context.ui.add_label_layout(Some(total_row), UiLayout::new(Vec2::new(120.0, 18.0)), "RGB --", 10.0, Color::WHITE);
        let (direct_swatch, direct_label) = add_color_value_row(context, ray_content, "Direct");
        let (emission_swatch, emission_label) = add_color_value_row(context, ray_content, "Emission");
        let (sky_swatch, sky_label) = add_color_value_row(context, ray_content, "Sky");
        let ray_weights_label = context.ui.add_label_layout(Some(ray_content), UiLayout::new(Vec2::new(0.0, 30.0)), "Directional weights: --", 10.0, Color::rgb(0.72, 0.76, 0.82));

        Self {
            open_button,
            content,
            window,
            freeze_button,
            refine_button,
            gpu_timing_button,
            gpu_timing_label,
            pick_button,
            move_button,
            status_label,
            cell_label,
            grid_metrics_label,
            cell_world_position_label,
            cell_drawer,
            rays_drawer,
            ray_buttons: ray_buttons.map(Option::unwrap),
            geometry_button,
            radiance_button,
            overview_button,
            show_other_gizmos_button,
            cell_swatches: cell_swatches.map(Option::unwrap),
            cell_rgb_labels: cell_rgb_labels.map(Option::unwrap),
            cell_sample_labels: cell_sample_labels.map(Option::unwrap),
            cell_refresh_label,
            ray_weights_label,
            direct_swatch,
            emission_swatch,
            sky_swatch,
            total_swatch,
            total_label,
            direct_label,
            emission_label,
            sky_label,
            status_trace_label,
            hit_position_label,
            hit_normal_label,
            sampling_label,
            cell_center_label,
            ray_origin_label,
            relocation_offset_label,
            pick_armed: false,
            movement_keys_active: false,
            previous_move_mask: 0,
            move_repeat_elapsed: 0.0,
        }
    }

    pub fn update(&mut self, context: &mut RuntimeContext) -> bool {
        let inspector_open = context.ui.button_toggled(self.open_button).unwrap_or(false);
        context.ui.set_panel_window_visible(self.window, inspector_open);
        let collapsed = context.ui.panel_window_collapsed(self.window).unwrap_or(false);
        context.ui.set_container_visible(self.content, inspector_open && !collapsed);
        context.gi_trace_gizmos_enabled = inspector_open;
        context.gizmos_enabled |= inspector_open;
        context.ui.set_panel_window_expanded_height(self.window, self.panel_height(context));
        update_drawer(context, self.cell_drawer, "RADIANCE CELL");
        update_drawer(context, self.rays_drawer, "RAY INSPECTOR");

        self.update_cell_selection(context);
        let was_pick_armed = self.pick_armed;
        let was_move_enabled = self.movement_keys_active;
        self.pick_armed = inspector_open && context.ui.button_toggled(self.pick_button).unwrap_or(false);
        let mut move_enabled = inspector_open && context.ui.button_toggled(self.move_button).unwrap_or(false);
        if self.pick_armed && !was_pick_armed {
            context.ui.set_button_toggled(self.move_button, false);
            move_enabled = false;
        } else if move_enabled && !was_move_enabled {
            context.ui.set_button_toggled(self.pick_button, false);
            self.pick_armed = false;
        } else if self.pick_armed {
            context.ui.set_button_toggled(self.move_button, false);
            move_enabled = false;
        } else if move_enabled {
            context.ui.set_button_toggled(self.pick_button, false);
        }
        if self.pick_armed && !was_pick_armed {
            context.ui.set_label_text(self.status_label, "Click a mesh surface in the scene");
        } else if move_enabled && !was_move_enabled {
            context.ui.set_label_text(self.status_label, "W/S: Z · A/D: X · R/F: Y");
        } else if !self.pick_armed && was_pick_armed {
            context.ui.set_label_text(self.status_label, "Pick a surface or move the selected cell");
        } else if !move_enabled && was_move_enabled {
            context.ui.set_label_text(self.status_label, "Pick a surface or move the selected cell");
        }
        context.cursor_capture_on_scene_click = !self.pick_armed;
        self.update_pick(context);
        self.update_cell_movement(context, move_enabled && inspector_open);
        context.freeze_gi_radiance = context.ui.button_toggled(self.freeze_button).unwrap_or(context.freeze_gi_radiance);
        set_checkbox_mark(context, self.freeze_button);
        context.gi_radiance_temporal_accumulation_enabled = context.ui.button_toggled(self.refine_button).unwrap_or(context.gi_radiance_temporal_accumulation_enabled);
        set_checkbox_mark(context, self.refine_button);
        if context.ui.button_toggled(self.gpu_timing_button).unwrap_or(false) {
            context.request_gpu_timing_sample = true;
            context.ui.set_button_toggled(self.gpu_timing_button, false);
        }
        context.ui.set_label_text(self.gpu_timing_label, format!("Scene pass GPU: {:.2} ms", context.performance_stats.gpu_scene_ms));
        context.gi_trace_overview_enabled = context.ui.button_toggled(self.overview_button).unwrap_or(false);
        set_checkbox_mark(context, self.overview_button);

        self.update_ray_selection(context);
        self.update_trace_display(context);
        self.movement_keys_active = move_enabled && inspector_open;
        let show_other_gizmos = context.ui.button_toggled(self.show_other_gizmos_button).unwrap_or(false);
        set_checkbox_mark(context, self.show_other_gizmos_button);
        let rays_open = self.rays_drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        let radiance_focus = inspector_open && rays_open && context.gi_trace_debug_mode == RendererGiTraceDebugMode::Radiance && !show_other_gizmos;
        context.gi_global_gizmos_enabled &= !radiance_focus;
        context.gi_gizmos_enabled &= !radiance_focus;
        context.normal_gizmos_enabled &= !radiance_focus;
        context.light_gizmos_enabled &= !radiance_focus;
        context.sdf_occupancy_gizmos_enabled &= !radiance_focus;
        self.movement_keys_active
    }

    fn panel_height(&self, context: &RuntimeContext) -> f32 {
        let cell_open = self.cell_drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(true);
        let rays_open = self.rays_drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false);
        272.0 + if cell_open { self.cell_drawer.content_height } else { 0.0 } + if rays_open { self.rays_drawer.content_height } else { 0.0 }
    }

    fn update_cell_selection(&mut self, context: &mut RuntimeContext) {
        let grid = selected_grid(context);
        for axis in 0..3 {
            context.gi_selected_radiance_cell[axis] = context.gi_selected_radiance_cell[axis].min(grid.resolution[axis].saturating_sub(1));
        }
        let field_extent = Vec3::from_array(grid.maximum) - Vec3::from_array(grid.minimum);
        let resolution = Vec3::new(grid.resolution[0] as f32, grid.resolution[1] as f32, grid.resolution[2] as f32);
        let cell_size = field_extent / resolution;
        let cell_coordinate = Vec3::new(context.gi_selected_radiance_cell[0] as f32, context.gi_selected_radiance_cell[1] as f32, context.gi_selected_radiance_cell[2] as f32);
        let cell_center = Vec3::from_array(grid.minimum) + (cell_coordinate + Vec3::splat(0.5)) * cell_size;
        context.ui.set_label_text(
            self.cell_label,
            format!(
                "Radiance field · cell ({}, {}, {}) / ({}, {}, {})",
                context.gi_selected_radiance_cell[0], context.gi_selected_radiance_cell[1], context.gi_selected_radiance_cell[2], grid.resolution[0], grid.resolution[1], grid.resolution[2]
            ),
        );
        if field_extent.is_finite() && field_extent.min_element() > 0.0 {
            context.ui.set_label_text(self.grid_metrics_label, format!("Field size: {} m · Cell size: {} m", format_vec3(field_extent.to_array()), format_vec3(cell_size.to_array())));
            context.ui.set_label_text(self.cell_world_position_label, format!("Cell center: {}", format_vec3(cell_center.to_array())));
        } else {
            context.ui.set_label_text(self.grid_metrics_label, "Field size: waiting for renderer bounds");
            context.ui.set_label_text(self.cell_world_position_label, "Cell center: waiting for renderer bounds");
        }
    }

    fn update_pick(&mut self, context: &mut RuntimeContext) {
        if !self.pick_armed || !context.input.left_mouse_pressed() || context.ui.pointer_press_consumed() {
            return;
        }
        match raycast_scene(context) {
            Some(position) => {
                if let Some((cell, _)) = nearest_cell_in_grid(position, selected_grid(context)) {
                    context.gi_selected_radiance_cell = cell;
                    context.ui.set_label_text(self.status_label, format!("Picked surface at {}", format_vec3(position.to_array())));
                } else {
                    context.ui.set_label_text(self.status_label, "Radiance field bounds are not ready");
                }
            }
            None => {
                context.ui.set_label_text(self.status_label, "No mesh surface hit");
            }
        }
        self.pick_armed = false;
        context.ui.set_button_toggled(self.pick_button, false);
        context.cursor_capture_on_scene_click = true;
    }

    fn update_cell_movement(&mut self, context: &mut RuntimeContext, enabled: bool) {
        if !enabled {
            self.previous_move_mask = 0;
            self.move_repeat_elapsed = 0.0;
            return;
        }
        let input = &context.input;
        let mask = u8::from(input.is_key_down(KeyCode::KeyW))
            | (u8::from(input.is_key_down(KeyCode::KeyS)) << 1)
            | (u8::from(input.is_key_down(KeyCode::KeyA)) << 2)
            | (u8::from(input.is_key_down(KeyCode::KeyD)) << 3)
            | (u8::from(input.is_key_down(KeyCode::KeyR)) << 4)
            | (u8::from(input.is_key_down(KeyCode::KeyF)) << 5);
        if mask == 0 {
            self.previous_move_mask = 0;
            self.move_repeat_elapsed = 0.0;
            return;
        }
        let first_press = mask & !self.previous_move_mask != 0;
        self.move_repeat_elapsed += context.time.delta_seconds();
        if first_press || self.move_repeat_elapsed >= MOVE_REPEAT_SECONDS {
            let delta = Vec3::new(
                (i32::from(input.is_key_down(KeyCode::KeyD)) - i32::from(input.is_key_down(KeyCode::KeyA))) as f32,
                (i32::from(input.is_key_down(KeyCode::KeyR)) - i32::from(input.is_key_down(KeyCode::KeyF))) as f32,
                (i32::from(input.is_key_down(KeyCode::KeyS)) - i32::from(input.is_key_down(KeyCode::KeyW))) as f32,
            );
            let grid = selected_grid(context);
            for axis in 0..3 {
                let next = context.gi_selected_radiance_cell[axis] as i32 + delta[axis] as i32;
                context.gi_selected_radiance_cell[axis] = next.clamp(0, grid.resolution[axis].saturating_sub(1) as i32) as u32;
            }
            self.move_repeat_elapsed = 0.0;
            context.ui.set_label_text(self.status_label, "W/S: Z · A/D: X · R/F: Y");
        }
        self.previous_move_mask = mask;
    }

    fn update_ray_selection(&mut self, context: &mut RuntimeContext) {
        context.gi_trace_selected_ray = context.gi_trace_selected_ray.min(RAY_COUNT as u32 - 1);
        let current_ray = context.gi_trace_selected_ray as usize;
        if let Some((index, _)) = self.ray_buttons.iter().enumerate().find(|(index, button)| *index != current_ray && context.ui.button_toggled(**button).unwrap_or(false)) {
            context.gi_trace_selected_ray = index as u32;
        }
        for (index, button) in self.ray_buttons.iter().enumerate() {
            context.ui.set_button_toggled(*button, index as u32 == context.gi_trace_selected_ray);
        }
        let mode = context.gi_trace_debug_mode;
        let geometry_selected = context.ui.button_toggled(self.geometry_button).unwrap_or(mode == RendererGiTraceDebugMode::Geometry);
        let radiance_selected = context.ui.button_toggled(self.radiance_button).unwrap_or(mode == RendererGiTraceDebugMode::Radiance);
        context.gi_trace_debug_mode = match mode {
            RendererGiTraceDebugMode::Geometry if radiance_selected => RendererGiTraceDebugMode::Radiance,
            RendererGiTraceDebugMode::Radiance if geometry_selected => RendererGiTraceDebugMode::Geometry,
            _ => mode,
        };
        context.ui.set_button_toggled(self.geometry_button, context.gi_trace_debug_mode == RendererGiTraceDebugMode::Geometry);
        context.ui.set_button_toggled(self.radiance_button, context.gi_trace_debug_mode == RendererGiTraceDebugMode::Radiance);
    }

    fn update_trace_display(&mut self, context: &mut RuntimeContext) {
        let cell = context.gi_selected_radiance_cell;
        let selected_trace = context.gi_trace_diagnostics[context.gi_trace_selected_ray as usize];
        let trace_matches_cell = selected_trace.selected_cell == cell;
        let trace_is_current = selected_trace.valid && trace_matches_cell;
        let relocation_is_current = selected_trace.relocation_data_valid && trace_matches_cell;
        let displayed_trace = if trace_is_current { selected_trace } else { RendererGiRayDiagnostic::default() };
        let cell_is_current = context.gi_trace_diagnostics.iter().any(|diagnostic| diagnostic.valid && diagnostic.selected_cell == cell);
        for channel_index in 0..CHANNEL_COUNT {
            let diagnostic = context.gi_trace_diagnostics[channel_index * RAYS_PER_CHANNEL];
            let channel_is_current = diagnostic.valid && diagnostic.selected_cell == cell;
            let channel_value = if channel_is_current { diagnostic.stored_radiance } else { [0.0; 3] };
            update_swatch(context, self.cell_swatches[channel_index], channel_value);
            context.ui.set_label_text(self.cell_rgb_labels[channel_index], if channel_is_current { format!("RGB {}", format_rgb(channel_value)) } else { "RGB --".to_owned() });
            let samples = if channel_is_current {
                format!("{} valid / {} tried · blend {:.2}", diagnostic.accumulated_sample_count, diagnostic.attempted_sample_count, diagnostic.blend_factor)
            } else {
                "-- valid / -- tried".to_owned()
            };
            context.ui.set_label_text(self.cell_sample_labels[channel_index], samples);
        }
        let refresh_text = if cell_is_current {
            let age =
                context.gi_trace_diagnostics.iter().filter(|diagnostic| diagnostic.valid && diagnostic.selected_cell == cell).map(|diagnostic| diagnostic.last_update_age_frames).min().unwrap_or(0);
            format!("Last refresh: {age} frames ago")
        } else {
            "Last refresh: waiting for trace data".to_owned()
        };
        context.ui.set_label_text(self.cell_refresh_label, refresh_text);
        context.ui.set_label_text(
            self.status_trace_label,
            if trace_is_current {
                format!("{} · {:.2} m", termination_name(displayed_trace.termination_reason), displayed_trace.travelled_distance)
            } else if relocation_is_current && !selected_trace.relocation_succeeded {
                "RELOCATION_FAILED · no clear origin within 1.0 m".to_owned()
            } else {
                "Waiting for trace data".to_owned()
            },
        );
        let relocation_data = if relocation_is_current { selected_trace } else { RendererGiRayDiagnostic::default() };
        let cell_center_text = if relocation_is_current { format_vec3(relocation_data.original_cell_center) } else { "--".to_owned() };
        let ray_origin_text = if relocation_is_current { format_vec3(relocation_data.ray_origin) } else { "--".to_owned() };
        let relocation_offset_text = if relocation_is_current {
            let moved = relocation_data.relocation_offset.iter().any(|value| value.abs() > 0.001);
            format!("{} m · {}", format_vec3(relocation_data.relocation_offset), if moved { "relocated" } else { "unchanged" })
        } else {
            "--".to_owned()
        };
        context.ui.set_label_text(self.cell_center_label, format!("Cell center: {cell_center_text}"));
        context.ui.set_label_text(self.ray_origin_label, format!("Ray origin: {ray_origin_text}"));
        context.ui.set_label_text(self.relocation_offset_label, format!("Relocation offset: {relocation_offset_text}"));
        let hit = trace_is_current && displayed_trace.termination_reason == 1;
        context.ui.set_label_text(self.hit_position_label, format!("Hit position: {}", if hit { format_vec3(displayed_trace.hit_position) } else { "--".to_owned() }));
        context.ui.set_label_text(self.hit_normal_label, format!("Normal: {}", if hit { format_vec3(displayed_trace.hit_normal) } else { "--".to_owned() }));
        update_swatch(context, self.total_swatch, displayed_trace.radiance);
        context.ui.set_label_text(self.total_label, if trace_is_current { format!("RGB {}", format_rgb(displayed_trace.radiance)) } else { "RGB --".to_owned() });
        for (swatch, label, value) in [
            (self.direct_swatch, self.direct_label, displayed_trace.direct_radiance),
            (self.emission_swatch, self.emission_label, displayed_trace.emission_radiance),
            (self.sky_swatch, self.sky_label, displayed_trace.sky_radiance),
        ] {
            update_swatch(context, swatch, value);
            context.ui.set_label_text(label, if trace_is_current { format!("RGB {}", format_rgb(value)) } else { "RGB --".to_owned() });
        }
        let channel_index = context.gi_trace_selected_ray as usize / RAYS_PER_CHANNEL;
        let sample_index = context.gi_trace_selected_ray as usize % RAYS_PER_CHANNEL + 1;
        let channel_names = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
        let sampling = if trace_is_current {
            format!("{} sample {sample_index}/{RAYS_PER_CHANNEL} · Direction: {}", channel_names[channel_index], format_vec3(displayed_trace.ray_direction))
        } else {
            "Direction: --".to_owned()
        };
        context.ui.set_label_text(self.sampling_label, sampling);
        let weights = displayed_trace.directional_weights.map(|weight| format!("{weight:.2}"));
        let weight_text = if trace_is_current {
            format!("L0 {} · L1 {} · L2 {} · L3 {} · L4 {} · L5 {}", weights[0], weights[1], weights[2], weights[3], weights[4], weights[5])
        } else {
            "Directional weights: --".to_owned()
        };
        context.ui.set_label_text(self.ray_weights_label, weight_text);
    }
}

fn selected_grid(context: &RuntimeContext) -> RendererRadianceFieldGrid {
    let mut grid = context.gi_radiance_field_grid;
    if grid.resolution.contains(&0) {
        grid.resolution = context.gi_radiance_field_resolution.resolution;
        grid.resolution = grid.resolution.map(|count| count.max(1));
    }
    grid
}

fn nearest_cell_in_grid(position: Vec3, grid: RendererRadianceFieldGrid) -> Option<([u32; 3], f32)> {
    if grid.resolution.contains(&0) {
        return None;
    }
    let minimum = Vec3::from_array(grid.minimum);
    let maximum = Vec3::from_array(grid.maximum);
    let extent = maximum - minimum;
    if !minimum.is_finite() || !maximum.is_finite() || extent.min_element() <= 0.0 {
        return None;
    }
    let cell_size = extent / Vec3::new(grid.resolution[0] as f32, grid.resolution[1] as f32, grid.resolution[2] as f32);
    let cell = std::array::from_fn(|axis| (((position[axis] - minimum[axis]) / cell_size[axis] - 0.5).round() as i32).clamp(0, grid.resolution[axis] as i32 - 1) as u32);
    let center = minimum + (Vec3::from_array(cell.map(|coordinate| coordinate as f32)) + Vec3::splat(0.5)) * cell_size;
    Some((cell, center.distance_squared(position)))
}

fn raycast_scene(context: &RuntimeContext) -> Option<Vec3> {
    let camera = context.active_camera.and_then(|camera_id| context.scene.get_camera(camera_id))?;
    let (width, height) = context.ui.viewport_size();
    let cursor = context.input.cursor_position();
    let normalized = Vec2::new(cursor.x / width.max(1) as f32 * 2.0 - 1.0, 1.0 - cursor.y / height.max(1) as f32 * 2.0);
    let aspect = width.max(1) as f32 / height.max(1) as f32;
    let tangent = (camera.field_of_view_y.to_radians() * 0.5).tan();
    let local_direction = Vec3::new(normalized.x * aspect * tangent, normalized.y * tangent, -1.0).normalize();
    let ray_origin = camera.transform.position;
    let ray_direction = (camera.transform.rotation * local_direction).normalize();
    let mut nearest_distance = f32::INFINITY;
    for instance in context.scene.instances() {
        for triangle in instance.mesh.indices().chunks_exact(3) {
            let first = instance.transform.transform_point(Vec3::from_array(instance.mesh.vertices()[triangle[0] as usize].position));
            let second = instance.transform.transform_point(Vec3::from_array(instance.mesh.vertices()[triangle[1] as usize].position));
            let third = instance.transform.transform_point(Vec3::from_array(instance.mesh.vertices()[triangle[2] as usize].position));
            if let Some(distance) = ray_triangle_distance(ray_origin, ray_direction, first, second, third)
                && distance < nearest_distance
            {
                nearest_distance = distance;
            }
        }
    }
    nearest_distance.is_finite().then_some(ray_origin + ray_direction * nearest_distance)
}

fn ray_triangle_distance(origin: Vec3, direction: Vec3, first: Vec3, second: Vec3, third: Vec3) -> Option<f32> {
    let edge_one = second - first;
    let edge_two = third - first;
    let cross = direction.cross(edge_two);
    let determinant = edge_one.dot(cross);
    if determinant.abs() < 0.000001 {
        return None;
    }
    let inverse_determinant = determinant.recip();
    let offset = origin - first;
    let barycentric_u = offset.dot(cross) * inverse_determinant;
    if !(0.0..=1.0).contains(&barycentric_u) {
        return None;
    }
    let cross = offset.cross(edge_one);
    let barycentric_v = direction.dot(cross) * inverse_determinant;
    if barycentric_v < 0.0 || barycentric_u + barycentric_v > 1.0 {
        return None;
    }
    let distance = edge_two.dot(cross) * inverse_determinant;
    (distance > 0.0).then_some(distance)
}

fn add_drawer(context: &mut RuntimeContext, parent: UiContainerId, title: &str, expanded: bool, content_height: f32) -> Drawer {
    let header = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 22.0)), expanded);
    let content = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, content_height)), UiAutoLayout::column(0.0, 3.0));
    context.ui.set_container_visible(content, expanded);
    context.ui.set_button_text_centered(header, false);
    context.ui.set_button_toggled_colors(header, Color::rgb(0.15, 0.2, 0.28), Color::rgb(0.1, 0.12, 0.17));
    let drawer = Drawer { header: Some(header), content: Some(content), content_height };
    update_drawer(context, drawer, title);
    drawer
}

fn update_drawer(context: &mut RuntimeContext, drawer: Drawer, title: &str) {
    if let (Some(header), Some(content)) = (drawer.header, drawer.content) {
        let expanded = context.ui.button_toggled(header).unwrap_or(false);
        context.ui.set_container_visible(content, expanded);
        context.ui.set_button_text(header, title, 12.0, Color::WHITE);
    }
}

fn add_section(context: &mut RuntimeContext, parent: UiContainerId, title: &str) {
    context.ui.add_label_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 15.0)), title, 11.0, Color::rgb(0.42, 0.7, 0.96));
}

fn add_mode_button(context: &mut RuntimeContext, parent: UiContainerId, title: &str, selected: bool) -> UiButtonId {
    let button = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(74.0, 22.0)), selected);
    context.ui.set_button_text(button, title, 11.0, Color::WHITE);
    button
}

fn add_checkbox(context: &mut RuntimeContext, parent: UiContainerId, title: &str, enabled: bool) -> UiButtonId {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 18.0)), UiAutoLayout::row(0.0, 8.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 17.0)), title, 12.0, Color::WHITE);
    let button = context.ui.add_toggle_button_layout(Some(row), UiLayout::new(Vec2::new(15.0, 15.0)), enabled);
    set_checkbox_mark(context, button);
    button
}

fn add_color_swatch(context: &mut RuntimeContext, parent: UiContainerId, color: [f32; 3], size: f32) -> UiButtonId {
    let button = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(size, size)), true);
    update_swatch(context, button, color);
    button
}

fn add_color_value_row(context: &mut RuntimeContext, parent: UiContainerId, title: &str) -> (UiButtonId, UiLabelId) {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, 20.0)), UiAutoLayout::row(0.0, 5.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(58.0, 18.0)), title, 10.0, Color::rgb(0.72, 0.76, 0.82));
    let swatch = add_color_swatch(context, row, [0.0; 3], 16.0);
    let label = context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(118.0, 18.0)), "RGB --", 10.0, Color::WHITE);
    (swatch, label)
}

fn update_swatch(context: &mut RuntimeContext, button: UiButtonId, value: [f32; 3]) {
    let channel = |input: f32| (input.max(0.0) / (input.max(0.0) + 0.3)).clamp(0.06, 1.0);
    let color = Color::rgb(channel(value[0]), channel(value[1]), channel(value[2]));
    context.ui.set_button_toggled_colors(button, color, color);
}

fn set_checkbox_mark(context: &mut RuntimeContext, button: UiButtonId) {
    let text = if context.ui.button_toggled(button).unwrap_or(false) { "✓" } else { "" };
    context.ui.set_button_text(button, text, 12.0, Color::WHITE);
}

fn termination_name(reason: u32) -> &'static str {
    match reason {
        1 => "HIT",
        2 => "MAX_STEPS",
        3 => "MAX_DISTANCE",
        4 => "GRID_EXIT",
        5 => "RELOCATION_FAILED",
        _ => "UNRESOLVED",
    }
}

fn format_rgb(value: [f32; 3]) -> String {
    format!("{:.3}, {:.3}, {:.3}", value[0], value[1], value[2])
}

fn format_vec3(value: [f32; 3]) -> String {
    format!("{:.2}, {:.2}, {:.2}", value[0], value[1], value[2])
}
