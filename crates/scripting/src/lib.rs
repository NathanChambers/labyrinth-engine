use rhai::{Engine, Scope};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraPose {
    pub position: [f32; 3],
    pub target: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRenderMode {
    Lit,
    Unlit,
    Wireframe,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureSettings {
    pub width: u32,
    pub height: u32,
    pub output_directory: String,
    pub include_ui: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self { width: 1280, height: 720, output_directory: "captures".into(), include_ui: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CaptureAction {
    SetRenderEffects { shadows: bool, irradiance: bool, gizmos: bool },
    SetRenderMode(CaptureRenderMode),
    SetUiEnabled(bool),
    SetCamera(CameraPose),
    MoveCamera { pose: CameraPose, duration: f32 },
    Wait(f32),
    Screenshot(String),
    PerformanceReport(String),
    Exit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CapturePlan {
    pub settings: CaptureSettings,
    pub actions: Vec<CaptureAction>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptCapabilities {
    pub camera_control: bool,
    pub capture: bool,
    pub exit: bool,
}

impl ScriptCapabilities {
    pub const fn all() -> Self {
        Self { camera_control: true, capture: true, exit: true }
    }
}

#[derive(Clone, Debug, Default)]
struct ScriptBuilder {
    plan: CapturePlan,
    capabilities: ScriptCapabilities,
}

impl ScriptBuilder {
    fn configure(&mut self, width: i64, height: i64, output_directory: String, include_ui: bool) {
        if !self.capabilities.capture {
            return;
        }
        self.plan.settings.width = width.max(1) as u32;
        self.plan.settings.height = height.max(1) as u32;
        self.plan.settings.output_directory = output_directory;
        self.plan.settings.include_ui = include_ui;
    }

    fn set_camera(&mut self, position: rhai::Array, target: rhai::Array) -> Result<(), Box<rhai::EvalAltResult>> {
        if !self.capabilities.camera_control {
            return Err("camera control capability is not granted".into());
        }
        self.plan.actions.push(CaptureAction::SetCamera(camera_pose(position, target)?));
        Ok(())
    }

    fn set_render_effects(&mut self, shadows: bool, irradiance: bool, gizmos: bool) {
        if !self.capabilities.capture {
            return;
        }
        self.plan.actions.push(CaptureAction::SetRenderEffects { shadows, irradiance, gizmos });
    }

    fn set_render_mode(&mut self, mode: String) -> Result<(), Box<rhai::EvalAltResult>> {
        if !self.capabilities.capture {
            return Ok(());
        }
        let mode = match mode.as_str() {
            "lit" => CaptureRenderMode::Lit,
            "unlit" => CaptureRenderMode::Unlit,
            "wireframe" => CaptureRenderMode::Wireframe,
            _ => return Err("render mode must be lit, unlit, or wireframe".into()),
        };
        self.plan.actions.push(CaptureAction::SetRenderMode(mode));
        Ok(())
    }

    fn set_ui_enabled(&mut self, enabled: bool) {
        if self.capabilities.capture {
            self.plan.actions.push(CaptureAction::SetUiEnabled(enabled));
        }
    }

    fn move_camera(&mut self, position: rhai::Array, target: rhai::Array, duration: f64) -> Result<(), Box<rhai::EvalAltResult>> {
        if !self.capabilities.camera_control {
            return Err("camera control capability is not granted".into());
        }
        if duration < 0.0 {
            return Err("camera movement duration cannot be negative".into());
        }
        self.plan.actions.push(CaptureAction::MoveCamera { pose: camera_pose(position, target)?, duration: duration as f32 });
        Ok(())
    }

    fn wait(&mut self, seconds: f64) -> Result<(), Box<rhai::EvalAltResult>> {
        if seconds < 0.0 {
            return Err("wait duration cannot be negative".into());
        }
        self.plan.actions.push(CaptureAction::Wait(seconds as f32));
        Ok(())
    }

    fn screenshot(&mut self, name: String) {
        if !self.capabilities.capture {
            return;
        }
        self.plan.actions.push(CaptureAction::Screenshot(name));
    }

    fn report(&mut self, name: String) {
        if !self.capabilities.capture {
            return;
        }
        self.plan.actions.push(CaptureAction::PerformanceReport(name));
    }

    fn sequence(&mut self, prefix: String, count: i64, interval: f64) -> Result<(), Box<rhai::EvalAltResult>> {
        if !self.capabilities.capture {
            return Ok(());
        }
        if count < 1 || interval < 0.0 || (count > 1 && interval == 0.0) {
            return Err("capture sequence requires a positive count and a positive interval between frames".into());
        }
        for index in 0..count {
            self.plan.actions.push(CaptureAction::Screenshot(format!("{prefix}_{index:04}")));
            if index + 1 < count {
                self.plan.actions.push(CaptureAction::Wait(interval as f32));
            }
        }
        Ok(())
    }

    fn exit(&mut self) {
        if !self.capabilities.exit {
            return;
        }
        self.plan.actions.push(CaptureAction::Exit);
    }
}

pub fn compile_capture_script(source: &str) -> Result<CapturePlan, String> {
    compile_capture_script_with_capabilities(source, ScriptCapabilities::all())
}

pub fn compile_capture_script_with_capabilities(source: &str, capabilities: ScriptCapabilities) -> Result<CapturePlan, String> {
    let mut engine = Engine::new();
    engine.set_max_operations(100_000);
    let mut scope = Scope::new();
    engine.register_type_with_name::<ScriptBuilder>("Capture");
    scope.push("capture", ScriptBuilder { plan: CapturePlan::default(), capabilities });
    engine.register_fn("configure", ScriptBuilder::configure);
    engine.register_fn("set_camera", ScriptBuilder::set_camera);
    engine.register_fn("set_render_effects", ScriptBuilder::set_render_effects);
    engine.register_fn("set_render_mode", ScriptBuilder::set_render_mode);
    engine.register_fn("set_ui_enabled", ScriptBuilder::set_ui_enabled);
    engine.register_fn("move_camera", ScriptBuilder::move_camera);
    engine.register_fn("wait", ScriptBuilder::wait);
    engine.register_fn("screenshot", ScriptBuilder::screenshot);
    engine.register_fn("report", ScriptBuilder::report);
    engine.register_fn("sequence", ScriptBuilder::sequence);
    engine.register_fn("exit", ScriptBuilder::exit);
    engine.eval_with_scope::<()>(&mut scope, source).map_err(|error| error.to_string())?;
    Ok(scope.get_value::<ScriptBuilder>("capture").ok_or("capture script did not initialize its plan")?.plan)
}

fn camera_pose(position: rhai::Array, target: rhai::Array) -> Result<CameraPose, Box<rhai::EvalAltResult>> {
    if position.len() != 3 || target.len() != 3 {
        return Err("camera positions and targets require three values".into());
    }
    let position = [number(&position[0])?, number(&position[1])?, number(&position[2])?];
    let target = [number(&target[0])?, number(&target[1])?, number(&target[2])?];
    Ok(CameraPose { position, target })
}

fn number(value: &rhai::Dynamic) -> Result<f32, Box<rhai::EvalAltResult>> {
    value.clone().try_cast::<f64>().map(|number| number as f32).or_else(|| value.clone().try_cast::<i64>().map(|number| number as f32)).ok_or_else(|| "camera coordinates must be numeric".into())
}

#[cfg(test)]
mod tests {
    use super::{CaptureAction, compile_capture_script};

    #[test]
    fn compiles_capture_plan_with_exit() {
        let plan = compile_capture_script(
            r#"
                capture.configure(640, 360, "captures/test", false);
                capture.set_camera([3.0, 2.0, 4.0], [0.0, 0.5, 0.0]);
                capture.screenshot("start");
                capture.wait(0.25);
                capture.move_camera([5.0, 3.0, 2.0], [0.0, 0.5, 0.0], 1.0);
                capture.screenshot("end");
                capture.sequence("orbit", 3, 0.25);
                capture.exit();
            "#,
        )
        .expect("script should compile");
        assert_eq!(plan.settings.width, 640);
        assert_eq!(plan.actions.len(), 11);
        assert!(matches!(plan.actions.last(), Some(CaptureAction::Exit)));
    }
}
