use std::path::{Path, PathBuf};

use asset_import::load_fbx_allow_missing_textures;
use labyrinth::{Application, Camera, Color, DirectionalLight, FontAsset, GraphicsApi, IrradianceMode, RuntimeContext, UiAnchor, Vec2, Vec3, WindowMode, WindowSettings, run};
use labyrinth_debug_ui::{DebugPanels, DebugPanelsConfig, FreeCamera};

struct SponzaDemo {
    model_path: PathBuf,
    debug_panels: Option<DebugPanels>,
    free_camera: FreeCamera,
}

impl Application for SponzaDemo {
    fn setup(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        context.irradiance_mode = IrradianceMode::AabbSdf;
        context.render_settings.irradiance_enabled = false;
        context.render_settings.triangle_probe_occlusion_enabled = false;
        context.render_settings.shadows_enabled = false;
        context.render_settings.ambient_intensity = 0.08;

        let imported_meshes = load_fbx_allow_missing_textures(&self.model_path)?;
        for imported in imported_meshes {
            context.scene.spawn_mesh_with_material(imported.mesh, imported.transform, imported.material);
        }

        let camera = context.scene.spawn_camera(Camera { clear_color: Color::rgb(0.035, 0.045, 0.065), ..Camera::default() });
        if let Some(camera) = context.scene.get_camera_mut(camera) {
            camera.transform.position = Vec3::new(0.0, 5.0, 0.0);
            camera.look_at(Vec3::new(10.0, 5.8, 0.0));
            self.free_camera = FreeCamera::from_rotation(camera.transform.rotation);
        }
        context.active_camera = Some(camera);
        let sun = context.scene.spawn_light(DirectionalLight { direction: Vec3::new(0.35, 0.82, 0.45).normalize(), color: Color::rgb(1.0, 0.9, 0.76), intensity: 1.4 });
        context.active_lights = vec![sun];
        context.sky_lighting = labyrinth::RendererSkyLighting { color: Color::rgb(0.32, 0.38, 0.5), intensity: 0.35 };
        let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/default.ttf");
        context.ui.set_font(FontAsset::from_file(font_path)?);
        self.debug_panels =
            Some(DebugPanels::new(context, DebugPanelsConfig::new("Render Debug").with_position(UiAnchor::TOP_RIGHT, Vec2::new(-24.0, 24.0)).with_performance_offset(Vec2::new(-24.0, 500.0))));
        Ok(())
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        self.debug_panels.as_mut().expect("shared debug panels are initialized").update(context);
        let movement_keys_active = self.debug_panels.as_ref().is_some_and(DebugPanels::movement_keys_active);
        let camera_id = context.active_camera.ok_or("active camera was not assigned")?;
        let delta_seconds = context.time.delta_seconds();
        self.free_camera.update(context, camera_id, delta_seconds, movement_keys_active)?;
        Ok(())
    }
}

fn main() -> Result<(), String> {
    let mut settings = WindowSettings { title: "Labyrinth Engine — Sponza".into(), ..WindowSettings::default() };
    let mut model_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/meshes/Sponza.fbx");
    for argument in std::env::args().skip(1) {
        if let Some(path) = argument.strip_prefix("--model=") {
            model_path = PathBuf::from(path);
            continue;
        }
        match argument.as_str() {
            "--windowed" => settings.mode = WindowMode::Windowed,
            "--borderless" => settings.mode = WindowMode::Borderless,
            "--fullscreen" => settings.mode = WindowMode::Exclusive,
            "--vulkan" => settings.graphics_api = GraphicsApi::Vulkan,
            "--dx12" => settings.graphics_api = GraphicsApi::DirectX12,
            "--opengl" => settings.graphics_api = GraphicsApi::OpenGl,
            "--metal" => settings.graphics_api = GraphicsApi::Metal,
            argument if argument.starts_with("--capture-script=") => settings.debug_script = Some(argument.trim_start_matches("--capture-script=").into()),
            _ => return Err(format!("unknown option: {argument}")),
        }
    }
    if !model_path.is_file() {
        return Err(format!("Sponza FBX does not exist: {}", model_path.display()));
    }
    run(settings, SponzaDemo { model_path, debug_panels: None, free_camera: FreeCamera::default() })
}
