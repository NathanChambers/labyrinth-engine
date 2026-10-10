use labyrinth::{Color, IrradianceMode, RendererRadianceSurfaceCandidate, RuntimeContext, UiAnchor, UiAutoLayout, UiButtonId, UiContainerId, UiLabelId, UiLayout, UiPanelWindowId, Vec2};

const PANEL_WIDTH: f32 = 720.0;
const PANEL_HEIGHT: f32 = 680.0;
const HEADER_HEIGHT: f32 = 26.0;
const CANDIDATE_COUNT: usize = labyrinth::RADIANCE_SURFACE_CANDIDATE_COUNT;

pub struct SurfaceInspector {
    open_button: UiButtonId,
    pick_button: UiButtonId,
    content: UiContainerId,
    window: UiPanelWindowId,
    status: UiLabelId,
    field_label: UiLabelId,
    point_label: UiLabelId,
    normal_label: UiLabelId,
    sdf_origin_label: UiLabelId,
    visibility_summary_label: UiLabelId,
    sdf_summary_label: UiLabelId,
    candidate_first_labels: [UiLabelId; CANDIDATE_COUNT],
    candidate_second_labels: [UiLabelId; CANDIDATE_COUNT],
    candidate_third_labels: [UiLabelId; CANDIDATE_COUNT],
    pick_armed: bool,
    has_requested_sample: bool,
}

