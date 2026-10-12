use labyrinth::{Color, IrradianceMode, RenderDebugMode, RuntimeContext, UiAnchor, UiAutoLayout, UiButtonId, UiContainerId, UiGraphId, UiLabelId, UiLayout, UiPanelWindowId, Vec2};

mod free_camera;
mod radiance_inspector;
mod surface_inspector;

pub use free_camera::FreeCamera;
pub use radiance_inspector::RadianceInspector;
pub use surface_inspector::SurfaceInspector;

const PANEL_WIDTH: f32 = 320.0;
const HEADER_HEIGHT: f32 = 26.0;
const DRAWER_HEADER_HEIGHT: f32 = 22.0;
const CHECKBOX_ROW_HEIGHT: f32 = 20.0;
const DRAWER_CONTENT_SPACING: f32 = 3.0;
const PERFORMANCE_REFRESH_SECONDS: f32 = 0.25;
const FPS_SAMPLE_SECONDS: f32 = 0.5;
const PERFORMANCE_HEIGHT: f32 = 180.0;

#[derive(Clone, Copy, Default)]
pub struct FpsCounter {
    elapsed_seconds: f32,
    frame_count: u32,
}

impl FpsCounter {
    pub fn update(&mut self, delta_seconds: f32) -> Option<f32> {
        if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            return None;
        }

        self.elapsed_seconds += delta_seconds;
        self.frame_count += 1;
        if self.elapsed_seconds < FPS_SAMPLE_SECONDS {
            return None;
        }

        let fps = self.frame_count as f32 / self.elapsed_seconds;
        self.elapsed_seconds = 0.0;
        self.frame_count = 0;
        Some(fps)
    }
}

pub fn format_fps_counter(fps: f32) -> String {
    format!("{fps:5.0}")
}

pub fn add_fps_counter_overlay(context: &mut RuntimeContext) -> UiLabelId {
    let label = context.ui.add_label_layout(None, UiLayout::absolute(labyrinth::UiRect::new(8.0, 8.0, 56.0, 20.0)), "   --", 16.0, Color::WHITE);
    context.ui.set_label_visible_when_debug_ui_hidden(label, true);
    label
}

#[derive(Clone, Debug)]
pub struct DebugPanelsConfig {
    pub title: String,
    pub anchor: UiAnchor,
    pub offset: Vec2,
    pub use_window_background: bool,
    pub performance_offset: Vec2,
    pub extra_gizmo_content_height: f32,
}

impl DebugPanelsConfig {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            anchor: UiAnchor::TOP_LEFT,
            offset: Vec2::new(24.0, 24.0),
            use_window_background: false,
            performance_offset: Vec2::new(-24.0, 24.0),
            extra_gizmo_content_height: 0.0,
        }
    }

    pub fn with_position(mut self, anchor: UiAnchor, offset: Vec2) -> Self {
        self.anchor = anchor;
        self.offset = offset;
        self
    }

    pub fn with_window_background(mut self) -> Self {
        self.use_window_background = true;
        self
    }

    pub fn with_performance_offset(mut self, offset: Vec2) -> Self {
        self.performance_offset = offset;
        self
    }

    pub fn with_extra_gizmo_content_height(mut self, height: f32) -> Self {
        self.extra_gizmo_content_height = height.max(0.0);
        self
    }
}

#[derive(Clone, Copy, Default)]
struct Drawer {
    header: Option<UiButtonId>,
    content: Option<UiContainerId>,
    content_height: f32,
}

