use ab_glyph::{Font, PxScale, ScaleFont, point};
use math::{Color, Vec2};
use mesh::UiVertex;

use crate::{FontAsset, InputState};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UiSliderId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UiButtonId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UiLabelId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UiContainerId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UiPanelId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct UiGraphId(usize);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl UiRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    fn contains(self, point: Vec2) -> bool {
        point.x >= self.x && point.x <= self.x + self.width && point.y >= self.y && point.y <= self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UiAnchor {
    pub x: f32,
    pub y: f32,
}

impl UiAnchor {
    pub const CENTER: Self = Self { x: 0.5, y: 0.5 };
    pub const TOP_LEFT: Self = Self { x: 0.0, y: 0.0 };
    pub const TOP_RIGHT: Self = Self { x: 1.0, y: 0.0 };
    pub const BOTTOM_LEFT: Self = Self { x: 0.0, y: 1.0 };
    pub const BOTTOM_RIGHT: Self = Self { x: 1.0, y: 1.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

pub type UiPivot = UiAnchor;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiLayout {
    pub size: Vec2,
    pub anchor: UiAnchor,
    pub pivot: UiPivot,
    pub offset: Vec2,
}

impl UiLayout {
    pub const fn new(size: Vec2) -> Self {
        Self { size, anchor: UiAnchor::CENTER, pivot: UiPivot::CENTER, offset: Vec2::ZERO }
    }

    pub const fn anchored(size: Vec2, anchor: UiAnchor, pivot: UiPivot, offset: Vec2) -> Self {
        Self { size, anchor, pivot, offset }
    }

    pub const fn absolute(rect: UiRect) -> Self {
        Self::anchored(Vec2::new(rect.width, rect.height), UiAnchor::TOP_LEFT, UiPivot::TOP_LEFT, Vec2::new(rect.x, rect.y))
    }

    fn resolve(self, parent: UiRect) -> UiRect {
        let anchor_position = Vec2::new(parent.x + parent.width * self.anchor.x, parent.y + parent.height * self.anchor.y) + self.offset;
        let position = anchor_position - Vec2::new(self.size.x * self.pivot.x, self.size.y * self.pivot.y);
        UiRect::new(position.x, position.y, self.size.x, self.size.y)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiFlexDirection {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug)]
pub struct UiAutoLayout {
    pub direction: UiFlexDirection,
    pub padding: f32,
    pub spacing: f32,
}

impl UiAutoLayout {
    pub const fn column(padding: f32, spacing: f32) -> Self {
        Self { direction: UiFlexDirection::Column, padding, spacing }
    }

    pub const fn row(padding: f32, spacing: f32) -> Self {
        Self { direction: UiFlexDirection::Row, padding, spacing }
    }
}

#[derive(Clone, Copy, Debug)]
struct UiSlider {
    layout: UiLayout,
    parent: Option<UiContainerId>,
    child_index: usize,
    min: f32,
    max: f32,
    value: f32,
    track_color: Color,
    fill_color: Color,
    knob_color: Color,
}

#[derive(Clone, Debug)]
struct UiButton {
    layout: UiLayout,
    parent: Option<UiContainerId>,
    child_index: usize,
    toggled: bool,
    text: String,
    text_size: f32,
    text_color: Color,
    text_centered: bool,
    toggled_color: Color,
    untoggled_color: Color,
}

#[derive(Clone, Debug)]
struct UiLabel {
    layout: UiLayout,
    parent: Option<UiContainerId>,
    child_index: usize,
    text: String,
    size: f32,
    color: Color,
    visible: bool,
}

#[derive(Clone, Copy, Debug)]
enum UiChild {
    Slider(UiSliderId),
    Button(UiButtonId),
    Label(UiLabelId),
    Container(UiContainerId),
}

#[derive(Clone, Debug)]
struct UiContainer {
    layout: UiLayout,
    parent: Option<UiContainerId>,
    child_index: usize,
    auto_layout: Option<UiAutoLayout>,
    children: Vec<UiChild>,
    visible: bool,
}

#[derive(Clone, Debug)]
struct UiPanel {
    layout: UiLayout,
    color: Color,
}

#[derive(Clone, Debug)]
struct UiGraph {
    layout: UiLayout,
    values: Vec<f32>,
    color: Color,
    background: Color,
    visible: bool,
}

#[derive(Debug)]
pub struct UiCanvas {
    window: UiLayout,
    sliders: Vec<UiSlider>,
    buttons: Vec<UiButton>,
    labels: Vec<UiLabel>,
    containers: Vec<UiContainer>,
    panels: Vec<UiPanel>,
    graphs: Vec<UiGraph>,
    font: Option<FontAsset>,
}

impl UiCanvas {
    pub fn new(window: UiRect) -> Self {
        Self { window: UiLayout::absolute(window), sliders: Vec::new(), buttons: Vec::new(), labels: Vec::new(), containers: Vec::new(), panels: Vec::new(), graphs: Vec::new(), font: None }
    }

    pub fn set_font(&mut self, font: FontAsset) {
        self.font = Some(font);
    }

    pub fn clear_font(&mut self) {
        self.font = None;
    }

    pub fn set_window_layout(&mut self, layout: UiLayout) {
        self.window = layout;
    }

    pub fn add_container(&mut self, parent: Option<UiContainerId>, layout: UiLayout, auto_layout: UiAutoLayout) -> UiContainerId {
        let id = UiContainerId(self.containers.len());
        let child_index = parent.map(|parent| self.add_child(parent, UiChild::Container(id))).unwrap_or(0);
        self.containers.push(UiContainer { layout, parent, child_index, auto_layout: Some(auto_layout), children: Vec::new(), visible: true });
        id
    }

    pub fn set_container_visible(&mut self, container: UiContainerId, visible: bool) -> bool {
        let Some(container) = self.containers.get_mut(container.0) else { return false };
        container.visible = visible;
        true
    }

    pub fn set_label_visible(&mut self, label: UiLabelId, visible: bool) -> bool {
        let Some(label) = self.labels.get_mut(label.0) else { return false };
        label.visible = visible;
        true
    }

    pub fn add_panel(&mut self, layout: UiLayout, color: Color) -> UiPanelId {
        let id = UiPanelId(self.panels.len());
        self.panels.push(UiPanel { layout, color });
        id
    }

    pub fn set_panel_layout(&mut self, panel: UiPanelId, layout: UiLayout) -> bool {
        let Some(panel) = self.panels.get_mut(panel.0) else { return false };
        panel.layout = layout;
        true
    }

    pub fn add_graph(&mut self, layout: UiLayout, color: Color, background: Color) -> UiGraphId {
        let id = UiGraphId(self.graphs.len());
        self.graphs.push(UiGraph { layout, values: Vec::new(), color, background, visible: true });
        id
    }

    pub fn set_graph_visible(&mut self, graph: UiGraphId, visible: bool) -> bool {
        let Some(graph) = self.graphs.get_mut(graph.0) else { return false };
        graph.visible = visible;
        true
    }

    pub fn set_graph_values(&mut self, graph: UiGraphId, values: &[f32]) -> bool {
        let Some(graph) = self.graphs.get_mut(graph.0) else { return false };
        graph.values.clear();
        graph.values.extend_from_slice(values);
        true
    }

    pub fn add_slider(&mut self, rect: UiRect, min: f32, max: f32, value: f32) -> UiSliderId {
        self.add_slider_layout(None, UiLayout::absolute(rect), min, max, value)
    }

    pub fn add_slider_layout(&mut self, parent: Option<UiContainerId>, layout: UiLayout, min: f32, max: f32, value: f32) -> UiSliderId {
        assert!(max > min, "UI slider maximum must be greater than minimum");
        let id = UiSliderId(self.sliders.len());
        let child_index = parent.map(|parent| self.add_child(parent, UiChild::Slider(id))).unwrap_or(0);
        self.sliders.push(UiSlider {
            layout,
            parent,
            child_index,
            min,
            max,
            value: value.clamp(min, max),
            track_color: Color::rgb(0.18, 0.2, 0.24),
            fill_color: Color::rgb(0.2, 0.55, 0.95),
            knob_color: Color::WHITE,
        });
        id
    }

    pub fn slider_value(&self, slider: UiSliderId) -> Option<f32> {
        self.sliders.get(slider.0).map(|slider| slider.value)
    }

    pub fn set_slider_value(&mut self, slider: UiSliderId, value: f32) -> bool {
        let Some(slider) = self.sliders.get_mut(slider.0) else { return false };
        slider.value = value.clamp(slider.min, slider.max);
        true
    }

    pub fn add_toggle_button(&mut self, rect: UiRect, toggled: bool) -> UiButtonId {
        self.add_toggle_button_layout(None, UiLayout::absolute(rect), toggled)
    }

    pub fn add_toggle_button_layout(&mut self, parent: Option<UiContainerId>, layout: UiLayout, toggled: bool) -> UiButtonId {
        let id = UiButtonId(self.buttons.len());
        let child_index = parent.map(|parent| self.add_child(parent, UiChild::Button(id))).unwrap_or(0);
        self.buttons.push(UiButton {
            layout,
            parent,
            child_index,
            toggled,
            text: String::new(),
            text_size: 11.0,
            text_color: Color::WHITE,
            text_centered: true,
            toggled_color: Color::rgb(0.2, 0.75, 0.35),
            untoggled_color: Color::rgb(0.35, 0.38, 0.44),
        });
        id
    }

    pub fn set_button_toggled_colors(&mut self, button: UiButtonId, toggled_color: Color, untoggled_color: Color) -> bool {
        let Some(button) = self.buttons.get_mut(button.0) else { return false };
        button.toggled_color = toggled_color;
        button.untoggled_color = untoggled_color;
        true
    }

    pub fn set_button_text(&mut self, button: UiButtonId, text: impl Into<String>, size: f32, color: Color) -> bool {
        let Some(button) = self.buttons.get_mut(button.0) else { return false };
        button.text = text.into();
        button.text_size = size.max(1.0);
        button.text_color = color;
        true
    }

    pub fn set_button_text_centered(&mut self, button: UiButtonId, centered: bool) -> bool {
        let Some(button) = self.buttons.get_mut(button.0) else { return false };
        button.text_centered = centered;
        true
    }

    pub fn button_toggled(&self, button: UiButtonId) -> Option<bool> {
        self.buttons.get(button.0).map(|button| button.toggled)
    }

    pub fn set_button_toggled(&mut self, button: UiButtonId, toggled: bool) -> bool {
        let Some(button) = self.buttons.get_mut(button.0) else { return false };
        button.toggled = toggled;
        true
    }

    pub fn add_label(&mut self, position: Vec2, text: impl Into<String>, size: f32, color: Color) -> UiLabelId {
        self.add_label_layout(None, UiLayout::absolute(UiRect::new(position.x, position.y, 0.0, size)), text, size, color)
    }

    pub fn add_label_layout(&mut self, parent: Option<UiContainerId>, layout: UiLayout, text: impl Into<String>, size: f32, color: Color) -> UiLabelId {
        let id = UiLabelId(self.labels.len());
        let child_index = parent.map(|parent| self.add_child(parent, UiChild::Label(id))).unwrap_or(0);
        self.labels.push(UiLabel { layout, parent, child_index, text: text.into(), size: size.max(1.0), color, visible: true });
        id
    }

    pub fn set_label_text(&mut self, label: UiLabelId, text: impl Into<String>) -> bool {
        let Some(label) = self.labels.get_mut(label.0) else { return false };
        label.text = text.into();
        true
    }

    pub fn update(&mut self, input: &InputState, width: u32, height: u32) {
        if input.cursor_captured() || (!input.left_mouse_down() && !input.left_mouse_pressed()) {
            return;
        }
        let root = UiRect::new(0.0, 0.0, width as f32, height as f32);
        let cursor = input.cursor_position();
        if input.left_mouse_pressed() {
            for index in 0..self.buttons.len() {
                let button = UiChild::Button(UiButtonId(index));
                if self.child_is_visible(button) && self.control_rect(button, root).contains(cursor) {
                    self.buttons[index].toggled = !self.buttons[index].toggled;
                }
            }
        }
        if !input.left_mouse_down() {
            return;
        }
        for index in 0..self.sliders.len() {
            let slider = UiChild::Slider(UiSliderId(index));
            if !self.child_is_visible(slider) {
                continue;
            }
            let rect = self.control_rect(slider, root);
            if rect.contains(cursor) {
                let amount = ((cursor.x - rect.x) / rect.width).clamp(0.0, 1.0);
                let slider = &mut self.sliders[index];
                slider.value = slider.min + (slider.max - slider.min) * amount;
            }
        }
    }

    pub fn vertices(&self, width: u32, height: u32) -> Vec<UiVertex> {
        let root = UiRect::new(0.0, 0.0, width as f32, height as f32);
        let window = self.window.resolve(root);
        let mut vertices = Vec::with_capacity(30);
        add_rect(&mut vertices, window, Color::rgba(0.03, 0.04, 0.06, 0.94), width, height);
        add_rect(&mut vertices, UiRect::new(window.x, window.y, window.width, 26.0), Color::rgba(0.1, 0.12, 0.17, 1.0), width, height);
        for panel in &self.panels {
            add_rect(&mut vertices, panel.layout.resolve(root), panel.color, width, height);
        }
        for graph in &self.graphs {
            if graph.visible {
                add_graph(&mut vertices, graph, graph.layout.resolve(root), width, height);
            }
        }
        for (index, slider) in self.sliders.iter().enumerate() {
            let child = UiChild::Slider(UiSliderId(index));
            if !self.child_is_visible(child) {
                continue;
            }
            let rect = self.control_rect(child, root);
            let track = UiRect::new(rect.x, rect.y + rect.height * 0.35, rect.width, rect.height * 0.3);
            add_rect(&mut vertices, track, slider.track_color, width, height);
            let amount = (slider.value - slider.min) / (slider.max - slider.min);
            add_rect(&mut vertices, UiRect::new(track.x, track.y, track.width * amount, track.height), slider.fill_color, width, height);
            let knob_x = rect.x + rect.width * amount - 5.0;
            add_rect(&mut vertices, UiRect::new(knob_x, rect.y, 10.0, rect.height), slider.knob_color, width, height);
        }
        for (index, button) in self.buttons.iter().enumerate() {
            let child = UiChild::Button(UiButtonId(index));
            if !self.child_is_visible(child) {
                continue;
            }
            let color = if button.toggled { button.toggled_color } else { button.untoggled_color };
            add_rect(&mut vertices, self.control_rect(child, root), color, width, height);
        }
        if let Some(font) = &self.font {
            for (index, button) in self.buttons.iter().enumerate() {
                let child = UiChild::Button(UiButtonId(index));
                if self.child_is_visible(child) && !button.text.is_empty() {
                    let mut rect = self.control_rect(UiChild::Button(UiButtonId(index)), root);
                    if !button.text_centered {
                        rect.x += 8.0;
                        rect.width = (rect.width - 16.0).max(0.0);
                    }
                    add_text(&mut vertices, font.font(), &button.text, button.text_size, button.text_color, rect, (width, height), button.text_centered);
                }
            }
            for (index, label) in self.labels.iter().enumerate() {
                let child = UiChild::Label(UiLabelId(index));
                if !self.child_is_visible(child) {
                    continue;
                }
                let rect = self.control_rect(child, root);
                add_label(&mut vertices, font.font(), label, rect, (width, height));
            }
        }
        vertices
    }

    fn add_child(&mut self, parent: UiContainerId, child: UiChild) -> usize {
        let container = self.containers.get_mut(parent.0).expect("UI parent container does not exist");
        let index = container.children.len();
        container.children.push(child);
        index
    }

    fn control_rect(&self, child: UiChild, root: UiRect) -> UiRect {
        match child {
            UiChild::Slider(id) => {
                let item = self.sliders[id.0];
                self.item_rect(item.parent, item.child_index, item.layout, root)
            }
            UiChild::Button(id) => {
                let item = &self.buttons[id.0];
                self.item_rect(item.parent, item.child_index, item.layout, root)
            }
            UiChild::Label(id) => {
                let item = &self.labels[id.0];
                self.item_rect(item.parent, item.child_index, item.layout, root)
            }
            UiChild::Container(id) => self.container_rect(id, root),
        }
    }

    fn child_is_visible(&self, child: UiChild) -> bool {
        let parent = match child {
            UiChild::Slider(id) => self.sliders[id.0].parent,
            UiChild::Button(id) => self.buttons[id.0].parent,
            UiChild::Label(id) => {
                let label = &self.labels[id.0];
                return label.visible && self.container_is_visible(label.parent);
            }
            UiChild::Container(id) => return self.container_is_visible(Some(id)),
        };
        self.container_is_visible(parent)
    }

    fn container_is_visible(&self, mut container_id: Option<UiContainerId>) -> bool {
        while let Some(id) = container_id {
            let container = &self.containers[id.0];
            if !container.visible {
                return false;
            }
            container_id = container.parent;
        }
        true
    }

    fn container_rect(&self, id: UiContainerId, root: UiRect) -> UiRect {
        let container = &self.containers[id.0];
        let parent = container.parent.map(|parent| self.container_rect(parent, root)).unwrap_or(root);
        let base = container.layout.resolve(parent);
        if let Some(parent_id) = container.parent { self.auto_child_rect(parent, parent_id, container.child_index, container.layout, base) } else { base }
    }

    fn item_rect(&self, parent_id: Option<UiContainerId>, child_index: usize, layout: UiLayout, root: UiRect) -> UiRect {
        let parent_rect = parent_id.map(|parent| self.container_rect(parent, root)).unwrap_or(root);
        let base = layout.resolve(parent_rect);
        parent_id.map(|parent| self.auto_child_rect(parent_rect, parent, child_index, layout, base)).unwrap_or(base)
    }

    fn auto_child_rect(&self, parent_rect: UiRect, parent_id: UiContainerId, child_index: usize, layout: UiLayout, fallback: UiRect) -> UiRect {
        let Some(auto) = self.containers[parent_id.0].auto_layout else { return fallback };
        let children = &self.containers[parent_id.0].children;
        let visible_children = children.iter().filter(|child| self.child_is_visible(**child)).count();
        let available = match auto.direction {
            UiFlexDirection::Column => parent_rect.height - auto.padding * 2.0 - auto.spacing * visible_children.saturating_sub(1) as f32,
            UiFlexDirection::Row => parent_rect.width - auto.padding * 2.0 - auto.spacing * visible_children.saturating_sub(1) as f32,
        };
        let fixed = children
            .iter()
            .filter(|child| self.child_is_visible(**child))
            .map(|child| self.child_layout(*child))
            .map(|child_layout| match auto.direction {
                UiFlexDirection::Column => child_layout.size.y,
                UiFlexDirection::Row => child_layout.size.x,
            })
            .filter(|size| *size > 0.0)
            .sum::<f32>();
        let fill_count = children
            .iter()
            .filter(|child| self.child_is_visible(**child))
            .map(|child| self.child_layout(*child))
            .filter(|child_layout| match auto.direction {
                UiFlexDirection::Column => child_layout.size.y <= 0.0,
                UiFlexDirection::Row => child_layout.size.x <= 0.0,
            })
            .count();
        let fill_size = if fill_count == 0 { 0.0 } else { ((available - fixed) / fill_count as f32).max(0.0) };
        let mut cursor = auto.padding;
        for child in self.containers[parent_id.0].children.iter().take(child_index).filter(|child| self.child_is_visible(**child)) {
            let prior = self.child_layout(*child);
            let prior_size = match auto.direction {
                UiFlexDirection::Column => prior.size.y,
                UiFlexDirection::Row => prior.size.x,
            };
            cursor += if prior_size <= 0.0 { fill_size } else { prior_size } + auto.spacing;
        }
        let main_size = match auto.direction {
            UiFlexDirection::Column => {
                if layout.size.y <= 0.0 {
                    fill_size
                } else {
                    layout.size.y
                }
            }
            UiFlexDirection::Row => {
                if layout.size.x <= 0.0 {
                    fill_size
                } else {
                    layout.size.x
                }
            }
        };
        match auto.direction {
            UiFlexDirection::Column => UiRect::new(
                parent_rect.x + auto.padding + layout.offset.x,
                parent_rect.y + cursor + layout.offset.y,
                if layout.size.x <= 0.0 { parent_rect.width - auto.padding * 2.0 } else { layout.size.x },
                main_size,
            ),
            UiFlexDirection::Row => UiRect::new(
                parent_rect.x + cursor + layout.offset.x,
                parent_rect.y + auto.padding + layout.offset.y,
                main_size,
                if layout.size.y <= 0.0 { parent_rect.height - auto.padding * 2.0 } else { layout.size.y },
            ),
        }
    }

    fn child_layout(&self, child: UiChild) -> UiLayout {
        match child {
            UiChild::Slider(id) => self.sliders[id.0].layout,
            UiChild::Button(id) => self.buttons[id.0].layout,
            UiChild::Label(id) => self.labels[id.0].layout,
            UiChild::Container(id) => self.containers[id.0].layout,
        }
    }
}

fn add_label(vertices: &mut Vec<UiVertex>, font: &ab_glyph::FontArc, label: &UiLabel, rect: UiRect, viewport: (u32, u32)) {
    add_text(vertices, font, &label.text, label.size, label.color, rect, viewport, false);
}

fn add_graph(vertices: &mut Vec<UiVertex>, graph: &UiGraph, rect: UiRect, width: u32, height: u32) {
    add_rect(vertices, rect, graph.background, width, height);
    if graph.values.len() < 2 {
        return;
    }
    let maximum = graph.values.iter().copied().fold(16.7, f32::max).max(0.001);
    let step = rect.width / (graph.values.len() - 1) as f32;
    for (index, values) in graph.values.windows(2).enumerate() {
        let start = Vec2::new(rect.x + index as f32 * step, rect.y + rect.height - (values[0] / maximum).clamp(0.0, 1.0) * rect.height);
        let end = Vec2::new(rect.x + (index + 1) as f32 * step, rect.y + rect.height - (values[1] / maximum).clamp(0.0, 1.0) * rect.height);
        add_line(vertices, start, end, graph.color, 2.0, width, height);
    }
}

fn add_line(vertices: &mut Vec<UiVertex>, start: Vec2, end: Vec2, color: Color, thickness: f32, width: u32, height: u32) {
    let delta = end - start;
    let length = delta.length().max(0.001);
    let normal = Vec2::new(-delta.y, delta.x) / length * (thickness * 0.5);
    let corners = [start + normal, start - normal, end - normal, end + normal];
    let to_ndc = |point: Vec2| [point.x / width as f32 * 2.0 - 1.0, 1.0 - point.y / height as f32 * 2.0];
    let color = color.to_array();
    let first = UiVertex { position: to_ndc(corners[0]), color };
    let second = UiVertex { position: to_ndc(corners[1]), color };
    let third = UiVertex { position: to_ndc(corners[2]), color };
    let fourth = UiVertex { position: to_ndc(corners[3]), color };
    vertices.extend_from_slice(&[first, second, third, first, third, fourth]);
}

fn add_text(vertices: &mut Vec<UiVertex>, font: &ab_glyph::FontArc, text: &str, size: f32, color: Color, rect: UiRect, viewport: (u32, u32), centered: bool) {
    let scaled = font.as_scaled(PxScale::from(size));
    let text_width = text.chars().map(|character| scaled.h_advance(scaled.glyph_id(character))).sum::<f32>();
    let mut cursor_x = if centered { rect.x + (rect.width - text_width).max(0.0) * 0.5 } else { rect.x };
    let baseline = rect.y + (rect.height - size).max(0.0) * 0.5 + scaled.ascent();
    for character in text.chars() {
        let mut glyph = scaled.scaled_glyph(character);
        glyph.position = point(cursor_x, baseline);
        cursor_x += scaled.h_advance(glyph.id);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let bounds = outline.px_bounds();
        outline.draw(|x, y, coverage| {
            if coverage <= 0.0 {
                return;
            }
            let color = Color::rgba(color.red, color.green, color.blue, color.alpha * coverage);
            add_rect(vertices, UiRect::new(bounds.min.x + x as f32, bounds.min.y + y as f32, 1.0, 1.0), color, viewport.0, viewport.1);
        });
    }
}

fn add_rect(vertices: &mut Vec<UiVertex>, rect: UiRect, color: Color, width: u32, height: u32) {
    let left = rect.x / width as f32 * 2.0 - 1.0;
    let right = (rect.x + rect.width) / width as f32 * 2.0 - 1.0;
    let top = 1.0 - rect.y / height as f32 * 2.0;
    let bottom = 1.0 - (rect.y + rect.height) / height as f32 * 2.0;
    let color = color.to_array();
    let first = UiVertex { position: [left, top], color };
    let second = UiVertex { position: [left, bottom], color };
    let third = UiVertex { position: [right, bottom], color };
    let fourth = UiVertex { position: [right, top], color };
    vertices.extend_from_slice(&[first, second, third, first, third, fourth]);
}

#[cfg(test)]
mod tests {
    use super::{UiAnchor, UiLayout, UiRect};
    use math::Vec2;

    #[test]
    fn default_layout_is_centered_in_parent() {
        let rect = UiLayout::new(Vec2::new(20.0, 10.0)).resolve(UiRect::new(0.0, 0.0, 100.0, 80.0));
        assert_eq!(rect, UiRect::new(40.0, 35.0, 20.0, 10.0));
    }

    #[test]
    fn anchored_layout_uses_parent_edges_and_offset() {
        let layout = UiLayout::anchored(Vec2::new(30.0, 12.0), UiAnchor::BOTTOM_RIGHT, UiAnchor::TOP_RIGHT, Vec2::new(-8.0, -6.0));
        let rect = layout.resolve(UiRect::new(10.0, 20.0, 100.0, 80.0));
        assert_eq!(rect, UiRect::new(72.0, 94.0, 30.0, 12.0));
    }
}
