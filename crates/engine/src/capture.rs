use std::fs;
use std::path::{Component, Path, PathBuf};

use math::{Camera, Vec3, lerp_vec3};
use renderer::{CapturedDepthBuffer, Renderer};
use scripting::{CameraPose, CaptureAction, CapturePlan, CaptureRenderMode, compile_capture_script};

use crate::scene::{CameraId, Scene};

pub struct DebugCapture {
    plan: CapturePlan,
    output_root: PathBuf,
    action_index: usize,
    wait_remaining: f32,
    movement: Option<Movement>,
    pending_capture: Option<String>,
    pending_report: Option<String>,
    render_effects: Option<(bool, bool, bool)>,
    ambient_occlusion: Option<bool>,
    direct_light_visibility: Option<bool>,
    shadow_quality: Option<u8>,
    render_mode: Option<CaptureRenderMode>,
    ui_enabled: Option<bool>,
    exit_requested: bool,
}

struct Movement {
    start: CameraPose,
    target: CameraPose,
    elapsed: f32,
}

impl DebugCapture {
    pub fn from_script(source: &str, output_root: PathBuf) -> Result<Self, String> {
        let plan = compile_capture_script(source)?;
        validate_relative_path(Path::new(&plan.settings.output_directory))?;
        Ok(Self {
            plan,
            output_root,
            action_index: 0,
            wait_remaining: 0.0,
            movement: None,
            pending_capture: None,
            pending_report: None,
            render_effects: None,
            ambient_occlusion: None,
            direct_light_visibility: None,
            shadow_quality: None,
            render_mode: None,
            ui_enabled: None,
            exit_requested: false,
        })
    }

    pub fn advance(&mut self, scene: &mut Scene, active_camera: Option<CameraId>, delta_seconds: f32) -> Result<(), String> {
        let camera_id = active_camera.ok_or("capture script requires an active camera")?;
        let camera = scene.get_camera_mut(camera_id).ok_or("capture script camera was removed")?;
        let mut remaining = delta_seconds.max(0.0);
        loop {
            if self.action_index >= self.plan.actions.len() {
                self.exit_requested = true;
                return Ok(());
            }
            match &self.plan.actions[self.action_index] {
                CaptureAction::SetRenderEffects { shadows, irradiance, gizmos } => {
                    self.render_effects = Some((*shadows, *irradiance, *gizmos));
                    self.action_index += 1;
                }
                CaptureAction::SetAmbientOcclusion(enabled) => {
                    self.ambient_occlusion = Some(*enabled);
                    self.action_index += 1;
                }
                CaptureAction::SetDirectLightVisibility(enabled) => {
                    self.direct_light_visibility = Some(*enabled);
                    self.action_index += 1;
                }
                CaptureAction::SetShadowQuality(quality) => {
                    self.shadow_quality = Some(*quality);
                    self.action_index += 1;
                }
                CaptureAction::SetRenderMode(mode) => {
                    self.render_mode = Some(*mode);
                    self.action_index += 1;
                }
                CaptureAction::SetUiEnabled(enabled) => {
                    self.ui_enabled = Some(*enabled);
                    self.action_index += 1;
                }
                CaptureAction::SetCamera(pose) => {
                    apply_pose(camera, *pose);
                    self.action_index += 1;
                }
                CaptureAction::MoveCamera { pose, duration } => {
                    if self.movement.is_none() {
                        self.movement = Some(Movement { start: current_pose(camera), target: *pose, elapsed: 0.0 });
                    }
                    let movement = self.movement.as_mut().ok_or("capture movement state was not initialized")?;
                    let step = (remaining).min((*duration - movement.elapsed).max(0.0));
                    movement.elapsed += step;
                    remaining -= step;
                    let amount = if *duration <= 0.0 { 1.0 } else { (movement.elapsed / *duration).clamp(0.0, 1.0) };
                    apply_pose(camera, interpolate_pose(movement.start, movement.target, amount));
                    if amount >= 1.0 {
                        self.movement = None;
                        self.action_index += 1;
                    } else {
                        return Ok(());
                    }
                }
                CaptureAction::Wait(seconds) => {
                    if self.wait_remaining == 0.0 {
                        self.wait_remaining = *seconds;
                    }
                    let step = remaining.min(self.wait_remaining);
                    self.wait_remaining -= step;
                    remaining -= step;
                    if self.wait_remaining > 0.0 {
                        return Ok(());
                    }
                    self.action_index += 1;
                }
                CaptureAction::Screenshot(name) => {
                    if self.pending_capture.is_some() {
                        return Err("capture script requested more than one screenshot in a frame".into());
                    }
                    self.pending_capture = Some(name.clone());
                    self.action_index += 1;
                    return Ok(());
                }
                CaptureAction::PerformanceReport(name) => {
                    if self.pending_report.is_some() {
                        return Err("capture script requested more than one performance report in a frame".into());
                    }
                    self.pending_report = Some(name.clone());
                    self.action_index += 1;
                    return Ok(());
                }
                CaptureAction::Exit => {
                    self.exit_requested = true;
                    self.action_index += 1;
                    return Ok(());
                }
            }
        }
    }