pub struct DebugPanels {
    inspector_window: UiPanelWindowId,
    display_mode_drawer: Drawer,
    render_options_drawer: Drawer,
    gizmos_drawer: Drawer,
    display_mode_buttons: [UiButtonId; 4],
    gi_mode_button_container: UiContainerId,
    gi_support_hint: UiContainerId,
    irradiance_row: UiContainerId,
    irradiance_button: UiButtonId,
    triangle_probe_occlusion_row: UiContainerId,
    triangle_probe_occlusion_button: UiButtonId,
    probe_relocation_row: UiContainerId,
    probe_relocation_button: UiButtonId,
    shadows_row: UiContainerId,
    shadows_button: UiButtonId,
    gizmos_row: UiContainerId,
    normals_row: UiContainerId,
    lights_row: UiContainerId,
    gizmos_button: UiButtonId,
    normals_button: UiButtonId,
    lights_button: UiButtonId,
    gi_gizmos_content: UiContainerId,
    global_probes_row: UiContainerId,
    global_probes_button: UiButtonId,
    volume_gi_hint: UiContainerId,
    sdf_gizmos_content: UiContainerId,
    sdf_occupancy_row: UiContainerId,
    sdf_occupancy_button: UiButtonId,
    performance_content: UiContainerId,
    performance_window: UiPanelWindowId,
    performance_label: UiLabelId,
    performance_frame_label: UiLabelId,
    fps_counter_label: UiLabelId,
    performance_graph: UiGraphId,
    frame_history: Vec<f32>,
    fps_counter: FpsCounter,
    fps: Option<f32>,
    performance_refresh_elapsed: f32,
    radiance_inspector: RadianceInspector,
    surface_inspector: SurfaceInspector,
    movement_keys_active: bool,
    extra_gizmo_content_height: f32,
}

