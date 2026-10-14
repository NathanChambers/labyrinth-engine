use std::path::Path;

use labyrinth::{Application, FontAsset, RuntimeContext, UiLabelId, WindowSettings, run};
use labyrinth_debug_ui::{FpsCounter, add_fps_counter_overlay, format_fps_counter};

const WARMUP_SECONDS: f32 = 2.0;

struct ControlScene {
    profile_seconds: Option<f32>,
    fps_counter_enabled: bool,
    elapsed_seconds: f32,
    frame_intervals_ms: Vec<f32>,
    engine_frame_ms: Vec<f32>,
    fps_label: Option<UiLabelId>,
    fps_counter: FpsCounter,
    font: Option<FontAsset>,
}

impl ControlScene {
    fn new(profile_seconds: Option<f32>, fps_counter_enabled: bool) -> Self {
        Self { profile_seconds, fps_counter_enabled, elapsed_seconds: 0.0, frame_intervals_ms: Vec::new(), engine_frame_ms: Vec::new(), fps_label: None, fps_counter: FpsCounter::default(), font: None }
    }

    fn report_samples(&self) {
        println!("Build profile: {}", if cfg!(debug_assertions) { "debug" } else { "release" });
        println!("Scene entities: meshes=0 cameras=0 lights=0");
        println!("FPS counter: {}", if self.fps_counter_enabled { "enabled" } else { "disabled" });
        println!("Captured frames: {}", self.frame_intervals_ms.len());
        report_series("Frame interval", &self.frame_intervals_ms);

        report_series("Engine CPU frame", &self.engine_frame_ms);
    }
}

impl Application for ControlScene {
    fn setup(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        if self.fps_counter_enabled {
            let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/primitives/assets/fonts/default.ttf");
            let font = FontAsset::from_file(font_path)?;
            context.ui.set_font(font.clone());
            self.font = Some(font);
            self.fps_label = Some(add_fps_counter_overlay(context));
        }
        context.ui_enabled = self.fps_counter_enabled;
        context.cursor_capture_on_scene_click = false;

        Ok(())
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        let delta_seconds = context.time.delta_seconds();
        let sample_frame = self.elapsed_seconds >= WARMUP_SECONDS;
        self.elapsed_seconds += delta_seconds;
        if self.fps_counter_enabled
            && let Some(fps) = self.fps_counter.update(delta_seconds)
        {
            if let Some(label) = self.fps_label {
                context.ui.set_label_text(label, format_fps_counter(fps));
            }
        }

        if self.profile_seconds.is_some() && sample_frame {
            self.frame_intervals_ms.push(delta_seconds * 1000.0);
            self.engine_frame_ms.push(context.performance_stats.frame_cpu_ms);
        }

        if self.profile_seconds.is_some_and(|duration| self.elapsed_seconds >= duration) {
            context.request_exit();
        }

        Ok(())
    }

    fn cleanup(&mut self, _context: &mut RuntimeContext) {
        if self.profile_seconds.is_some() {
            self.report_samples();
        }
    }
}

fn report_series(name: &str, samples: &[f32]) {
    if samples.is_empty() {
        println!("{name}: no samples");
        return;
    }
    let mut sorted_samples = samples.to_vec();
    sorted_samples.sort_by(f32::total_cmp);
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let standard_deviation = (samples.iter().map(|sample| (sample - mean).powi(2)).sum::<f32>() / samples.len() as f32).sqrt();
    let frames_per_second = if name == "Frame interval" { 1000.0 / mean } else { 0.0 };
    println!(
        "{name}: mean={mean:.3} ms stddev={standard_deviation:.3} ms p50={:.3} ms p95={:.3} ms p99={:.3} ms max={:.3} ms{}",
        percentile(&sorted_samples, 0.50),
        percentile(&sorted_samples, 0.95),
        percentile(&sorted_samples, 0.99),
        sorted_samples[sorted_samples.len() - 1],
        if frames_per_second > 0.0 { format!(" mean_fps={frames_per_second:.1}") } else { String::new() },
    );
}

fn percentile(sorted_samples: &[f32], percentile: f32) -> f32 {
    let index = ((sorted_samples.len() - 1) as f32 * percentile).ceil() as usize;

    sorted_samples[index]
}

fn main() -> Result<(), String> {
    let mut settings = WindowSettings { title: "Labyrinth Engine Control Scene".into(), ..WindowSettings::default() };
    let mut profile_seconds = None;
    let mut fps_counter_enabled = true;

    for argument in std::env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--profile-seconds=") {
            let duration = value.parse::<f32>().map_err(|error| format!("invalid profile duration: {error}"))?;
            if !duration.is_finite() || duration < WARMUP_SECONDS + 1.0 {
                return Err(format!("profile duration must be at least {:.0} seconds", WARMUP_SECONDS + 1.0));
            }
            profile_seconds = Some(duration);
        } else if argument == "--windowed" {
            settings.mode = labyrinth::WindowMode::Windowed;
        } else if argument == "--vulkan" {
            settings.graphics_api = labyrinth::GraphicsApi::Vulkan;
        } else if argument == "--no-fps" {
            fps_counter_enabled = false;
        } else {
            return Err(format!("unknown option: {argument}"));
        }
    }

    run(settings, ControlScene::new(profile_seconds, fps_counter_enabled))
}
