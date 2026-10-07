pub use glam::{EulerRot, Mat2, Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub transform: Transform,
    pub clear_color: Color,
    pub field_of_view_y: f32,
    pub near_clip: f32,
    pub far_clip: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalLight {
    pub direction: Vec3,
    pub color: Color,
    pub intensity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Color,
    pub intensity: f32,
    pub range: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotLight {
    pub position: Vec3,
    pub direction: Vec3,
    pub color: Color,
    pub intensity: f32,
    pub range: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Light {
    Directional(DirectionalLight),
    Point(PointLight),
    Spot(SpotLight),
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self { direction: Vec3::new(0.45, 1.0, 0.35), color: Color::WHITE, intensity: 1.0 }
    }
}

impl Camera {
    pub fn look_at(&mut self, target: Vec3) {
        let direction = (target - self.transform.position).normalize_or_zero();
        if direction.length_squared() > 0.0 {
            let yaw = (-direction.x).atan2(-direction.z);
            let pitch = direction.y.asin();
            self.transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        }
    }

    pub fn view_projection(&self, aspect_ratio: f32) -> Mat4 {
        let projection = Mat4::perspective_rh(self.field_of_view_y.to_radians(), aspect_ratio, self.near_clip, self.far_clip);
        projection * self.transform.matrix().inverse()
    }
}

impl Default for Camera {
    fn default() -> Self {
        let mut camera = Self { transform: Transform::from_position(Vec3::new(3.4, 2.5, 5.2)), clear_color: Color::rgb(0.08, 0.11, 0.16), field_of_view_y: 45.0, near_clip: 0.1, far_clip: 100.0 };
        camera.look_at(Vec3::new(0.0, 0.4, 0.0));
        camera
    }
}

impl Color {
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const RED: Self = Self::rgb(1.0, 0.0, 0.0);
    pub const GREEN: Self = Self::rgb(0.0, 1.0, 0.0);
    pub const BLUE: Self = Self::rgb(0.0, 0.0, 1.0);

    pub const fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue, alpha: 1.0 }
    }

    pub const fn rgba(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self { red, green, blue, alpha }
    }

    pub const fn to_array(self) -> [f32; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::WHITE
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Easing {
    #[default]
    Linear,
    SmoothStep,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutCubic,
}

pub fn ease(kind: Easing, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        Easing::Linear => t,
        Easing::SmoothStep => t * t * (3.0 - 2.0 * t),
        Easing::EaseInQuad => t * t,
        Easing::EaseOutQuad => 1.0 - (1.0 - t) * (1.0 - t),
        Easing::EaseInOutCubic => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) * 0.5
            }
        }
    }
}

pub fn lerp(start: f32, end: f32, amount: f32) -> f32 {
    start + (end - start) * amount.clamp(0.0, 1.0)
}

pub fn lerp_vec3(start: Vec3, end: Vec3, amount: f32) -> Vec3 {
    start.lerp(end, amount.clamp(0.0, 1.0))
}

pub fn slerp_quat(start: Quat, end: Quat, amount: f32) -> Quat {
    start.slerp(end, amount.clamp(0.0, 1.0))
}

pub fn ping_pong(time: f32, duration: f32) -> f32 {
    if duration <= 0.0 {
        return 0.0;
    }
    let cycle = (time / duration).rem_euclid(2.0);
    if cycle <= 1.0 { cycle } else { 2.0 - cycle }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self { position: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE };

    pub const fn new(position: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self { position, rotation, scale }
    }

    pub const fn from_position(position: Vec3) -> Self {
        Self { position, ..Self::IDENTITY }
    }

    pub const fn from_scale(scale: Vec3) -> Self {
        Self { scale, ..Self::IDENTITY }
    }

    pub fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }

    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.matrix().transform_point3(point)
    }

    pub fn transform_vector(self, vector: Vec3) -> Vec3 {
        self.matrix().transform_vector3(vector)
    }

    pub fn translate(&mut self, offset: Vec3) {
        self.position += offset;
    }

    pub fn rotate(&mut self, rotation: Quat) {
        self.rotation = (rotation * self.rotation).normalize();
    }

    pub fn scale_by(&mut self, factor: Vec3) {
        self.scale *= factor;
    }

    pub fn with_position(mut self, position: Vec3) -> Self {
        self.position = position;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::{Color, Easing, Quat, Transform, Vec3, ease, ping_pong};

    #[test]
    fn default_color_is_white() {
        assert_eq!(Color::default(), Color::WHITE);
        assert_eq!(Color::rgb(0.2, 0.4, 0.6).to_array(), [0.2, 0.4, 0.6, 1.0]);
    }

    #[test]
    fn matrix_applies_scale_rotation_and_position() {
        let transform = Transform::new(Vec3::new(3.0, 2.0, 1.0), Quat::IDENTITY, Vec3::splat(2.0));
        assert_eq!(transform.transform_point(Vec3::X), Vec3::new(5.0, 2.0, 1.0));
    }

    #[test]
    fn mutation_helpers_preserve_value_type() {
        let mut transform = Transform::default();
        transform.translate(Vec3::X);
        transform.rotate(Quat::from_rotation_y(0.5));
        transform.scale_by(Vec3::splat(2.0));
        assert_eq!(transform.position, Vec3::X);
        assert_eq!(transform.scale, Vec3::splat(2.0));
        assert!((transform.rotation.length() - 1.0).abs() < 0.00001);
    }

    #[test]
    fn easing_and_ping_pong_are_bounded() {
        assert_eq!(ease(Easing::EaseInOutCubic, -1.0), 0.0);
        assert_eq!(ease(Easing::EaseInOutCubic, 2.0), 1.0);
        assert_eq!(ping_pong(0.0, 2.0), 0.0);
        assert_eq!(ping_pong(2.0, 2.0), 1.0);
        assert_eq!(ping_pong(4.0, 2.0), 0.0);
    }
}