impl DebugPanels {
    pub fn new(context: &mut RuntimeContext, config: DebugPanelsConfig) -> Self {
        let anchor = config.anchor;
        let offset = config.offset;
        let panel_layout = UiLayout::anchored(Vec2::new(PANEL_WIDTH, 400.0), anchor, anchor, offset);
        let inspector_panel = (!config.use_window_background).then(|| context.ui.add_panel(panel_layout, Color::rgba(0.03, 0.04, 0.06, 0.94)));
        let inspector_header = context.ui.add_toggle_button_layout(None, UiLayout::anchored(Vec2::new(PANEL_WIDTH, HEADER_HEIGHT), anchor, anchor, offset), false);
        context.ui.set_button_text(inspector_header, config.title.clone(), 15.0, Color::WHITE);
        context.ui.set_button_text_centered(inspector_header, false);
        context.ui.set_button_toggled_colors(inspector_header, Color::rgba(0.1, 0.12, 0.17, 1.0), Color::rgba(0.1, 0.12, 0.17, 1.0));

        let stack_offset = offset + Vec2::new(0.0, HEADER_HEIGHT + 6.0);
        let inspector_content = context.ui.add_container(None, UiLayout::anchored(Vec2::new(PANEL_WIDTH, 560.0), anchor, anchor, stack_offset), UiAutoLayout::column(10.0, 3.0));
        let inspector_window = if let Some(panel) = inspector_panel {
            context.ui.register_panel_window(panel, inspector_header, inspector_content, HEADER_HEIGHT)
        } else {
            context.ui.register_window_panel(inspector_header, inspector_content, HEADER_HEIGHT)
        };

        let display_mode_drawer = add_drawer(context, inspector_content, "DISPLAY MODE", 24.0);
        let mode_row = context.ui.add_container(display_mode_drawer.content, UiLayout::new(Vec2::new(0.0, 24.0)), UiAutoLayout::row(0.0, 6.0));
        let lit_button = add_mode_button(context, mode_row, "Lit", context.render_debug_mode == RenderDebugMode::LitMaterials);
        let unlit_button = add_mode_button(context, mode_row, "Unlit", context.render_debug_mode == RenderDebugMode::UnlitMaterials);
        let wire_button = add_mode_button(context, mode_row, "Wire", context.render_debug_mode == RenderDebugMode::Wireframe);
        let gi_mode_button_container = context.ui.add_container(Some(mode_row), UiLayout::new(Vec2::new(68.0, 22.0)), UiAutoLayout::row(0.0, 0.0));
        let gi_support_button = add_mode_button(context, gi_mode_button_container, "SDF Support", context.render_debug_mode == RenderDebugMode::GiSupport);
        let display_mode_buttons = [lit_button, unlit_button, wire_button, gi_support_button];
        let gi_support_hint = context.ui.add_container(display_mode_drawer.content, UiLayout::new(Vec2::new(0.0, 32.0)), UiAutoLayout::column(0.0, 0.0));
        context.ui.add_label_layout(Some(gi_support_hint), UiLayout::new(Vec2::new(0.0, 16.0)), "SDF Support: R valid cells · G spatial", 9.0, Color::rgb(0.72, 0.76, 0.82));
        context.ui.add_label_layout(Some(gi_support_hint), UiLayout::new(Vec2::new(0.0, 16.0)), "B directional · magenta means no usable cells", 9.0, Color::rgb(0.72, 0.76, 0.82));

        let render_options_drawer = add_drawer(context, inspector_content, "RENDER OPTIONS", 4.0 * CHECKBOX_ROW_HEIGHT + 3.0 * DRAWER_CONTENT_SPACING);
        let render_options_content = render_options_drawer.content.expect("render options drawer content exists");
        let (irradiance_row, irradiance_button) = add_checkbox(context, render_options_content, "Global illumination", context.render_settings.irradiance_enabled);
        let (triangle_probe_occlusion_row, triangle_probe_occlusion_button) =
            add_checkbox(context, render_options_content, "Triangle probe occlusion", context.render_settings.triangle_probe_occlusion_enabled);
        let (probe_relocation_row, probe_relocation_button) = add_checkbox(context, render_options_content, "Relocate GI probes", context.gi_radiance_probe_relocation_enabled);
        let (shadows_row, shadows_button) = add_checkbox(context, render_options_content, "Shadows", context.render_settings.shadows_enabled);

        let gizmo_content_height = 3.0 * CHECKBOX_ROW_HEIGHT + 2.0 * DRAWER_CONTENT_SPACING + config.extra_gizmo_content_height;
        let gizmos_drawer = add_drawer(context, inspector_content, "GIZMOS", gizmo_content_height);
        let gizmos_content = gizmos_drawer.content.expect("gizmos drawer content exists");
        let (gizmos_row, gizmos_button) = add_checkbox(context, gizmos_content, "Show gizmos", context.gizmos_enabled);
        let (normals_row, normals_button) = add_checkbox(context, gizmos_content, "Normals", context.normal_gizmos_enabled);
        let (lights_row, lights_button) = add_checkbox(context, gizmos_content, "Lights", context.light_gizmos_enabled);
        let gi_gizmos_content = context.ui.add_container(Some(gizmos_content), UiLayout::new(Vec2::new(0.0, 23.0)), UiAutoLayout::column(0.0, DRAWER_CONTENT_SPACING));
        let (global_probes_row, global_probes_button) = add_checkbox(context, gi_gizmos_content, "GI global probes", context.gi_global_gizmos_enabled);
        let volume_gi_hint = context.ui.add_container(Some(gi_gizmos_content), UiLayout::new(Vec2::new(0.0, 34.0)), UiAutoLayout::column(0.0, 0.0));
        context.ui.add_label_layout(Some(volume_gi_hint), UiLayout::new(Vec2::new(0.0, 16.0)), "Volume GI: show gizmos for its probe grid.", 9.0, Color::rgb(0.72, 0.76, 0.82));
        context.ui.add_label_layout(Some(volume_gi_hint), UiLayout::new(Vec2::new(0.0, 16.0)), "Ray inspectors currently require AABB-SDF GI.", 9.0, Color::rgb(0.72, 0.76, 0.82));
        let sdf_gizmos_content = context.ui.add_container(Some(gi_gizmos_content), UiLayout::new(Vec2::new(0.0, 70.0)), UiAutoLayout::column(0.0, DRAWER_CONTENT_SPACING));
        let (sdf_occupancy_row, sdf_occupancy_button) = add_checkbox(context, sdf_gizmos_content, "SDF occupied voxels", context.sdf_occupancy_gizmos_enabled);
        let radiance_inspector = RadianceInspector::new(context, sdf_gizmos_content);
        let surface_inspector = SurfaceInspector::new(context, sdf_gizmos_content);

        let performance_anchor = UiAnchor::TOP_RIGHT;
        let performance_offset = config.performance_offset;
        let performance_panel_layout = UiLayout::anchored(Vec2::new(PANEL_WIDTH, PERFORMANCE_HEIGHT), performance_anchor, performance_anchor, performance_offset);
        let performance_panel = context.ui.add_panel(performance_panel_layout, Color::rgba(0.03, 0.04, 0.06, 0.94));
        let performance_header =
            context.ui.add_toggle_button_layout(None, UiLayout::anchored(Vec2::new(PANEL_WIDTH, HEADER_HEIGHT), performance_anchor, performance_anchor, performance_offset), false);
        context.ui.set_button_text(performance_header, "Performance", 15.0, Color::WHITE);
        context.ui.set_button_text_centered(performance_header, false);
        context.ui.set_button_toggled_colors(performance_header, Color::rgba(0.1, 0.12, 0.17, 1.0), Color::rgba(0.1, 0.12, 0.17, 1.0));
        let performance_content = context.ui.add_container(
            None,
            UiLayout::anchored(Vec2::new(PANEL_WIDTH, PERFORMANCE_HEIGHT - HEADER_HEIGHT), performance_anchor, performance_anchor, performance_offset + Vec2::new(0.0, HEADER_HEIGHT)),
            UiAutoLayout::column(10.0, 2.0),
        );
        let performance_label = context.ui.add_label_layout(Some(performance_content), UiLayout::new(Vec2::new(0.0, 20.0)), "CPU --", 11.0, Color::rgb(0.62, 0.68, 0.78));
        let performance_frame_label = context.ui.add_label_layout(Some(performance_content), UiLayout::new(Vec2::new(0.0, 20.0)), "FPS -- | Frame -- ms", 12.0, Color::WHITE);
        let fps_counter_label = add_fps_counter_overlay(context);
        let performance_graph = context.ui.add_graph_layout(Some(performance_content), UiLayout::new(Vec2::new(0.0, 72.0)), Color::rgb(0.25, 0.75, 1.0), Color::rgba(0.05, 0.07, 0.1, 0.95));
        let performance_window = context.ui.register_panel_window(performance_panel, performance_header, performance_content, HEADER_HEIGHT);

        let mut panels = Self {
            inspector_window,
            display_mode_drawer,
            render_options_drawer,
            gizmos_drawer,
            display_mode_buttons,
            gi_mode_button_container,
            gi_support_hint,
            irradiance_row,
            irradiance_button,
            triangle_probe_occlusion_row,
            triangle_probe_occlusion_button,
            probe_relocation_row,
            probe_relocation_button,
            shadows_row,
            shadows_button,
            gizmos_row,
            normals_row,
            lights_row,
            gizmos_button,
            normals_button,
            lights_button,
            gi_gizmos_content,
            global_probes_row,
            global_probes_button,
            volume_gi_hint,
            sdf_gizmos_content,
            sdf_occupancy_row,
            sdf_occupancy_button,
            performance_content,
            performance_window,
            performance_label,
            performance_frame_label,
            fps_counter_label,
            performance_graph,
            frame_history: Vec::new(),
            fps_counter: FpsCounter::default(),
            fps: None,
            performance_refresh_elapsed: 0.0,
            radiance_inspector,
            surface_inspector,
            movement_keys_active: false,
            extra_gizmo_content_height: config.extra_gizmo_content_height,
        };
        panels.update_feature_availability(context);
        panels.update_layout(context);
        panels
    }