    pub fn capture_pending(&mut self, renderer: &mut Renderer, camera: &Camera) -> Result<(), String> {
        let Some(name) = self.pending_capture.take() else {
            return Ok(());
        };
        if self.plan.settings.include_ui {
            return Err("capture script UI output is not implemented yet".into());
        }
        let frame = renderer.capture(camera, self.plan.settings.width, self.plan.settings.height)?;
        let directory = self.output_root.join(&self.plan.settings.output_directory);
        fs::create_dir_all(&directory).map_err(|error| format!("failed to create capture directory: {error}"))?;
        let file_name = safe_file_name(&name)?;
        let path = directory.join(format!("{file_name}.png"));
        write_png(&path, &frame)?;
        write_depth_capture(&directory, file_name, "scene_depth", &frame.scene_depth, Some((camera.near_clip, camera.far_clip)))?;
        write_depth_capture(&directory, file_name, "directional_shadow", &frame.directional_shadow, None)?;
        write_depth_capture(&directory, file_name, "spot_shadow", &frame.spot_shadow, None)?;
        write_depth_capture(&directory, file_name, "point_shadow", &frame.point_shadow, None)?;
        eprintln!("Capture written to {}", path.display());
        Ok(())
    }

    pub fn report_pending(&mut self, renderer: &Renderer) -> Result<(), String> {
        let Some(name) = self.pending_report.take() else {
            return Ok(());
        };
        let directory = self.output_root.join(&self.plan.settings.output_directory);
        fs::create_dir_all(&directory).map_err(|error| format!("failed to create report directory: {error}"))?;
        let file_name = safe_file_name(&name)?;
        let path = directory.join(format!("{file_name}.txt"));
        let stats = renderer.performance_stats();
        let contents = format!(
            "frame_cpu_ms={:.3}\ngpu_shadow_ms={:.3}\ngpu_scene_ms={:.3}\ngpu_gizmo_ms={:.3}\ngpu_ui_ms={:.3}\ngpu_total_ms={:.3}\nlight_update_ms={:.3}\ninstance_update_ms={:.3}\nprobe_update_ms={:.3}\nirradiance_upload_ms={:.3}\nvisibility_rebuild_ms={:.3}\nprobes_updated={}\nprobe_budget={}\nstatic_samples={}\ndynamic_samples={}\nregions={}\n",
            stats.frame_cpu_ms,
            stats.gpu_shadow_ms,
            stats.gpu_scene_ms,
            stats.gpu_gizmo_ms,
            stats.gpu_ui_ms,
            stats.gpu_total_ms,
            stats.light_update_ms,
            stats.instance_update_ms,
            stats.probe_update_ms,
            stats.irradiance_upload_ms,
            stats.visibility_rebuild_ms,
            stats.probes_updated,
            stats.probe_budget,
            stats.static_sample_count,
            stats.dynamic_sample_count,
            stats.region_count,
        );
        fs::write(&path, contents).map_err(|error| format!("failed to write performance report: {error}"))?;
        eprintln!("Performance report written to {}", path.display());
        Ok(())
    }

    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    pub fn has_pending_capture(&self) -> bool {
        self.pending_capture.is_some()
    }

