use std::path::Path;

use labyrinth::{Application, Camera, Color, DirectionalLight, FontAsset, GraphicsApi, IrradianceMode, Material, MeshId, PointLight, RuntimeContext, Transform, UiAnchor, Vec2, Vec3, WindowMode, WindowSettings, primitives, run};
use labyrinth_debug_ui::{DebugPanels, DebugPanelsConfig};

struct CornellBoxDemo {
    debug_panels: Option<DebugPanels>,
}

impl Application for CornellBoxDemo {
    fn setup(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        context.irradiance_mode = IrradianceMode::AabbSdf;
        context.render_settings.irradiance_enabled = false;
        context.render_settings.triangle_probe_occlusion_enabled = false;
        context.render_settings.shadows_enabled = false;
        context.render_settings.ambient_intensity = 0.0;

        let white = Color::rgb(0.72, 0.72, 0.72);
        let red = Color::rgb(0.72, 0.08, 0.06);
        let green = Color::rgb(0.08, 0.58, 0.12);
        spawn_box(context, Vec3::new(0.0, -0.1, 0.0), Vec3::new(5.0, 0.2, 5.0), white);
        spawn_box(context, Vec3::new(0.0, 2.5, -2.5), Vec3::new(5.0, 5.0, 0.2), white);
        spawn_box(context, Vec3::new(-2.5, 2.5, 0.0), Vec3::new(0.2, 5.0, 5.0), red);
        spawn_box(context, Vec3::new(2.5, 2.5, 0.0), Vec3::new(0.2, 5.0, 5.0), green);
        spawn_box(context, Vec3::new(0.0, 5.1, 0.0), Vec3::new(5.0, 0.2, 5.0), white);

        let short_box = context
            .scene
            .spawn_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, white), Transform::from_position(Vec3::new(-0.75, 0.7, -0.55)).with_scale(Vec3::new(1.1, 1.4, 1.1)));
        if let Some(instance) = context.scene.get_mesh_mut(short_box) {
            instance.material = Material { roughness: 0.82, ..Material::default() };
        }
        let tall_box = context
            .scene
            .spawn_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, white), Transform::from_position(Vec3::new(0.75, 1.05, -1.15)).with_scale(Vec3::new(1.0, 2.1, 1.0)));
        if let Some(instance) = context.scene.get_mesh_mut(tall_box) {
            instance.material = Material { roughness: 0.82, ..Material::default() };
        }

        let mirror_sphere = context
            .scene
            .spawn_mesh(primitives::sphere([0.0, 0.0, 0.0], 0.62, Color::rgb(0.88, 0.9, 0.94), 40, 28), Transform::from_position(Vec3::new(-0.95, 0.62, 0.85)));
        set_material(context, mirror_sphere, Material { metallic: 0.92, roughness: 0.08, ..Material::default() });
        let diffuse_sphere = context
            .scene
            .spawn_mesh(primitives::sphere([0.0, 0.0, 0.0], 0.62, Color::rgb(0.78, 0.78, 0.78), 40, 28), Transform::from_position(Vec3::new(0.95, 0.62, 0.85)));
        set_material(context, diffuse_sphere, Material { roughness: 0.72, ..Material::default() });

        let light_color = Color::rgb(1.0, 0.92, 0.78);
        let light = context.scene.spawn_mesh(
            primitives::cube([0.0, 0.0, 0.0], 1.0, light_color),
            Transform::from_position(Vec3::new(0.0, 4.94, 0.0)).with_scale(Vec3::new(1.15, 0.025, 0.9)),
        );
        set_material(context, light, Material { base_color: light_color, emission: light_color, emission_strength: 0.5, ..Material::default() });

        let camera = context.scene.spawn_camera(Camera { clear_color: Color::rgb(0.025, 0.03, 0.04), ..Camera::default() });
        if let Some(camera) = context.scene.get_camera_mut(camera) {
            camera.transform.position = Vec3::new(0.0, 2.55, 8.7);
            camera.transform.rotation = labyrinth::Quat::from_euler(labyrinth::EulerRot::YXZ, 0.0, -0.08, 0.0);
        }
        context.active_camera = Some(camera);
        let overhead = context.scene.spawn_point_light(PointLight { position: Vec3::new(0.0, 4.65, 0.0), color: light_color, intensity: 1.8, range: 8.0 });
        let fill = context.scene.spawn_light(DirectionalLight { direction: Vec3::new(-0.45, 0.85, 0.35).normalize(), color: Color::WHITE, intensity: 0.08 });
        context.active_lights = vec![overhead, fill];
        context.sky_lighting = labyrinth::RendererSkyLighting { color: Color::rgb(0.28, 0.3, 0.34), intensity: 0.03 };
        let font_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/default.ttf");
        context.ui.set_font(FontAsset::from_file(font_path)?);
        self.debug_panels = Some(DebugPanels::new(
            context,
            DebugPanelsConfig::new("Render Debug")
                .with_position(UiAnchor::TOP_RIGHT, Vec2::new(-24.0, 24.0))
                .with_performance_offset(Vec2::new(-24.0, 500.0)),
        ));

        Ok(())
    }

    fn update(&mut self, context: &mut RuntimeContext) -> Result<(), String> {
        self.debug_panels.as_mut().expect("shared debug panels are initialized").update(context);

        Ok(())
    }
}

fn spawn_box(context: &mut RuntimeContext, position: Vec3, scale: Vec3, color: Color) -> MeshId {
    context.scene.spawn_mesh(primitives::cube([0.0, 0.0, 0.0], 1.0, color), Transform::from_position(position).with_scale(scale))
}

fn set_material(context: &mut RuntimeContext, mesh: MeshId, material: Material) {
    if let Some(instance) = context.scene.get_mesh_mut(mesh) {
        instance.material = material;
    }
}

fn main() -> Result<(), String> {
    let mut settings = WindowSettings { title: "Labyrinth Engine — Cornell Box".into(), ..WindowSettings::default() };

    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--windowed" => settings.mode = WindowMode::Windowed,
            "--borderless" => settings.mode = WindowMode::Borderless,
            "--fullscreen" => settings.mode = WindowMode::Exclusive,
            "--vulkan" => settings.graphics_api = GraphicsApi::Vulkan,
            "--dx12" => settings.graphics_api = GraphicsApi::DirectX12,
            "--opengl" => settings.graphics_api = GraphicsApi::OpenGl,
            "--metal" => settings.graphics_api = GraphicsApi::Metal,
            _ => return Err(format!("unknown option: {argument}")),
        }
    }

    run(settings, CornellBoxDemo { debug_panels: None })
}