    pub fn gizmos_content(&self) -> UiContainerId {
        self.gizmos_drawer.content.expect("gizmos drawer content exists")
    }

    pub fn update(&mut self, context: &mut RuntimeContext) {
        if !context.ui_enabled {
            self.movement_keys_active = false;
            return;
        }

        let delta_seconds = context.time.delta_seconds();
        if !context.ui.debug_ui_visible() {
            self.movement_keys_active = false;
            self.update_fps_counter(context, delta_seconds);
            return;
        }

        self.update_feature_availability(context);
        update_drawer(context, self.display_mode_drawer, "DISPLAY MODE");
        update_drawer(context, self.render_options_drawer, "RENDER OPTIONS");
        update_drawer(context, self.gizmos_drawer, "GIZMOS");
        self.update_layout(context);

        let modes = [RenderDebugMode::LitMaterials, RenderDebugMode::UnlitMaterials, RenderDebugMode::Wireframe, RenderDebugMode::GiSupport];
        if let Some((index, _)) =
            self.display_mode_buttons.iter().enumerate().find(|(index, button)| *index != render_mode_index(context.render_debug_mode) && context.ui.button_toggled(**button).unwrap_or(false))
        {
            context.render_debug_mode = modes[index];
        }
        for (index, button) in self.display_mode_buttons.iter().enumerate() {
            context.ui.set_button_toggled(*button, index == render_mode_index(context.render_debug_mode));
        }

        let mut settings = context.render_settings;
        if context.has_irradiance_source() {
            settings.irradiance_enabled = read_checkbox(context, self.irradiance_button, settings.irradiance_enabled);
        }
        if context.irradiance_mode == IrradianceMode::AabbSdf && context.has_irradiance_source() {
            settings.triangle_probe_occlusion_enabled = read_checkbox(context, self.triangle_probe_occlusion_button, settings.triangle_probe_occlusion_enabled);
            context.gi_radiance_probe_relocation_enabled = read_checkbox(context, self.probe_relocation_button, context.gi_radiance_probe_relocation_enabled);
        }
        settings.shadows_enabled = read_checkbox(context, self.shadows_button, settings.shadows_enabled);
        context.render_settings = settings;
        context.gizmos_enabled = read_checkbox(context, self.gizmos_button, context.gizmos_enabled);
        context.normal_gizmos_enabled = read_checkbox(context, self.normals_button, context.normal_gizmos_enabled);
        context.light_gizmos_enabled = read_checkbox(context, self.lights_button, context.light_gizmos_enabled);
        if context.has_irradiance_source() {
            context.gi_global_gizmos_enabled = read_checkbox(context, self.global_probes_button, context.gi_global_gizmos_enabled);
        }
        if context.irradiance_mode == IrradianceMode::AabbSdf && context.has_irradiance_source() {
            context.sdf_occupancy_gizmos_enabled = read_checkbox(context, self.sdf_occupancy_button, context.sdf_occupancy_gizmos_enabled);
        }
        let sdf_gi_available = context.render_settings.irradiance_enabled && context.irradiance_mode == IrradianceMode::AabbSdf && context.has_irradiance_source();
        self.movement_keys_active = self.radiance_inspector.update(context, sdf_gi_available);
        self.surface_inspector.update(context, sdf_gi_available);

        self.update_fps_counter(context, delta_seconds);
        self.performance_refresh_elapsed += delta_seconds;
        self.frame_history.push(delta_seconds * 1000.0);
        if self.frame_history.len() > 48 {
            self.frame_history.remove(0);
        }
        if self.performance_refresh_elapsed >= PERFORMANCE_REFRESH_SECONDS {
            self.performance_refresh_elapsed %= PERFORMANCE_REFRESH_SECONDS;
            let stats = context.performance_stats;
            context.ui.set_label_text(
                self.performance_label,
                format!("CPU {:.1} ms | SDF CPU {:.2} GPU {:.2} | GI {:.2} ms", stats.frame_cpu_ms, stats.sdf_build_cpu_ms, stats.gpu_sdf_update_ms, stats.gpu_sdf_gi_ms),
            );
            let fps_text = self.fps.map_or_else(|| "--".into(), |fps| format!("{fps:.1}"));
            context.ui.set_label_text(self.performance_frame_label, format!("FPS {fps_text} | Frame {:.2} ms", delta_seconds * 1000.0));
            context.ui.set_graph_values(self.performance_graph, &self.frame_history);
        }
    }