impl SurfaceInspector {
    pub fn new(context: &mut RuntimeContext, parent: UiContainerId) -> Self {
        let open_button = context.ui.add_toggle_button_layout(Some(parent), UiLayout::new(Vec2::new(0.0, 22.0)), false);
        context.ui.set_button_text(open_button, "GI Surface Inspector", 11.0, Color::WHITE);
        context.ui.set_button_text_centered(open_button, false);
        context.ui.set_button_toggled_colors(open_button, Color::rgb(0.18, 0.32, 0.44), Color::rgb(0.15, 0.2, 0.28));

        let anchor = UiAnchor::TOP_LEFT;
        let offset = Vec2::new(24.0, 20.0);
        let panel = context.ui.add_panel(UiLayout::anchored(Vec2::new(PANEL_WIDTH, PANEL_HEIGHT), anchor, anchor, offset), Color::rgba(0.03, 0.04, 0.06, 0.96));
        let header = context.ui.add_toggle_button_layout(None, UiLayout::anchored(Vec2::new(PANEL_WIDTH, HEADER_HEIGHT), anchor, anchor, offset), false);
        context.ui.set_button_text(header, "GI Surface Inspector", 15.0, Color::WHITE);
        context.ui.set_button_text_centered(header, false);
        context.ui.set_button_toggled_colors(header, Color::rgba(0.1, 0.12, 0.17, 1.0), Color::rgba(0.1, 0.12, 0.17, 1.0));
        let content = context.ui.add_container(
            None,
            UiLayout::anchored(Vec2::new(PANEL_WIDTH, PANEL_HEIGHT - HEADER_HEIGHT), anchor, anchor, offset + Vec2::new(0.0, HEADER_HEIGHT)),
            UiAutoLayout::column(8.0, 3.0),
        );
        let window = context.ui.register_panel_window(panel, header, content, HEADER_HEIGHT);
        context.ui.set_panel_window_visible(window, false);
        let pick_row = context.ui.add_container(Some(content), UiLayout::new(Vec2::new(0.0, 28.0)), UiAutoLayout::row(0.0, 8.0));
        let pick_button = context.ui.add_toggle_button_layout(Some(pick_row), UiLayout::new(Vec2::new(128.0, 28.0)), false);
        context.ui.set_button_text(pick_button, "Pick surface", 12.0, Color::WHITE);
        context.ui.set_button_toggled_colors(pick_button, Color::rgb(0.5, 0.28, 0.12), Color::rgb(0.15, 0.2, 0.28));
        let status = context.ui.add_label_layout(Some(pick_row), UiLayout::new(Vec2::new(0.0, 22.0)), "Open, then pick a scene surface", 12.0, Color::rgb(0.72, 0.76, 0.82));
        let field_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 20.0)), "Active field: scene radiance field", 12.0, Color::rgb(0.72, 0.76, 0.82));
        let point_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 20.0)), "Surface: --", 12.0, Color::WHITE);
        let normal_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 20.0)), "Normal: --", 12.0, Color::rgb(0.2, 0.6, 1.0));
        let sdf_origin_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 20.0)), "SDF start voxel: -- · origin offset: --", 12.0, Color::rgb(0.85, 0.72, 0.35));
        let visibility_summary_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 20.0)), "Triangle visibility: --", 12.0, Color::rgb(0.55, 0.95, 0.62));
        let sdf_summary_label = context.ui.add_label_layout(Some(content), UiLayout::new(Vec2::new(0.0, 20.0)), "SDF comparison: --", 11.0, Color::rgb(1.0, 0.62, 0.22));
        context.ui.add_label_layout(
            Some(content),
            UiLayout::new(Vec2::new(0.0, 20.0)),
            "Rays: green = mesh-clear · red = triangle hit · amber box = SDF occupied · cyan = hit normal · probe marker = contribution",
            11.0,
            Color::rgb(0.48, 0.72, 0.95),
        );
        let mut candidate_first_labels = [None; CANDIDATE_COUNT];
        let mut candidate_second_labels = [None; CANDIDATE_COUNT];
        let mut candidate_third_labels = [None; CANDIDATE_COUNT];
        for index in 0..CANDIDATE_COUNT {
            let row = context.ui.add_container(Some(content), UiLayout::new(Vec2::new(0.0, 52.0)), UiAutoLayout::column(0.0, 0.0));
            candidate_first_labels[index] = Some(context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 17.0)), format!("P{index}: waiting for a surface sample"), 11.0, Color::WHITE));
            candidate_second_labels[index] = Some(context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 17.0)), "", 11.0, Color::rgb(0.72, 0.76, 0.82)));
            candidate_third_labels[index] = Some(context.ui.add_label_layout(Some(row), UiLayout::new(Vec2::new(0.0, 17.0)), "", 11.0, Color::rgb(0.85, 0.72, 0.35)));
        }
        context.ui.set_container_visible(content, false);
        context.ui.set_button_toggled(open_button, false);
        Self {
            open_button,
            pick_button,
            content,
            window,
            status,
            field_label,
            point_label,
            normal_label,
            sdf_origin_label,
            visibility_summary_label,
            sdf_summary_label,
            candidate_first_labels: candidate_first_labels.map(Option::unwrap),
            candidate_second_labels: candidate_second_labels.map(Option::unwrap),
            candidate_third_labels: candidate_third_labels.map(Option::unwrap),
            pick_armed: false,
            has_requested_sample: false,
        }
    }

    pub fn update(&mut self, context: &mut RuntimeContext) {
        let open = context.ui.button_toggled(self.open_button).unwrap_or(false);
        context.ui.set_button_toggled(self.open_button, open);
        let field_enabled = context.irradiance_mode == IrradianceMode::AabbSdf && context.render_settings.irradiance_enabled;
        context.gi_surface_inspector_enabled = open && field_enabled;
        context.ui.set_panel_window_visible(self.window, open);
        let collapsed = context.ui.panel_window_collapsed(self.window).unwrap_or(false);
        context.ui.set_container_visible(self.content, open && !collapsed);
        if !open {
            self.pick_armed = false;
            context.ui.set_button_toggled(self.pick_button, false);
            context.gi_surface_inspector_pick_request = None;
            return;
        }
        if !field_enabled {
            self.pick_armed = false;
            context.ui.set_button_toggled(self.pick_button, false);
            context.cursor_capture_on_scene_click = true;
            context.ui.set_label_text(self.status, "Enable AABB-SDF GI to inspect the scene radiance field");
            return;
        }

        self.pick_armed = !collapsed && context.ui.button_toggled(self.pick_button).unwrap_or(false);
        context.cursor_capture_on_scene_click = !self.pick_armed;
        if self.pick_armed && context.input.left_mouse_pressed() && !context.ui.pointer_press_consumed() && !context.input.cursor_captured() {
            let cursor = context.input.cursor_position();
            let viewport = context.ui.viewport_size();
            if cursor.x >= 0.0 && cursor.y >= 0.0 && cursor.x < viewport.0 as f32 && cursor.y < viewport.1 as f32 {
                context.gi_surface_inspector_pick_request = Some([cursor.x as u32, cursor.y as u32]);
                self.has_requested_sample = true;
                context.ui.set_label_text(self.status, "Sampling depth-prepass surface...");
            }
            self.pick_armed = false;
            context.ui.set_button_toggled(self.pick_button, false);
            context.cursor_capture_on_scene_click = true;
        }

        let inspection = context.gi_surface_inspection;
        let grid = context.gi_radiance_field_grid;
        context.ui.set_label_text(self.field_label, format!("Active field: scene · {} × {} × {} probes", grid.resolution[0], grid.resolution[1], grid.resolution[2]));
        if context.gi_surface_inspection_pending {
            context.ui.set_label_text(self.status, "Sampling depth-prepass surface...");
            context.ui.set_label_text(self.point_label, "Surface: sampling...");
            context.ui.set_label_text(self.normal_label, "Normal: --");
            context.ui.set_label_text(self.sdf_origin_label, "SDF start voxel: pending");
            context.ui.set_label_text(self.visibility_summary_label, "Checking candidate rays against scene triangles...");
            context.ui.set_label_text(self.sdf_summary_label, "SDF voxel checks are included in the pending readback");
            for (index, label) in self.candidate_first_labels.iter().enumerate() {
                context.ui.set_label_text(*label, format!("P{index}: waiting for visibility results"));
                context.ui.set_label_text(self.candidate_second_labels[index], "");
                context.ui.set_label_text(self.candidate_third_labels[index], "");
            }
            return;
        } else if self.has_requested_sample && !inspection.valid {
            context.ui.set_label_text(self.status, "No renderable surface at that pixel");
        } else if !self.has_requested_sample {
            context.ui.set_label_text(self.status, "Pick a surface to inspect its eight candidates");
        }
        if !inspection.valid {
            if self.has_requested_sample && !context.gi_surface_inspection_pending {
                context.ui.set_label_text(self.visibility_summary_label, "No triangle visibility result available");
                context.ui.set_label_text(self.sdf_summary_label, "SDF comparison: no surface sample");
                for (index, label) in self.candidate_first_labels.iter().enumerate() {
                    context.ui.set_label_text(*label, format!("P{index}: no surface sample"));
                    context.ui.set_label_text(self.candidate_second_labels[index], "");
                    context.ui.set_label_text(self.candidate_third_labels[index], "");
                }
            }
            return;
        }
        self.has_requested_sample = true;
        context.ui.set_label_text(self.status, "Showing field contribution and mesh visibility results");
        context.ui.set_label_text(self.point_label, format!("Surface: {}", format_vec3(inspection.world_position)));
        context.ui.set_label_text(self.normal_label, format!("Normal: {}", format_vec3(inspection.normal)));
        let sdf_start_voxel = if inspection.sdf_start_in_bounds { format_vec3i(inspection.sdf_start_voxel) } else { "outside SDF".to_string() };
        let sdf_start_occupancy = if !inspection.sdf_start_in_bounds {
            ""
        } else if inspection.sdf_start_occupied {
            "occupied"
        } else {
            "clear"
        };
        context.ui.set_label_text(self.sdf_origin_label, format!("SDF start voxel: {sdf_start_voxel} · {sdf_start_occupancy} · origin offset {} m", format_vec3(inspection.sdf_origin_offset)));
        if let Some((index, candidate)) = candidate_with_most_contribution(inspection) {
            let mesh_blocked = candidate.triangle_occlusion_tested && candidate.triangle_occluded;
            let sdf_blocked = candidate.has_occupied_voxel_after_start;
            let comparison = match (mesh_blocked, sdf_blocked) {
                (true, true) => "mesh and SDF agree",
                (true, false) => "triangle-only blocker",
                (false, true) => "SDF-only candidate",
                (false, false) => "both clear",
            };
            let mesh_status = if !candidate.triangle_occlusion_tested {
                "not tested"
            } else if mesh_blocked {
                "BLOCKED"
            } else {
                "clear"
            };
            let sdf_status = if sdf_blocked { "occupied" } else { "clear" };
            context.ui.set_label_text(
                self.visibility_summary_label,
                format!("Highest contribution: P{index} · {:.1}% share · mesh {mesh_status} · SDF {sdf_status} · {comparison}", candidate.final_weight * 100.0),
            );
            if candidate.has_occupied_voxel_after_start {
                context.ui.set_label_text(
                    self.sdf_summary_label,
                    format!(
                        "P{index} first occupied SDF voxel {} · bounds {} to {} m",
                        format_vec3i(candidate.first_occupied_after_start),
                        format_vec3(candidate.first_occupied_after_start_minimum),
                        format_vec3(candidate.first_occupied_after_start_maximum)
                    ),
                );
            } else {
                context.ui.set_label_text(self.sdf_summary_label, format!("P{index}: no occupied SDF voxel along the segment"));
            }
        } else {
            context.ui.set_label_text(self.visibility_summary_label, "No valid probe candidates to inspect");
            context.ui.set_label_text(self.sdf_summary_label, "SDF comparison: no valid probe candidates");
        }
        if !inspection.inside_field {
            if !context.gi_surface_inspection_pending {
                context.ui.set_label_text(self.status, "Surface is outside the scene radiance field");
            }
            context.ui.set_label_text(self.visibility_summary_label, "No candidate probe segments to inspect");
            context.ui.set_label_text(self.sdf_summary_label, "SDF comparison: surface is outside the field");
            for (index, label) in self.candidate_first_labels.iter().enumerate() {
                context.ui.set_label_text(*label, format!("P{index}: no field candidate"));
                context.ui.set_label_text(self.candidate_second_labels[index], "");
                context.ui.set_label_text(self.candidate_third_labels[index], "");
            }
            return;
        }
        for (index, candidate) in inspection.candidates.iter().enumerate() {
            let (first, second, third) = candidate_text(index, *candidate);
            context.ui.set_label_text(self.candidate_first_labels[index], first);
            context.ui.set_label_text(self.candidate_second_labels[index], second);
            context.ui.set_label_text(self.candidate_third_labels[index], third);
            context.ui.set_label_color(self.candidate_second_labels[index], if candidate.probe_valid { color(candidate.contribution_radiance) } else { Color::rgb(1.0, 0.35, 0.25) });
            context.ui.set_label_color(self.candidate_third_labels[index], candidate_visibility_color(*candidate));
        }
    }
}

