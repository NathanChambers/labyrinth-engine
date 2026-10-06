#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TimeScaleId(usize);

#[derive(Debug)]
pub struct Time {
    delta_seconds: f32,
    elapsed_seconds: f32,
    scales: Vec<TimeScale>,
}

#[derive(Debug)]
struct TimeScale {
    factor: f32,
    elapsed_seconds: f32,
}

impl Time {
    pub fn new() -> Self {
        Self { delta_seconds: 0.0, elapsed_seconds: 0.0, scales: Vec::new() }
    }

    pub fn delta_seconds(&self) -> f32 {
        self.delta_seconds
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed_seconds
    }

    pub fn create_scale(&mut self, factor: f32) -> TimeScaleId {
        let id = TimeScaleId(self.scales.len());
        self.scales.push(TimeScale { factor: factor.max(0.0), elapsed_seconds: 0.0 });
        id
    }

    pub fn scale_factor(&self, scale: TimeScaleId) -> Option<f32> {
        self.scales.get(scale.0).map(|scale| scale.factor)
    }

    pub fn set_scale_factor(&mut self, scale: TimeScaleId, factor: f32) -> bool {
        let Some(scale) = self.scales.get_mut(scale.0) else {
            return false;
        };
        scale.factor = factor.max(0.0);
        true
    }

    pub fn delta_for(&self, scale: TimeScaleId) -> Option<f32> {
        self.scales.get(scale.0).map(|scale| self.delta_seconds * scale.factor)
    }

    pub fn elapsed_for(&self, scale: TimeScaleId) -> Option<f32> {
        self.scales.get(scale.0).map(|scale| scale.elapsed_seconds)
    }

    pub(crate) fn advance(&mut self, delta_seconds: f32) {
        self.delta_seconds = delta_seconds.max(0.0);
        self.elapsed_seconds += self.delta_seconds;
        for scale in &mut self.scales {
            scale.elapsed_seconds += self.delta_seconds * scale.factor;
        }
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Time;

    #[test]
    fn scales_have_independent_elapsed_time() {
        let mut time = Time::new();
        let full_speed = time.create_scale(1.0);
        let half_speed = time.create_scale(0.5);
        time.advance(2.0);
        assert_eq!(time.delta_for(full_speed), Some(2.0));
        assert_eq!(time.delta_for(half_speed), Some(1.0));
        assert_eq!(time.elapsed_for(full_speed), Some(2.0));
        assert_eq!(time.elapsed_for(half_speed), Some(1.0));
    }
}