    fn update_fps_counter(&mut self, context: &mut RuntimeContext, delta_seconds: f32) {
        if let Some(fps) = self.fps_counter.update(delta_seconds) {
            self.fps = Some(fps);
            context.ui.set_label_text(self.fps_counter_label, format_fps_counter(fps));
        }
    }

    pub fn movement_keys_active(&self) -> bool {
        self.movement_keys_active
    }

    fn update_feature_availability(&mut self, context: &mut RuntimeContext) {
        let gi_source_available = context.has_irradiance_source();
        let gi_enabled = gi_source_available && context.render_settings.irradiance_enabled;
        let sdf_gi_available = gi_source_available && context.irradiance_mode == IrradianceMode::AabbSdf;
        let sdf_gi_enabled = sdf_gi_available && context.render_settings.irradiance_enabled;
        if !sdf_gi_enabled && context.render_debug_mode == RenderDebugMode::GiSupport {
            context.render_debug_mode = RenderDebugMode::LitMaterials;
        }

        context.ui.set_container_visible(self.gi_mode_button_container, sdf_gi_enabled);
        context.ui.set_container_visible(self.gi_support_hint, sdf_gi_enabled);
        self.display_mode_drawer.content_height = 24.0 + if sdf_gi_enabled { 3.0 + 32.0 } else { 0.0 };
        if let Some(content) = self.display_mode_drawer.content {
            context.ui.set_container_size(content, Vec2::new(0.0, self.display_mode_drawer.content_height));
        }
        context.ui.set_container_visible(self.irradiance_row, gi_source_available);
        context.ui.set_container_visible(self.triangle_probe_occlusion_row, sdf_gi_available);
        context.ui.set_container_visible(self.probe_relocation_row, sdf_gi_available);
        context.ui.set_container_visible(self.shadows_row, true);
        context.ui.set_container_visible(self.gizmos_row, true);
        context.ui.set_container_visible(self.normals_row, true);
        context.ui.set_container_visible(self.lights_row, true);
        context.ui.set_container_visible(self.gi_gizmos_content, gi_enabled);
        context.ui.set_container_visible(self.global_probes_row, gi_enabled);
        context.ui.set_container_visible(self.volume_gi_hint, gi_enabled && !sdf_gi_available);
        context.ui.set_container_visible(self.sdf_gizmos_content, sdf_gi_enabled);
        context.ui.set_container_visible(self.sdf_occupancy_row, sdf_gi_enabled);

        let render_options_count = 1 + usize::from(gi_source_available) + 2 * usize::from(sdf_gi_available);
        let render_options_height = render_options_count as f32 * CHECKBOX_ROW_HEIGHT + (render_options_count - 1) as f32 * DRAWER_CONTENT_SPACING;
        self.render_options_drawer.content_height = render_options_height;
        if let Some(content) = self.render_options_drawer.content {
            context.ui.set_container_size(content, Vec2::new(0.0, render_options_height));
        }

        let sdf_content_height = CHECKBOX_ROW_HEIGHT + 2.0 * 22.0 + 2.0 * DRAWER_CONTENT_SPACING;
        context.ui.set_container_size(self.sdf_gizmos_content, Vec2::new(0.0, sdf_content_height));
        let gi_content_height = if sdf_gi_enabled {
            CHECKBOX_ROW_HEIGHT + DRAWER_CONTENT_SPACING + sdf_content_height
        } else if gi_enabled {
            CHECKBOX_ROW_HEIGHT + DRAWER_CONTENT_SPACING + 34.0
        } else {
            CHECKBOX_ROW_HEIGHT
        };
        context.ui.set_container_size(self.gi_gizmos_content, Vec2::new(0.0, gi_content_height));
        let gizmo_row_count = if gi_enabled { 4 } else { 3 };
        let gizmo_content_height =
            3.0 * CHECKBOX_ROW_HEIGHT + (gizmo_row_count - 1) as f32 * DRAWER_CONTENT_SPACING + if gi_enabled { gi_content_height } else { 0.0 } + self.extra_gizmo_content_height;
        self.gizmos_drawer.content_height = gizmo_content_height;
        if let Some(content) = self.gizmos_drawer.content {
            context.ui.set_container_size(content, Vec2::new(0.0, gizmo_content_height));
        }
    }