fn candidate_text(index: usize, candidate: RendererRadianceSurfaceCandidate) -> (String, String, String) {
    let [x, y, z] = candidate.grid_position;
    let probe = format_vec3(candidate.probe_position);
    (
        format!(
            "P{index}: grid ({x},{y},{z}) · {} probe {probe} · trilinear {:.0}% · same-side {:.0}%",
            if candidate.probe_valid { "valid" } else { "INVALID" },
            candidate.spatial_weight * 100.0,
            candidate.side_weight * 100.0
        ),
        format!(
            "Directional {} · support {:.0}% · final {:.0}% · contribution {}",
            format_vec3(candidate.directional_radiance),
            candidate.directional_support * 100.0,
            candidate.final_weight * 100.0,
            format_vec3(candidate.contribution_radiance)
        ),
        candidate_visibility_text(candidate),
    )
}

fn format_vec3i(value: [i32; 3]) -> String {
    format!("({}, {}, {})", value[0], value[1], value[2])
}

fn candidate_with_most_contribution(inspection: labyrinth::RendererRadianceSurfaceInspection) -> Option<(usize, RendererRadianceSurfaceCandidate)> {
    inspection.candidates.iter().copied().enumerate().filter(|(_, candidate)| candidate.probe_valid).max_by(|(_, first), (_, second)| first.final_weight.total_cmp(&second.final_weight))
}

