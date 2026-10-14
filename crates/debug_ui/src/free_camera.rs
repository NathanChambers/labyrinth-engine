use labyrinth::{CameraId, EulerRot, KeyCode, Quat, RuntimeContext, Vec3};

/// Shared mouse-look and flight controls for example and debug cameras.
#[derive(Clone, Copy, Debug, Default)]
pub struct FreeCamera {
    yaw: f32,
    pitch: f32,
}

impl FreeCamera {
    pub fn new(yaw: f32, pitch: f32) -> Self {
        Self { yaw, pitch: pitch.clamp(-1.5, 1.5) }
    }

    pub fn from_rotation(rotation: Quat) -> Self {
        let forward = rotation * Vec3::NEG_Z;

        Self::new((-forward.x).atan2(-forward.z), forward.y.asin())
    }

    pub fn rotation(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    /// Applies cursor-captured mouse look and WASD/Space/C flight controls.
    /// Shift increases movement speed. `movement_keys_active` disables keyboard
    /// movement while a debug UI inspector is using those keys.
    pub fn update(&mut self, context: &mut RuntimeContext, camera_id: CameraId, delta_seconds: f32, movement_keys_active: bool) -> Result<(), String> {
        if context.input.cursor_captured() {
            let mouse_delta = context.input.mouse_delta();
            self.yaw -= mouse_delta.x * 0.002;
            self.pitch = (self.pitch - mouse_delta.y * 0.002).clamp(-1.5, 1.5);
        }

        let rotation = self.rotation();
        let camera = context.scene.get_camera_mut(camera_id).ok_or("free camera target was removed")?;
        let mut movement = Vec3::ZERO;
        if !movement_keys_active {
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
        }

        let speed = if context.input.is_key_down(KeyCode::ShiftLeft) { 20.0 } else { 5.0 };
        camera.transform.rotation = rotation;
        camera.transform.translate(movement.normalize_or_zero() * speed * delta_seconds);

        Ok(())
    }
}
