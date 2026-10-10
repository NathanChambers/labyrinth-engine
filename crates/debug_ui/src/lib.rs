use labyrinth::{Color, RenderDebugMode, RuntimeContext, UiAnchor, UiAutoLayout, UiButtonId, UiContainerId, UiGraphId, UiLabelId, UiLayout, UiPanelWindowId, Vec2};

mod radiance_inspector;
mod surface_inspector;

pub use radiance_inspector::RadianceInspector;
pub use surface_inspector::SurfaceInspector;

const PANEL_WIDTH: f32 = 320.0;
const HEADER_HEIGHT: f32 = 26.0;
const DRAWER_HEADER_HEIGHT: f32 = 22.0;
const CHECKBOX_ROW_HEIGHT: f32 = 20.0;
const PERFORMANCE_HEIGHT: f32 = 180.0;

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
    irradiance_button: UiButtonId,
    screen_probe_gi_button: UiButtonId,
    shadows_button: UiButtonId,
    gizmos_button: UiButtonId,
    normals_button: UiButtonId,
    lights_button: UiButtonId,
    global_probes_button: UiButtonId,
    screen_probes_button: UiButtonId,
    sdf_occupancy_button: UiButtonId,
    performance_content: UiContainerId,
    performance_window: UiPanelWindowId,
    performance_label: UiLabelId,
    performance_frame_label: UiLabelId,
    performance_graph: UiGraphId,
    frame_history: Vec<f32>,
    smoothed_fps: f32,
    radiance_inspector: RadianceInspector,
    surface_inspector: SurfaceInspector,
    movement_keys_active: bool,
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

        let display_mode_drawer = add_drawer(context, inspector_content, "DISPLAY MODE", 70.0);
        let mode_row = context.ui.add_container(display_mode_drawer.content, UiLayout::new(Vec2::new(0.0, 24.0)), UiAutoLayout::row(0.0, 6.0));
        let display_mode_buttons = [
            add_mode_button(context, mode_row, "Lit", context.render_debug_mode == RenderDebugMode::LitMaterials),
            add_mode_button(context, mode_row, "Unlit", context.render_debug_mode == RenderDebugMode::UnlitMaterials),
            add_mode_button(context, mode_row, "Wire", context.render_debug_mode == RenderDebugMode::Wireframe),
            add_mode_button(context, mode_row, "GI Support", context.render_debug_mode == RenderDebugMode::GiSupport),
        ];
        context.ui.add_label_layout(display_mode_drawer.content, UiLayout::new(Vec2::new(0.0, 16.0)), "GI Support: R valid cells · G spatial weight", 9.0, Color::rgb(0.72, 0.76, 0.82));
        context.ui.add_label_layout(display_mode_drawer.content, UiLayout::new(Vec2::new(0.0, 16.0)), "B directional support · magenta no usable cells", 9.0, Color::rgb(0.72, 0.76, 0.82));

        let render_options_drawer = add_drawer(context, inspector_content, "RENDER OPTIONS", 3.0 * CHECKBOX_ROW_HEIGHT);
        let render_options_content = render_options_drawer.content.expect("render options drawer content exists");
        let irradiance_button = add_checkbox(context, render_options_content, "Global illumination", context.render_settings.irradiance_enabled);
        let screen_probe_gi_button = add_checkbox(context, render_options_content, "Screen probe GI", context.render_settings.surface_probes_enabled);
        let shadows_button = add_checkbox(context, render_options_content, "Shadows", context.render_settings.shadows_enabled);

        let gizmo_content_height = 6.0 * CHECKBOX_ROW_HEIGHT + config.extra_gizmo_content_height + 44.0;
        let gizmos_drawer = add_drawer(context, inspector_content, "GIZMOS", gizmo_content_height);
        let gizmos_content = gizmos_drawer.content.expect("gizmos drawer content exists");
        let gizmos_button = add_checkbox(context, gizmos_content, "Show gizmos", context.gizmos_enabled);
        let normals_button = add_checkbox(context, gizmos_content, "Normals", context.normal_gizmos_enabled);
        let lights_button = add_checkbox(context, gizmos_content, "Lights", context.light_gizmos_enabled);
        let global_probes_button = add_checkbox(context, gizmos_content, "GI global probes", context.gi_global_gizmos_enabled);
        let screen_probes_button = add_checkbox(context, gizmos_content, "Screen space probes", context.gi_gizmos_enabled);
        let sdf_occupancy_button = add_checkbox(context, gizmos_content, "SDF occupied voxels", context.sdf_occupancy_gizmos_enabled);
        let radiance_inspector = RadianceInspector::new(context, gizmos_content);
        let surface_inspector = SurfaceInspector::new(context, gizmos_content);

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
        let performance_graph = context.ui.add_graph_layout(Some(performance_content), UiLayout::new(Vec2::new(0.0, 72.0)), Color::rgb(0.25, 0.75, 1.0), Color::rgba(0.05, 0.07, 0.1, 0.95));
        let performance_window = context.ui.register_panel_window(performance_panel, performance_header, performance_content, HEADER_HEIGHT);

        let mut panels = Self {
            inspector_window,
            display_mode_drawer,
            render_options_drawer,
            gizmos_drawer,
            display_mode_buttons,
            irradiance_button,
            screen_probe_gi_button,
            shadows_button,
            gizmos_button,
            normals_button,
            lights_button,
            global_probes_button,
            screen_probes_button,
            sdf_occupancy_button,
            performance_content,
            performance_window,
            performance_label,
            performance_frame_label,
            performance_graph,
            frame_history: Vec::new(),
            smoothed_fps: 0.0,
            radiance_inspector,
            surface_inspector,
            movement_keys_active: false,
        };
        panels.update_layout(context);
        panels
    }

    pub fn gizmos_content(&self) -> UiContainerId {
        self.gizmos_drawer.content.expect("gizmos drawer content exists")
    }

    pub fn update(&mut self, context: &mut RuntimeContext) {
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
        settings.irradiance_enabled = read_checkbox(context, self.irradiance_button, settings.irradiance_enabled);
        settings.surface_probes_enabled = read_checkbox(context, self.screen_probe_gi_button, settings.surface_probes_enabled);
        settings.shadows_enabled = read_checkbox(context, self.shadows_button, settings.shadows_enabled);
        context.render_settings = settings;
        context.gizmos_enabled = read_checkbox(context, self.gizmos_button, context.gizmos_enabled);
        context.normal_gizmos_enabled = read_checkbox(context, self.normals_button, context.normal_gizmos_enabled);
        context.light_gizmos_enabled = read_checkbox(context, self.lights_button, context.light_gizmos_enabled);
        context.gi_global_gizmos_enabled = read_checkbox(context, self.global_probes_button, context.gi_global_gizmos_enabled);
        context.gi_gizmos_enabled = read_checkbox(context, self.screen_probes_button, context.gi_gizmos_enabled);
        context.sdf_occupancy_gizmos_enabled = read_checkbox(context, self.sdf_occupancy_button, context.sdf_occupancy_gizmos_enabled);
        self.movement_keys_active = self.radiance_inspector.update(context);
        self.surface_inspector.update(context);

        let delta_seconds = context.time.delta_seconds().max(0.0001);
        let stats = context.performance_stats;
        context
            .ui
            .set_label_text(self.performance_label, format!("CPU {:.1} ms | SDF {:.2}/{:.2} ms | GI {:.2} ms", stats.frame_cpu_ms, stats.sdf_build_cpu_ms, stats.sdf_upload_ms, stats.gpu_sdf_gi_ms));
        let fps = 1.0 / delta_seconds;
        self.smoothed_fps = if self.smoothed_fps == 0.0 { fps } else { self.smoothed_fps + (fps - self.smoothed_fps) * 0.1 };
        self.frame_history.push(delta_seconds * 1000.0);
        if self.frame_history.len() > 48 {
            self.frame_history.remove(0);
        }
        context.ui.set_label_text(self.performance_frame_label, format!("FPS {:.1} | Frame {:.2} ms", self.smoothed_fps, delta_seconds * 1000.0));
        context.ui.set_graph_values(self.performance_graph, &self.frame_history);
    }

    pub fn movement_keys_active(&self) -> bool {
        self.movement_keys_active
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
    let content = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, content_height)), UiAutoLayout::column(0.0, 3.0));
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

fn add_checkbox(context: &mut RuntimeContext, parent: UiContainerId, title: &str, enabled: bool) -> UiButtonId {
    let row = context.ui.add_container(Some(parent), UiLayout::new(Vec2::new(0.0, CHECKBOX_ROW_HEIGHT)), UiAutoLayout::row(0.0, 8.0));
    context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 18.0)), title, 12.0, Color::WHITE);
    let button = context.ui.add_toggle_button_layout(Some(row), UiLayout::new(Vec2::new(15.0, 15.0)), enabled);
    set_checkbox_mark(&mut context.ui, button);
    button
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