fn candidate_visibility_text(candidate: RendererRadianceSurfaceCandidate) -> String {
    if !candidate.triangle_occlusion_tested {
        return "Mesh: not tested · SDF voxel: --".to_string();
    }
    let sdf_status = if candidate.has_occupied_voxel_after_start { format!("SDF voxel {}", format_vec3i(candidate.first_occupied_after_start)) } else { "SDF clear".to_string() };
    if candidate.triangle_occluded {
        format!(
            "Mesh HIT I{}/T{} d={:.2} @ {} n={} b={} · {sdf_status}",
            candidate.triangle_instance_index,
            candidate.triangle_index,
            candidate.triangle_hit_distance,
            format_vec3(candidate.triangle_hit_position),
            format_vec3(candidate.triangle_hit_normal),
            format_vec3(candidate.triangle_hit_barycentric)
        )
    } else {
        format!("Mesh clear · {sdf_status}")
    }
}

fn candidate_visibility_color(candidate: RendererRadianceSurfaceCandidate) -> Color {
    if !candidate.triangle_occlusion_tested {
        Color::rgb(0.52, 0.56, 0.62)
    } else if candidate.triangle_occluded {
        Color::rgb(1.0, 0.32, 0.2)
    } else if candidate.has_occupied_voxel_after_start {
        Color::rgb(1.0, 0.68, 0.2)
    } else {
        Color::rgb(0.35, 0.95, 0.48)
    }
}

fn format_vec3(value: [f32; 3]) -> String {
    format!("{:.2}, {:.2}, {:.2}", value[0], value[1], value[2])
}

fn color(value: [f32; 3]) -> Color {
    Color::rgb(display_radiance(value[0]), display_radiance(value[1]), display_radiance(value[2]))
}

fn display_radiance(value: f32) -> f32 {
    let value = value.max(0.0);
    value / (1.0 + value)
}