    fn update_layout(&mut self, context: &mut RuntimeContext) {
        let drawers = [self.display_mode_drawer, self.render_options_drawer, self.gizmos_drawer];
        let expanded_drawer_count = drawers.iter().filter(|drawer| drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false)).count();
        let expanded_content_height =
            drawers.iter().filter(|drawer| drawer.header.and_then(|button| context.ui.button_toggled(button)).unwrap_or(false)).map(|drawer| drawer.content_height).sum::<f32>();
        let visible_stack_children = drawers.len() + expanded_drawer_count;
        let stack_height = 20.0 + drawers.len() as f32 * DRAWER_HEADER_HEIGHT + expanded_content_height + 3.0 * visible_stack_children.saturating_sub(1) as f32;
        let panel_height = HEADER_HEIGHT + 6.0 + stack_height;
        context.ui.set_panel_window_expanded_height(self.inspector_window, panel_height);

        let performance_collapsed = context.ui.panel_window_collapsed(self.performance_window).unwrap_or(false);
        context.ui.set_panel_window_expanded_height(self.performance_window, PERFORMANCE_HEIGHT);
        context.ui.set_container_visible(self.performance_content, !performance_collapsed);
    }
}

fn add_drawer(context: &mut RuntimeContext, parent: UiContainerId, title: &str, content_height: f32) -> Drawer {
    let header = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(0.0, DRAWER_HEADER_HEIGHT)), true);
    let content = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, content_height)), UiAutoLayout::column(0.0, DRAWER_CONTENT_SPACING));
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

