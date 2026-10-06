use math::Vec2;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

#[derive(Debug, Default)]
pub struct InputState {
    keys: HashSet<KeyCode>,
    mouse_delta: Vec2,
    cursor_position: Vec2,
    left_mouse_down: bool,
    left_mouse_pressed: bool,
    cursor_captured: bool,
}

impl InputState {
    pub fn is_key_down(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }

    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    pub fn cursor_position(&self) -> Vec2 {
        self.cursor_position
    }

    pub fn left_mouse_down(&self) -> bool {
        self.left_mouse_down
    }

    pub fn left_mouse_pressed(&self) -> bool {
        self.left_mouse_pressed
    }

    pub fn cursor_captured(&self) -> bool {
        self.cursor_captured
    }

    pub fn alt_held(&self) -> bool {
        self.is_key_down(KeyCode::AltLeft) || self.is_key_down(KeyCode::AltRight)
    }

    pub fn set_cursor_captured(&mut self, captured: bool) {
        self.cursor_captured = captured;
    }

    pub(crate) fn set_key(&mut self, key: KeyCode, pressed: bool) {
        if pressed {
            self.keys.insert(key);
        } else {
            self.keys.remove(&key);
        }
    }

    pub(crate) fn add_mouse_delta(&mut self, delta: Vec2) {
        self.mouse_delta += delta;
    }

    pub(crate) fn set_cursor_position(&mut self, position: Vec2) {
        self.cursor_position = position;
    }

    pub(crate) fn set_left_mouse_down(&mut self, down: bool) {
        self.left_mouse_down = down;
        if down {
            self.left_mouse_pressed = true;
        }
    }

    pub(crate) fn clear_frame(&mut self) {
        self.mouse_delta = Vec2::ZERO;
        self.left_mouse_pressed = false;
    }

    pub(crate) fn clear_keys(&mut self) {
        self.keys.clear();
    }
}