    pub fn has_pending_report(&self) -> bool {
        self.pending_report.is_some()
    }

    pub fn render_effects(&self) -> Option<(bool, bool, bool)> {
        self.render_effects
    }

    pub fn render_mode(&self) -> Option<CaptureRenderMode> {
        self.render_mode
    }

    pub fn shadow_quality(&self) -> Option<u8> {
        self.shadow_quality
    }

    pub fn ambient_occlusion(&self) -> Option<bool> {
        self.ambient_occlusion
    }

    pub fn direct_light_visibility(&self) -> Option<bool> {
        self.direct_light_visibility
    }

    pub fn ui_enabled(&self) -> Option<bool> {
        self.ui_enabled
    }
}

fn current_pose(camera: &Camera) -> CameraPose {
    let forward = camera.transform.rotation * Vec3::NEG_Z;
    CameraPose { position: camera.transform.position.to_array(), target: (camera.transform.position + forward).to_array() }
}

fn apply_pose(camera: &mut Camera, pose: CameraPose) {
    camera.transform.position = Vec3::from_array(pose.position);
    camera.look_at(Vec3::from_array(pose.target));
}

fn interpolate_pose(start: CameraPose, target: CameraPose, amount: f32) -> CameraPose {
    CameraPose {
        position: lerp_vec3(Vec3::from_array(start.position), Vec3::from_array(target.position), amount).to_array(),
        target: lerp_vec3(Vec3::from_array(start.target), Vec3::from_array(target.target), amount).to_array(),
    }
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.is_absolute() || path.components().any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err("capture output directory must be relative and cannot contain parent traversal".into());
    }
    Ok(())
}

fn safe_file_name(name: &str) -> Result<&str, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        return Err("capture names must be simple file names".into());
    }
    Ok(name.strip_suffix(".png").unwrap_or(name))
}

fn write_png(path: &Path, frame: &renderer::CapturedFrame) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|error| format!("failed to create capture file: {error}"))?;
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| format!("failed to write PNG header: {error}"))?;
    writer.write_image_data(&frame.rgba8).map_err(|error| format!("failed to write PNG pixels: {error}"))
}

fn write_depth_capture(directory: &Path, base_name: &str, buffer_name: &str, buffer: &CapturedDepthBuffer, camera_clip: Option<(f32, f32)>) -> Result<(), String> {
    for (layer, values) in buffer.layers.iter().enumerate() {
        let suffix = if buffer.layers.len() == 1 { String::new() } else { format!("_{layer}") };
        let preview_path = directory.join(format!("{base_name}_{buffer_name}{suffix}.png"));
        write_depth_preview(&preview_path, buffer.width, buffer.height, values, camera_clip)?;
    }
    Ok(())
}

fn write_depth_preview(path: &Path, width: u32, height: u32, values: &[f32], camera_clip: Option<(f32, f32)>) -> Result<(), String> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for value in values.iter().take((width * height) as usize) {
        let value = if let Some((near, far)) = camera_clip {
            let near = near.max(0.001);
            let far = far.max(near + 0.001);
            let depth = value.clamp(0.0, 1.0);
            let linear_distance = near * far / (far - depth * (far - near));
            ((linear_distance - near) / (far - near)).clamp(0.0, 1.0)
        } else if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let value = (1.0 - value).powf(0.35);
        let byte = (value * 255.0).round() as u8;
        pixels.extend_from_slice(&[byte, byte, byte, 255]);
    }
    let file = fs::File::create(path).map_err(|error| format!("failed to create depth preview: {error}"))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| format!("failed to write depth preview header: {error}"))?;
    writer.write_image_data(&pixels).map_err(|error| format!("failed to write depth preview pixels: {error}"))
}