fn add_checkbox(context: &mut RuntimeContext, parent: UiContainerId, title: &str, enabled: bool) -> (UiContainerId, UiButtonId) {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, CHECKBOX_ROW_HEIGHT)), UiAutoLayout::row(0.0, 8.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 18.0)), title, 12.0, Color::WHITE);
    let button = context.ui.add_toggle_button_layout(Some(row), UiLayout::new(Vec2::new(15.0, 15.0)), enabled);
    set_checkbox_mark(&mut context.ui, button);
    (row, button)
}

fn add_mode_button(context: &mut RuntimeContext, parent: UiContainerId, title: &str, selected: bool) -> UiButtonId {
    let button = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(68.0, 22.0)), selected);
    context.ui.set_button_text(button, title, 11.0, Color::WHITE);
    button
}

fn render_mode_index(mode: RenderDebugMode) -> usize {
    match mode {
        RenderDebugMode::LitMaterials => 0,
        RenderDebugMode::UnlitMaterials => 1,
        RenderDebugMode::Wireframe => 2,
        RenderDebugMode::ShadowVisibility => 0,
        RenderDebugMode::GiOnly => 0,
        RenderDebugMode::GiSupport => 3,
    }
}

fn read_checkbox(context: &mut RuntimeContext, button: UiButtonId, fallback: bool) -> bool {
    let enabled = context.ui.button_toggled(button).unwrap_or(fallback);
    set_checkbox_mark(&mut context.ui, button);
    enabled
}

fn set_checkbox_mark(context: &mut labyrinth::UiCanvas, button: UiButtonId) {
    let text = if context.button_toggled(button).unwrap_or(false) { "✓" } else { "" };
    context.set_button_text(button, text, 12.0, Color::WHITE);
}
